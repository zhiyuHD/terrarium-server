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
    let engagement = |count: i64, weight: f64, replies: bool| {
        let count = count.max(0) as u64;
        let baseline: f64 = (1..=20).map(action_multiplier).sum();
        weight / baseline * interaction_total(count, replies)
    };
    100.0 * (1.0 - age)
        + engagement(likes, 8.0, false)
        + engagement(replies, 6.0, true)
        + 3.0 * views.clamp(0, 100) as f64 / 100.0
}

/// The marginal value peaks around the fiftieth interaction, then falls to 0.1x at 100.
/// Interactions after 100 remain valuable at 0.1x, so their total contribution is unbounded.
fn action_multiplier(index: u64) -> f64 {
    match index {
        1..=50 => 1.0 + 4.0 * (index - 1) as f64 / 49.0,
        51..=100 => 5.0 - 4.9 * (index - 50) as f64 / 50.0,
        _ => 0.1,
    }
}
fn reply_multiplier(index: u64) -> f64 {
    match index {
        106..=110 => 7.5,
        111.. => 0.5,
        _ => action_multiplier(index),
    }
}
fn interaction_total(count: u64, replies: bool) -> f64 {
    if replies {
        return reply_total(count);
    }
    standard_total(count)
}
fn standard_total(count: u64) -> f64 {
    let first = count.min(50) as f64;
    let first_sum = first
        * (1.0
            + if first > 0.0 {
                1.0 + 4.0 * (first - 1.0) / 49.0
            } else {
                0.0
            })
        / 2.0;
    let middle_count = count.saturating_sub(50).min(50) as f64;
    let middle_sum = middle_count
        * (4.9
            + if middle_count > 0.0 {
                5.0 - 4.9 * middle_count / 50.0
            } else {
                0.0
            })
        / 2.0;
    first_sum + middle_sum + count.saturating_sub(100) as f64 * 0.1
}
fn reply_total(count: u64) -> f64 {
    let peak = (106..=110).map(reply_multiplier).sum::<f64>();
    standard_total(count.min(105))
        + count.saturating_sub(105).min(5) as f64 * (peak / 5.0)
        + count.saturating_sub(110) as f64 * 0.5
}
pub(crate) fn ranked_threads(
    conn: &Connection,
    query: &HashMap<String, String>,
    now: i64,
) -> Result<Vec<Value>> {
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
    Ok(rows)
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
        let rows = ranked_threads(conn, query, now)?;
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
    fn time_declines_without_floor_and_each_engagement_has_a_midrange_peak() {
        let day = 86_400_000;
        assert_eq!(score(0, 0, 0, 0, 0), 100.0);
        assert_eq!(score(0, day / 2, 0, 0, 0), 50.0);
        assert_eq!(score(0, day, 0, 0, 0), 0.0);
        assert!(score(0, day - 1, 0, 0, 0) > 0.0);
        assert!(score(0, day + 1, 0, 0, 0) < 0.0);
        assert!((score(0, 0, 20, 20, 100) - 117.0).abs() < 1e-9);
        assert!(score(0, 0, 50, 0, 0) > score(0, 0, 49, 0, 0));
        assert!(
            score(0, 0, 51, 0, 0) - score(0, 0, 50, 0, 0)
                > score(0, 0, 100, 0, 0) - score(0, 0, 99, 0, 0)
        );
        assert!(
            (score(0, 0, 100, 0, 0) - score(0, 0, 99, 0, 0))
                / (score(0, 0, 1, 0, 0) - score(0, 0, 0, 0, 0))
                - 0.1
                < 1e-9
        );
        let reply_base = score(0, 0, 0, 105, 0);
        let reply_after_peak = score(0, 0, 0, 106, 0) - reply_base;
        let reply_peak = score(0, 0, 0, 110, 0) - score(0, 0, 0, 109, 0);
        let reply_after_drop = score(0, 0, 0, 111, 0) - score(0, 0, 0, 110, 0);
        assert!((reply_after_peak - reply_peak).abs() < 1e-9);
        assert!((reply_after_drop - reply_peak / 15.0).abs() < 1e-9);
        assert!(score(0, 0, i64::MAX, 0, 0) > score(0, 0, 100, 0, 0));
        assert!(score(0, day + 1, 0, 0, 0) < 0.0);
        assert!(score(0, day / 4, 1000, 1000, 1000) > score(0, day + 1, 1000, 1000, 1000));
        for (l, r, v) in [(1, 0, 0), (0, 1, 0), (0, 0, 1)] {
            assert!(score(0, 0, l, r, v) > 100.0);
        }
    }
}
