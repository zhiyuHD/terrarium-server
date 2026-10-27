//! Administrative documents and feedback commit atomically alongside protocol notice cursors.
use crate::{
    policy::{at, effective_controls, iso, validate_document},
    protocol::{valid_id, valid_version},
    store::{Store, document, random_hex, save_document},
};
use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fmt;
use subtle::ConstantTimeEq;
pub const MAX_FEEDBACK: usize = 2000;
pub const FEEDBACK_PAGE_SIZE: usize = 100;
pub const FEEDBACK_ANONYMOUS_LIMIT: usize = 20;
pub const FEEDBACK_BOX_LIMIT: usize = 5;
pub const FEEDBACK_RATE_MS: i64 = 60_000;
#[derive(Debug)]
pub struct OperationError {
    pub status: u16,
    pub message: String,
}
impl OperationError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}
impl fmt::Display for OperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for OperationError {}
fn reject<T>(status: u16, message: &str) -> Result<T> {
    Err(OperationError::new(status, message).into())
}
pub fn trim(value: &str) -> &str {
    value.trim_matches(|c| {
        matches!(c,
            '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}'
            | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
            | '\u{205f}' | '\u{3000}' | '\u{feff}'
        )
    })
}
pub fn parse_feedback(value: &Value) -> Result<Value> {
    let Some(text) = value["text"].as_str().filter(|s| {
        !trim(s).is_empty()
            && s.encode_utf16().count() <= 2000
            && !s.chars().any(|c| {
                matches!(c,
                    '\u{0000}'..='\u{0008}' | '\u{000b}' | '\u{000c}'
                    | '\u{000e}'..='\u{001f}' | '\u{007f}'
                )
            })
    }) else {
        return reject(
            400,
            "Feedback text must contain 1..2000 characters without control characters",
        );
    };
    let mut parsed = json!({"text":trim(text)});
    if let Some(contact) = value.get("contact") {
        let Some(contact) = contact.as_str().filter(|s| {
            s.encode_utf16().count() <= 200
                && !s
                    .chars()
                    .any(|c| matches!(c, '\u{0000}'..='\u{001f}' | '\u{007f}'))
        }) else {
            return reject(
                400,
                "Feedback contact must be at most 200 characters without control characters",
            );
        };
        if !trim(contact).is_empty() {
            parsed["contact"] = json!(trim(contact));
        }
    }
    Ok(parsed)
}
pub(crate) fn validate_feedback(value: &Value) -> Result<()> {
    let next = value["nextId"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= 9_007_199_254_740_991)
        .context("Invalid feedback cursor")?;
    let items = value["feedback"]
        .as_array()
        .context("Invalid feedback store")?;
    anyhow::ensure!(items.len() <= MAX_FEEDBACK, "Invalid feedback count");
    let mut previous = 0;
    for item in items {
        parse_feedback(item)?;
        let id = item["id"]
            .as_u64()
            .filter(|n| *n > previous && *n < next)
            .context("Invalid feedback ID")?;
        previous = id;
        anyhow::ensure!(
            matches!(item["source"].as_str(), Some("web" | "world" | "ai"))
                && at(&item["createdAt"]).is_some()
                && item.get("resolvedAt").is_none_or(|v| at(v).is_some())
                && item.get("clientVersion").is_none_or(valid_version)
                && item
                    .get("boxId")
                    .is_none_or(|v| v.as_str().is_some_and(valid_id)),
            "Invalid feedback metadata"
        );
    }
    Ok(())
}
fn announcements(conn: &rusqlite::Connection) -> Result<Value> {
    Ok(document(conn, "announcements.json")?
        .unwrap_or_else(|| json!({"nextId":1,"announcements":[]})))
}
fn feedback(conn: &rusqlite::Connection) -> Result<Value> {
    Ok(document(conn, "feedback.json")?.unwrap_or_else(|| json!({"nextId":1,"feedback":[]})))
}
impl Store {
    pub fn admin_token(&self, create: bool) -> Result<String> {
        let read = |conn: &rusqlite::Connection| -> Result<Option<String>> {
            let saved: Option<Vec<u8>> = conn
                .query_row(
                    "SELECT content FROM legacy_documents WHERE name='admin-token'",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            saved
                .map(|bytes| {
                    let raw = String::from_utf8(bytes)?;
                    let token = trim(&raw);
                    anyhow::ensure!(!token.is_empty(), "Empty admin token");
                    Ok(token.to_owned())
                })
                .transpose()
        };
        if let Some(token) = read(&self.reader()?)? {
            return Ok(token);
        }
        anyhow::ensure!(
            create,
            "Admin token is missing; initialize it with admin-token --create"
        );
        self.transaction(|tx| {
            if let Some(token) = read(tx)? {
                return Ok(token);
            }
            let token = random_hex(32);
            tx.execute(
                "INSERT INTO legacy_documents VALUES('admin-token',?1)",
                [token.as_bytes()],
            )?;
            Ok(token)
        })
    }
    pub fn admin_authenticated(&self, provided: &str) -> Result<bool> {
        let expected = self.admin_token(false)?;
        Ok(bool::from(
            Sha256::digest(provided.as_bytes()).ct_eq(&Sha256::digest(expected.as_bytes())),
        ))
    }
    pub fn admin_boxes(&self) -> Result<Vec<Value>> {
        let conn = self.reader()?;
        let mut statement = conn.prepare(
            "SELECT ids.id, credentials.id IS NOT NULL,
                json_extract(boxes.entry, '$.state.name'),
                CASE WHEN boxes.id IS NULL THEN NULL ELSE boxes.kind END,
                boxes.score, boxes.reported_at,
                json_extract(boxes.entry, '$.clientVersion'),
                json_extract(boxes.entry, '$.coopanionVersion'),
                json_extract(boxes.entry, '$.reportTransport'),
                json_extract(boxes.entry, '$.clientSystem')
             FROM (SELECT id FROM credentials UNION SELECT id FROM boxes) ids
             LEFT JOIN credentials ON ids.id=credentials.id
             LEFT JOIN boxes ON ids.id=boxes.id ORDER BY ids.id",
        )?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, bool>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<f64>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<String>>(9)?,
            ))
        })?;
        rows.map(|row| {
            let (id, registered, name, kind, score, reported, client, coopanion, transport, system) = row?;
            let system: Option<Value> = system.map(|text| serde_json::from_str(&text)).transpose()?;
            Ok(json!({
                "id": id,
                "registered": registered,
                "name": name.as_deref().unwrap_or(&id),
                "kind": kind,
                "score": score,
                "reportedAt": reported,
                "clientVersion": client,
                "coopanionVersion": coopanion,
                "clientSystem": system,
                "reportTransport": transport,
                "wsConnections": 0
            }))
        })
        .collect()
    }

    pub fn update_control(&self, http: bool, input: &Value, now: i64) -> Result<Value> {
        self.update_named_control(
            if http {
                "httpPush"
            } else {
                "minimumClientVersion"
            },
            input,
            now,
        )
    }
    pub fn update_named_control(&self, key: &str, input: &Value, now: i64) -> Result<Value> {
        let http = matches!(key, "httpPush" | "forum");
        if !matches!(key, "httpPush" | "minimumClientVersion" | "forum" | "logo") {
            return reject(400, "Unknown control");
        }
        if key == "logo"
            && input["cancel"] != true
            && !matches!(
                input["version"].as_str(),
                Some("0.8.6" | "0.8.7" | "0.8.8" | "0.8.9" | "0.8.10" | "0.9.0")
            )
        {
            return reject(400, "Unknown logo version");
        }
        self.transaction(|tx| {
            let mut controls = effective_controls(
                document(tx, "controls.json")?.unwrap_or(Value::Null), now,
            );
            let Some(object) = input.as_object() else {
                return reject(400, "Invalid control");
            };
            if input["cancel"] == true && object.len() == 1 {
                controls[key]["scheduledChange"] = Value::Null;
            } else {
                let field = if http { "enabled" } else { "version" };
                if object.keys().any(|k| k != field && k != "at")
                    || !object.contains_key(field)
                    || (http && !input[field].is_boolean())
                    || (!http && !crate::policy::stable_version(&input[field]))
                {
                    return reject(400, if http {
                        "Supply enabled and an optional timestamp with timezone"
                    } else {
                        "Supply a stable version (major.minor.patch) or null and an optional timestamp with timezone"
                    });
                }
                if let Some(timestamp) = input.get("at") {
                    let Some(time) = at(timestamp).filter(|t| *t > now) else {
                        return reject(400, "Scheduled time must be in the future and include a timezone");
                    };
                    controls[key]["scheduledChange"] = json!({field: input[field], "at": iso(time)});
                } else {
                    controls[key] = json!({field: input[field], "scheduledChange": null});
                }
            }
            save_document(tx, "controls.json", &controls)?;
            Ok(controls)
        })
    }
    pub fn list_announcements(&self) -> Result<Value> {
        let data = announcements(&self.reader()?)?;
        Ok(json!({"announcements": data["announcements"]}))
    }
    pub fn publish_announcement(&self, input: &Value, now: i64) -> Result<Value> {
        self.transaction(|tx| {
            let mut data = announcements(tx)?;
            let id = data["nextId"].as_u64().unwrap();
            if id >= 9_007_199_254_740_991 {
                return reject(503, "Announcement cursor exhausted");
            }
            let default = |key: &str, fallback: Value| {
                input.get(key).filter(|v| !v.is_null()).cloned().unwrap_or(fallback)
            };
            let notice = json!({
                "id": id,
                "title": default("title", json!("")),
                "text": input["text"],
                "targets": default("targets", json!("all")),
                "createdAt": iso(now),
                "expiresAt": default("expiresAt", json!(iso(now + 7 * 86400000)))
            });
            let valid = json!({"nextId": id + 1, "announcements": [notice.clone()]});
            if validate_document("announcements.json", &valid).is_err()
                || !at(&notice["expiresAt"]).is_some_and(|at| at > now && at <= now + 30 * 86400000)
            {
                return reject(400, "Invalid announcement: supply text, optional title, registered target IDs and expiry within 30 days");
            }
            if let Some(ids) = notice["targets"].as_array() {
                for target in ids {
                    let exists: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM credentials WHERE id=?1)",
                        [target.as_str().unwrap()], |r| r.get(0),
                    )?;
                    if !exists {
                        return reject(400, "Invalid announcement: supply registered target IDs");
                    }
                }
            }
            let entries = data["announcements"].as_array_mut().unwrap();
            entries.retain(|n| at(&n["expiresAt"]).is_some_and(|at| at > now));
            if entries.len() >= 2000 {
                return reject(400, "Announcement capacity reached; revoke an announcement first");
            }
            entries.push(notice.clone());
            data["nextId"] = json!(id + 1);
            save_document(tx, "announcements.json", &data)?;
            Ok(notice)
        })
    }
    pub fn revoke_announcement(&self, id: u64) -> Result<bool> {
        self.transaction(|tx| {
            let mut data = announcements(tx)?;
            let entries = data["announcements"].as_array_mut().unwrap();
            let count = entries.len();
            entries.retain(|n| n["id"] != id);
            if count == entries.len() {
                return Ok(false);
            }
            save_document(tx, "announcements.json", &data)?;
            Ok(true)
        })
    }
    pub fn list_feedback(&self, after: u64) -> Result<Value> {
        let data = feedback(&self.reader()?)?;
        let items: Vec<_> = data["feedback"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["id"].as_u64().unwrap() > after)
            .take(FEEDBACK_PAGE_SIZE)
            .cloned()
            .collect();
        Ok(
            json!({"nextCursor":items.last().map(|v|v["id"].clone()).unwrap_or(json!(after)),"feedback":items}),
        )
    }
    pub fn submit_feedback(&self, input: &Value, box_id: Option<&str>, now: i64) -> Result<Value> {
        self.transaction(|tx| {
            let mut entry = parse_feedback(input)?;
            let source = input
                .get("source")
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or(json!("web"));
            if !matches!(source.as_str(), Some("web" | "world" | "ai"))
                || input
                    .get("clientVersion")
                    .is_some_and(|v| !valid_version(v))
            {
                return reject(400, "Invalid feedback metadata");
            }
            let mut data = feedback(tx)?;
            let items = data["feedback"].as_array_mut().unwrap();
            let recent = items
                .iter()
                .filter(|v| {
                    v["boxId"].as_str() == box_id
                        && at(&v["createdAt"]).is_some_and(|at| at > now - FEEDBACK_RATE_MS)
                })
                .count();
            if recent
                >= if box_id.is_some() {
                    FEEDBACK_BOX_LIMIT
                } else {
                    FEEDBACK_ANONYMOUS_LIMIT
                }
            {
                return reject(429, "Feedback limit reached; retry after one minute");
            }
            if items.len() >= MAX_FEEDBACK {
                if let Some(index) = items.iter().position(|v| v.get("resolvedAt").is_some()) {
                    items.remove(index);
                } else {
                    return reject(503, "Feedback inbox is full");
                }
            }
            let id = data["nextId"].as_u64().unwrap();
            if id >= 9_007_199_254_740_991 {
                return reject(503, "Feedback cursor exhausted");
            }
            entry["id"] = json!(id);
            entry["createdAt"] = json!(iso(now));
            entry["source"] = source;
            if let Some(box_id) = box_id {
                entry["boxId"] = json!(box_id);
            }
            if let Some(version) = input.get("clientVersion") {
                entry["clientVersion"] = version.clone();
            }
            data["feedback"].as_array_mut().unwrap().push(entry);
            data["nextId"] = json!(id + 1);
            save_document(tx, "feedback.json", &data)?;
            Ok(json!({"id":id,"createdAt":iso(now)}))
        })
    }
    pub fn resolve_feedback(&self, id: u64, now: i64) -> Result<bool> {
        self.transaction(|tx| {
            let mut data = feedback(tx)?;
            let entry = data["feedback"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|v| v["id"] == id);
            let Some(entry) = entry else {
                return Ok(false);
            };
            if entry.get("resolvedAt").is_none() {
                entry["resolvedAt"] = json!(iso(now));
                save_document(tx, "feedback.json", &data)?;
            }
            Ok(true)
        })
    }
}
