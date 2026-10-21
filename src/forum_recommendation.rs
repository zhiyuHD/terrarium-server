//! Recommendation cursors retain at most 64 immutable ID lists for ten minutes.
use crate::{forum, operations::OperationError, store::Store};
use anyhow::Result;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{LazyLock, Mutex},
};
const TTL: i64 = 600_000;
const CANDIDATES: i64 = 2000;
struct Snapshot {
    token: i64,
    database: PathBuf,
    filters: Vec<String>,
    created: i64,
    ids: Vec<i64>,
}
static SNAPSHOTS: LazyLock<Mutex<VecDeque<Snapshot>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));
fn score(created: i64, now: i64, likes: i64, replies: i64, views: i64) -> f64 {
    let age = now.saturating_sub(created).max(0) as f64 / 86_400_000.0;
    100.0 * (1.0 - age)
        + 8.0 * likes.clamp(0, 20) as f64 / 20.0
        + 6.0 * replies.clamp(0, 20) as f64 / 20.0
        + 3.0 * views.clamp(0, 100) as f64 / 100.0
}
pub(crate) fn page(
    store: &Store,
    conn: &Connection,
    query: &HashMap<String, String>,
    before: i64,
    limit: i64,
    now: i64,
) -> Result<Value> {
    let database = store.forum_path();
    let filters: Vec<String> = ["q", "author", "followingOf"]
        .iter()
        .map(|k| query.get(*k).cloned().unwrap_or_default())
        .collect();
    let fresh = if before == i64::MAX {
        let mut rows = forum::thread_rows(conn, query, i64::MAX, CANDIDATES)?;
        let rank = |p: &Value| {
            let created = chrono::DateTime::parse_from_rfc3339(p["createdAt"].as_str().unwrap())
                .unwrap()
                .timestamp_millis();
            score(
                created,
                now,
                p["likes"].as_i64().unwrap(),
                p["replies"].as_i64().unwrap(),
                p["views"].as_i64().unwrap(),
            )
        };
        rows.sort_by(|a, b| {
            rank(b)
                .total_cmp(&rank(a))
                .then_with(|| b["id"].as_i64().cmp(&a["id"].as_i64()))
        });
        Some(
            rows.iter()
                .map(|p| p["id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let (ids, next, created) = {
        let mut snapshots = SNAPSHOTS.lock().unwrap();
        snapshots.retain(|s| now.saturating_sub(s.created) < TTL);
        let (token, offset) = if let Some(ids) = fresh {
            let token = loop {
                let n = i64::from_str_radix(&crate::store::random_hex(4), 16)?;
                if n != 0 && !snapshots.iter().any(|s| s.token == n) {
                    break n;
                }
            };
            if snapshots.len() == 64 {
                snapshots.pop_front();
            }
            snapshots.push_back(Snapshot {
                token,
                database: database.clone(),
                filters: filters.clone(),
                created: now,
                ids,
            });
            (token, 0)
        } else {
            (before / 4096, (before % 4096) as usize)
        };
        let s = snapshots
            .iter()
            .find(|s| s.token == token && s.database == database)
            .ok_or_else(|| OperationError::new(410, "推荐分页已过期，请刷新列表"))?;
        if s.filters != filters || offset > s.ids.len() {
            return Err(OperationError::new(400, "Invalid recommendation cursor").into());
        }
        let end = (offset + limit as usize).min(s.ids.len());
        (
            s.ids[offset..end].to_vec(),
            if end < s.ids.len() {
                Some(token * 4096 + end as i64)
            } else {
                None
            },
            s.created,
        )
    };
    let mut rows = Vec::new();
    for id in ids {
        if let Some(mut p) = forum::post(conn, id)? {
            p["body"] = json!(
                p["body"]
                    .as_str()
                    .unwrap()
                    .chars()
                    .take(240)
                    .collect::<String>()
            );
            rows.push(p);
        }
    }
    Ok(
        json!({"threads":rows,"nextCursor":next,"sort":"recommended","rankedAt":crate::policy::iso(created),"externalContent":true}),
    )
}
#[cfg(test)]
mod tests {
    use super::score;
    #[test]
    fn time_declines_continuously_crosses_zero_and_dominates_capped_engagement() {
        let day = 86_400_000;
        assert_eq!(score(0, 0, 0, 0, 0), 100.0);
        assert_eq!(score(0, day / 2, 0, 0, 0), 50.0);
        assert_eq!(score(0, day, 0, 0, 0), 0.0);
        assert!(score(0, day - 1, 0, 0, 0) > 0.0);
        assert!(score(0, day + 1, 0, 0, 0) < 0.0);
        assert_eq!(score(0, 0, 20, 20, 100), 117.0);
        assert_eq!(score(0, 0, i64::MAX, i64::MAX, i64::MAX), 117.0);
        assert!(score(0, day / 4, 1000, 1000, 1000) < score(day / 4, day / 4, 0, 0, 0));
        for (l, r, v) in [(1, 0, 0), (0, 1, 0), (0, 0, 1)] {
            assert!(score(0, 0, l, r, v) > 100.0);
        }
    }
}
