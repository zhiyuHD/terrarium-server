use crate::{
    protocol::{valid_id, valid_version},
    store::{Store, document, save_document},
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, SecondsFormat};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
#[derive(Debug)]
pub struct Gate {
    pub controls: Value,
    pub notice: Option<Value>,
    pub rejection: Option<u16>,
}
pub fn iso(ms: i64) -> String {
    DateTime::from_timestamp_millis(ms)
        .unwrap()
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}
pub fn at(value: &Value) -> Option<i64> {
    value
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
}
pub fn meets_minimum(client: &Value, minimum: &Value) -> bool {
    if minimum.is_null() {
        return true;
    }
    if !valid_version(client) {
        return false;
    }
    let client = client.as_str().unwrap().split('+').next().unwrap();
    let main = client.split('-').next().unwrap();
    let min = minimum.as_str().unwrap_or("");
    for (a, b) in main.split('.').zip(min.split('.')) {
        let a = a.trim_start_matches('0');
        let b = b.trim_start_matches('0');
        match a.len().cmp(&b.len()).then(a.cmp(b)) {
            Ordering::Greater => return true,
            Ordering::Less => return false,
            _ => {}
        }
    }
    !client.contains('-')
}
pub(crate) fn stable_version(value: &Value) -> bool {
    value.is_null()
        || value.as_str().is_some_and(|v| {
            valid_version(value)
                && !v.contains(['-', '+'])
                && v.split('.').all(|n| n == "0" || !n.starts_with('0'))
        })
}
pub(crate) fn validate_document(name: &str, value: &Value) -> Result<()> {
    match name {
        "story.json" => crate::story::validate(value)?,
        "feedback.json" => crate::operations::validate_feedback(value)?,
        "controls.json" => {
            ensure!(
                value.is_object()
                    && value["httpPush"].is_object()
                    && value["httpPush"]["enabled"].is_boolean(),
                "Invalid HTTP Push controls"
            );
            if let Some(logo) = value.get("logo") {
                ensure!(
                    matches!(logo["version"].as_str(), Some("0.8.6" | "0.8.7")),
                    "Invalid logo version"
                );
                let schedule = &logo["scheduledChange"];
                ensure!(
                    schedule.is_null()
                        || (matches!(schedule["version"].as_str(), Some("0.8.6" | "0.8.7"))
                            && at(&schedule["at"]).is_some()),
                    "Invalid logo schedule"
                );
            }
            if let Some(forum) = value.get("forum") {
                ensure!(forum["enabled"].is_boolean(), "Invalid forum control");
                let scheduled = &forum["scheduledChange"];
                ensure!(
                    scheduled.is_null()
                        || (scheduled["enabled"].is_boolean() && at(&scheduled["at"]).is_some()),
                    "Invalid forum schedule"
                );
            }
            let scheduled = &value["httpPush"]["scheduledChange"];
            ensure!(
                scheduled.is_null()
                    || (scheduled.is_object()
                        && scheduled["enabled"].is_boolean()
                        && at(&scheduled["at"]).is_some()),
                "Invalid HTTP Push schedule"
            );
            if let Some(floor) = value.get("minimumClientVersion") {
                ensure!(
                    floor.is_object() && floor.get("version").is_some_and(stable_version),
                    "Invalid minimum client version"
                );
                let scheduled = &floor["scheduledChange"];
                ensure!(
                    scheduled.is_null()
                        || (scheduled.is_object()
                            && scheduled.get("version").is_some_and(stable_version)
                            && at(&scheduled["at"]).is_some()),
                    "Invalid minimum version schedule"
                );
            }
        }
        "announcements.json" => {
            let next = value["nextId"]
                .as_u64()
                .filter(|n| *n > 0 && *n <= 9_007_199_254_740_991)
                .context("Invalid announcement cursor")?;
            let announcements = value["announcements"]
                .as_array()
                .context("Invalid announcements")?;
            ensure!(announcements.len() <= 2000, "Invalid announcement count");
            let mut previous = 0;
            for notice in announcements {
                let id = notice["id"]
                    .as_u64()
                    .filter(|n| *n > previous && *n < next)
                    .context("Invalid announcement ID")?;
                previous = id;
                ensure!(
                    notice["title"]
                        .as_str()
                        .is_some_and(|s| s.encode_utf16().count() <= 160)
                        && notice["text"].as_str().is_some_and(
                            |s| !s.trim().is_empty() && s.encode_utf16().count() <= 2000
                        ),
                    "Invalid announcement text"
                );
                ensure!(
                    at(&notice["createdAt"])
                        .zip(at(&notice["expiresAt"]))
                        .is_some_and(|(created, expires)| expires > created),
                    "Invalid announcement dates"
                );
                ensure!(
                    notice["targets"] == "all"
                        || notice["targets"].as_array().is_some_and(|a| !a.is_empty()
                            && a.len() <= 100
                            && a.iter().all(|v| v.as_str().is_some_and(valid_id))),
                    "Invalid announcement targets"
                );
            }
        }
        "protocol-notices.json" => {
            for client in value
                .as_object()
                .context("Invalid protocol notice identities")?
                .values()
            {
                ensure!(
                    client.is_object()
                        && (client["transport"] == "http" || client["transport"] == "websocket")
                        && client.get("version").is_none_or(valid_version),
                    "Invalid protocol client"
                );
                if let Some(id) = client.get("id") {
                    ensure!(
                        id.as_u64()
                            .is_some_and(|n| n > 0 && n <= 9_007_199_254_740_991)
                            && client["key"].is_string()
                            && at(&client["createdAt"]).is_some(),
                        "Invalid protocol notice identity"
                    );
                }
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn effective_controls(mut controls: Value, now: i64) -> Value {
    if controls.is_null() {
        controls = json!({"httpPush":{"enabled":true,"scheduledChange":null},"minimumClientVersion":{"version":null,"scheduledChange":null}});
    }
    if controls.get("logo").is_none() {
        controls["logo"] = json!({"version":"0.8.6","scheduledChange":null});
    }
    if at(&controls["logo"]["scheduledChange"]["at"]).is_some_and(|at| at <= now) {
        controls["logo"] = json!({"version":controls["logo"]["scheduledChange"]["version"],"scheduledChange":null});
    }
    if controls.get("forum").is_none() {
        controls["forum"] = json!({"enabled":false,"scheduledChange":null});
    }
    if at(&controls["forum"]["scheduledChange"]["at"]).is_some_and(|at| at <= now) {
        controls["forum"] = json!({"enabled":controls["forum"]["scheduledChange"]["enabled"],"scheduledChange":null});
    }
    if controls["httpPush"].get("scheduledChange").is_none() {
        controls["httpPush"]["scheduledChange"] = Value::Null;
    }
    if controls.get("minimumClientVersion").is_none() {
        controls["minimumClientVersion"] = json!({"version":null,"scheduledChange":null});
    }
    if at(&controls["httpPush"]["scheduledChange"]["at"]).is_some_and(|at| at <= now) {
        controls["httpPush"] = json!({"enabled":controls["httpPush"]["scheduledChange"]["enabled"],"scheduledChange":null});
    }
    if at(&controls["minimumClientVersion"]["scheduledChange"]["at"]).is_some_and(|at| at <= now) {
        controls["minimumClientVersion"] = json!({"version":controls["minimumClientVersion"]["scheduledChange"]["version"],"scheduledChange":null});
    }
    controls
}
// Explicit object ordering retains the SHA-256 policy identities already stored by the TS server.
fn scheduled_policy(controls: &Value) -> String {
    let floor = &controls["minimumClientVersion"]["scheduledChange"];
    let http = &controls["httpPush"]["scheduledChange"];
    let floor = if floor.is_null() {
        "null".to_owned()
    } else {
        format!(
            "{{\"version\":{},\"at\":{}}}",
            floor["version"], floor["at"]
        )
    };
    let http = if http.is_null() {
        "null".to_owned()
    } else {
        format!("{{\"enabled\":{},\"at\":{}}}", http["enabled"], http["at"])
    };
    format!("{{\"floor\":{floor},\"http\":{http}}}")
}
fn notice_content(
    controls: &Value,
    version: &Value,
    transport: &str,
    obsolete: bool,
    ignored_http: bool,
) -> Option<(String, String)> {
    let minimum = &controls["minimumClientVersion"]["version"];
    if obsolete || ignored_http {
        let mut text =
            "当前客户端不满足上报要求，提交将被忽略，无法更新服务端存档和排行榜。".to_owned();
        if ignored_http {
            text.push_str("HTTP Push 已停止接收内容，请改用 WebSocket Push。");
        }
        if obsolete {
            text.push_str(&format!(
                "客户端版本 {} 低于最低要求 {}。",
                version.as_str().unwrap_or("未上报或无效"),
                minimum.as_str().unwrap_or("")
            ));
        }
        text.push_str("必须升级 cortico-world-terrarium 到最新版本才能继续提交内容。只需在主人下次与你交谈时顺带说明升级要求，不要主动发送消息。");
        return Some((
            text,
            format!(
                "{{\"minimum\":{},\"httpPush\":{}}}",
                minimum, controls["httpPush"]["enabled"]
            ),
        ));
    }
    let floor = &controls["minimumClientVersion"]["scheduledChange"];
    let http = &controls["httpPush"]["scheduledChange"];
    let mut deadlines = vec![];
    if !floor.is_null() && !meets_minimum(version, &floor["version"]) {
        deadlines.push(format!(
            "{} 起最低版本将为 {}",
            floor["at"].as_str().unwrap_or(""),
            floor["version"].as_str().unwrap_or("")
        ));
    }
    if transport == "http" && http["enabled"] == false {
        deadlines.push(format!(
            "{} 起 HTTP Push 将停止接收",
            http["at"].as_str().unwrap_or("")
        ));
    }
    if deadlines.is_empty() {
        return None;
    }
    Some((
        format!(
            "升级预告：{}。期限前仍可按现行规则提交，请在宽限期内升级。只需在主人下次与你交谈时顺带提及，不要主动发送消息。",
            deadlines.join("；")
        ),
        scheduled_policy(controls),
    ))
}
impl Store {
    pub fn controls(&self, now: i64) -> Result<Value> {
        Ok(effective_controls(
            document(&self.reader()?, "controls.json")?.unwrap_or(Value::Null),
            now,
        ))
    }
    pub fn gate(&self, id: &str, version: &Value, transport: &str, now: i64) -> Result<Gate> {
        self.transaction(|tx| {
            let controls =
                effective_controls(document(tx, "controls.json")?.unwrap_or(Value::Null), now);
            let obsolete = !meets_minimum(version, &controls["minimumClientVersion"]["version"]);
            let ignored_http = transport == "http" && controls["httpPush"]["enabled"] == false;
            let original = document(tx, "protocol-notices.json")?.unwrap_or_else(|| json!({}));
            let mut clients = original.clone();
            let client = clients
                .as_object_mut()
                .unwrap()
                .entry(id.to_string())
                .or_insert_with(|| json!({}));
            client["transport"] = json!(transport);
            if valid_version(version) {
                client["version"] = version.clone();
            } else {
                client.as_object_mut().unwrap().remove("version");
            }
            let notice = if let Some((text, policy)) =
                notice_content(&controls, version, transport, obsolete, ignored_http)
            {
                let key = format!("{:x}", Sha256::digest(policy.as_bytes()));
                let matches = |client: &Value| {
                    client["key"] == key
                        && client["id"].as_u64().is_some()
                        && at(&client["createdAt"]).is_some_and(|at| at + 30 * 86400000 > now)
                };
                if !matches(&clients[id]) {
                    let shared = clients
                        .as_object()
                        .unwrap()
                        .values()
                        .find(|other| matches(other))
                        .cloned();
                    let (notice_id, created) = if let Some(shared) = shared {
                        (shared["id"].clone(), shared["createdAt"].clone())
                    } else {
                        let mut announcements = document(tx, "announcements.json")?
                            .unwrap_or_else(|| json!({"nextId": 1, "announcements": []}));
                        let n = announcements["nextId"].as_u64().unwrap();
                        ensure!(n < 9_007_199_254_740_991, "Announcement cursor exhausted");
                        announcements["nextId"] = json!(n + 1);
                        save_document(tx, "announcements.json", &announcements)?;
                        (json!(n), json!(iso(now)))
                    };
                    clients[id]["id"] = notice_id;
                    clients[id]["createdAt"] = created;
                    clients[id]["key"] = json!(key);
                }
                let client = &clients[id];
                Some(json!({
                    "id": client["id"], "title": "客户端需要升级", "text": text,
                    "targets": [id], "createdAt": client["createdAt"],
                    "expiresAt": iso(at(&client["createdAt"]).unwrap() + 30 * 86400000)
                }))
            } else {
                None
            };
            if clients != original {
                save_document(tx, "protocol-notices.json", &clients)?;
            }
            let rejection = if obsolete {
                Some(426)
            } else if ignored_http {
                Some(410)
            } else {
                None
            };
            Ok(Gate {
                controls,
                notice,
                rejection,
            })
        })
    }

    pub fn announcement_page(
        &self,
        id: Option<&str>,
        after: u64,
        latest: bool,
        notice: Option<Value>,
        now: i64,
    ) -> Result<Value> {
        let conn = self.reader()?;
        let data = document(&conn, "announcements.json")?
            .unwrap_or_else(|| json!({"nextId":1,"announcements":[]}));
        let mut visible: Vec<_> = data["announcements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| {
                at(&n["expiresAt"]).is_some_and(|at| at > now)
                    && (n["targets"] == "all"
                        || id.is_some_and(|id| {
                            n["targets"]
                                .as_array()
                                .is_some_and(|a| a.iter().any(|v| v == id))
                        }))
                    && (latest || n["id"].as_u64().unwrap_or(0) > after)
            })
            .cloned()
            .collect();
        if let Some(notice) = notice.filter(|n| n["id"].as_u64().unwrap_or(0) > after) {
            visible.push(notice);
        }
        visible.sort_by_key(|n| n["id"].as_u64().unwrap_or(0));
        let cursor = data["nextId"].as_u64().unwrap() - 1;
        if latest {
            let len = visible.len();
            visible = visible.into_iter().skip(len.saturating_sub(20)).collect();
            return Ok(json!({"announcements":visible,"nextCursor":cursor}));
        }
        let next = if visible.len() > 20 {
            visible[19]["id"].as_u64().unwrap()
        } else {
            cursor
        };
        visible.truncate(20);
        Ok(json!({"announcements":visible,"nextCursor":next}))
    }
    pub fn observed_client(&self, id: &str) -> Result<Value> {
        let conn = self.reader()?;
        if let Some(doc) = document(&conn, "protocol-notices.json")?
            && let Some(client) = doc.get(id)
        {
            return Ok(client.clone());
        }
        let entry = self.get(id)?;
        Ok(
            json!({"version":entry.as_ref().map(|e|e["clientVersion"].clone()).unwrap_or(Value::Null),"transport":entry.as_ref().and_then(|e|e["reportTransport"].as_str()).unwrap_or("http")}),
        )
    }
}
