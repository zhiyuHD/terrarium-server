//! Bounded administration responses retain the legacy full-list API for existing callers.
use crate::{
    metrics::{Metrics, memory_bytes},
    operations::OperationError,
    policy::{at, iso},
    store::Store,
};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxQuery {
    pub view: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    pub q: Option<String>,
    pub version: Option<String>,
    pub scope: Option<String>,
    pub ids: Option<String>,
    pub pending_only: Option<bool>,
}
fn version(row: &Value) -> &str {
    row["clientVersion"]
        .as_str()
        .unwrap_or(if row["reportedAt"].is_string() {
            "≤0.7.3（推定）"
        } else {
            "未上报"
        })
}
impl BoxQuery {
    pub fn page(&self, count: usize) -> Result<(usize, usize, usize)> {
        let size = self.page_size.unwrap_or(50);
        ensure!(
            (1..=100).contains(&size) && self.page.unwrap_or(1) > 0,
            OperationError::new(400, "Invalid pagination")
        );
        let pages = count.div_ceil(size).max(1);
        Ok((self.page.unwrap_or(1).min(pages), size, pages))
    }
}
pub fn boxes(store: &Store, metrics: &Metrics, query: &BoxQuery, now: i64) -> Result<Value> {
    let mut boxes = store.admin_boxes()?;
    metrics.annotate(&mut boxes);
    let mut response = json!({"serverAt":iso(now),"uptimeSec":metrics.uptime(),"memoryBytes":memory_bytes(),"acceptedReports":metrics.accepted_reports()});
    let view = query.view.as_deref().unwrap_or("");
    if view.is_empty() {
        response["boxes"] = json!(boxes);
        return Ok(response);
    }
    if view == "statistics" {
        let scope = query.scope.as_deref().unwrap_or("reported");
        ensure!(
            ["reported", "recent", "online"].contains(&scope),
            OperationError::new(400, "Invalid statistics scope")
        );
        response["statistics"] = statistics(&boxes, scope, now);
        return Ok(response);
    }
    if view == "lookup" {
        let ids: Vec<String> = serde_json::from_str(query.ids.as_deref().unwrap_or("[]"))
            .map_err(|_| OperationError::new(400, "Invalid lookup IDs"))?;
        ensure!(
            ids.len() <= 100,
            OperationError::new(400, "Too many lookup IDs")
        );
        let ids: HashSet<_> = ids.iter().map(String::as_str).collect();
        response["boxes"] = json!(
            boxes
                .into_iter()
                .filter(|b| ids.contains(b["id"].as_str().unwrap()))
                .collect::<Vec<_>>()
        );
        return Ok(response);
    }
    ensure!(
        ["page", "ids"].contains(&view),
        OperationError::new(400, "Invalid box view")
    );
    let search = query.q.as_deref().unwrap_or("").trim().to_lowercase();
    ensure!(
        search.chars().count() <= 100 && query.version.as_deref().unwrap_or("").len() <= 100,
        OperationError::new(400, "Invalid box filter")
    );
    let mut versions: Vec<_> = boxes.iter().map(|b| version(b).to_owned()).collect();
    versions.sort();
    versions.dedup();
    response["total"] = json!(boxes.len());
    response["versions"] = json!(versions);
    let notices = store.list_announcements()?;
    response["counts"] = json!({"registered":boxes.len(),"online":boxes.iter().filter(|b|b["wsConnections"].as_u64().unwrap_or(0)>0).count(),"pendingFeedback":store.admin_feedback_pending()?,"activeNotices":notices["announcements"].as_array().unwrap().iter().filter(|n|at(&n["expiresAt"]).is_some_and(|t|t>now)).count()});
    boxes.retain(|b| {
        query
            .version
            .as_deref()
            .is_none_or(|v| v == "*" || v == version(b))
            && (search.is_empty()
                || format!(
                    "{} {}",
                    b["name"].as_str().unwrap_or(""),
                    b["id"].as_str().unwrap()
                )
                .to_lowercase()
                .contains(&search))
    });
    boxes.sort_by(|a, b| {
        b["reportedAt"]
            .as_str()
            .cmp(&a["reportedAt"].as_str())
            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
    });
    response["filtered"] = json!(boxes.len());
    if view == "ids" {
        boxes.retain(|b| b["registered"] == true);
        response["tooMany"] = json!(boxes.len() > 100);
        response["boxes"] = if boxes.len() > 100 {
            json!([])
        } else {
            json!(
                boxes
                    .iter()
                    .map(|b| json!({"id":b["id"],"name":b["name"],"registered":true}))
                    .collect::<Vec<_>>()
            )
        };
        return Ok(response);
    }
    let (page, size, pages) = query.page(boxes.len())?;
    response["page"] = json!(page);
    response["pageSize"] = json!(size);
    response["pages"] = json!(pages);
    response["boxes"] = json!(
        boxes
            .into_iter()
            .skip((page - 1) * size)
            .take(size)
            .collect::<Vec<_>>()
    );
    Ok(response)
}
fn statistics(boxes: &[Value], scope: &str, now: i64) -> Value {
    let reported: Vec<_> = boxes
        .iter()
        .filter(|b| b["reportedAt"].is_string())
        .collect();
    let recent =
        |b: &&Value| at(&b["reportedAt"]).is_some_and(|t| t <= now && now - t <= 86_400_000);
    let selected: Vec<_> = reported
        .iter()
        .copied()
        .filter(|b| match scope {
            "recent" => recent(b),
            "online" => b["wsConnections"].as_u64().unwrap_or(0) > 0,
            _ => true,
        })
        .collect();
    let titles = [
        "World 版本",
        "操作系统",
        "系统版本",
        "部署识别",
        "Coopanion 版本",
        "生态箱类型",
        "最近提交方式",
    ];
    let mut dimensions: [BTreeMap<String, usize>; 7] = Default::default();
    for b in &selected {
        let system = b["clientSystem"]["name"].as_str();
        let os_version = b["clientSystem"]["version"].as_str();
        let labels = [
            b["clientVersion"]
                .as_str()
                .unwrap_or("≤0.7.3（推定）")
                .to_owned(),
            system.unwrap_or("未上报").to_owned(),
            system
                .zip(os_version)
                .map(|(s, v)| format!("{s} · {v}"))
                .unwrap_or("未上报".into()),
            if b["coopanionVersion"].is_string() {
                "Coopanion"
            } else {
                "未识别"
            }
            .into(),
            b["coopanionVersion"]
                .as_str()
                .unwrap_or("未上报")
                .to_owned(),
            match b["kind"].as_str() {
                Some("land") => "地箱",
                Some("water") => "水箱",
                _ => "未知",
            }
            .into(),
            match b["reportTransport"].as_str() {
                Some("websocket") => "WebSocket",
                Some("http") => "HTTP",
                _ => "未记录",
            }
            .into(),
        ];
        for (counts, label) in dimensions.iter_mut().zip(labels) {
            *counts.entry(label).or_default() += 1;
        }
    }
    let charts: Vec<_> = titles
        .into_iter()
        .zip(dimensions)
        .map(|(title, counts)| {
            let mut rows: Vec<_> = counts
                .into_iter()
                .map(|(label, count)| json!({"label":label,"count":count}))
                .collect();
            rows.sort_by(|a, b| {
                b["count"]
                    .as_u64()
                    .cmp(&a["count"].as_u64())
                    .then_with(|| a["label"].as_str().cmp(&b["label"].as_str()))
            });
            json!({"title":title,"rows":rows})
        })
        .collect();
    json!({"total":selected.len(),"reported":reported.len(),"unreported":boxes.len()-reported.len(),"recent":reported.iter().copied().filter(recent).count(),"charts":charts})
}
