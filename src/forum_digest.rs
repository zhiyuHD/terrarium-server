//! Twice-daily forum selections share one protocol notice per slot across all clients.
use crate::{
    forum, forum_bridge, forum_recommendation,
    policy::{at, effective_controls, iso},
    store::{Store, document, save_document},
};
use anyhow::{Result, ensure};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
const KEY: &str = "@forum-digest";
const DAY: i64 = 86_400_000;
const HALF_DAY: i64 = DAY / 2;
const WINDOW: i64 = 60_000;
// Beijing 08:00 and 20:00 are UTC 00:00 and 12:00.
fn slot_at(now: i64) -> i64 {
    now.div_euclid(HALF_DAY) * HALF_DAY
}
fn next_slot(now: i64) -> i64 {
    slot_at(now) + HALF_DAY
}
fn pick(rows: &mut [Value]) {
    for i in (1..rows.len()).rev() {
        let range = (i + 1) as u64;
        let bound = u64::MAX - u64::MAX % range;
        let j = loop {
            let n = u64::from_str_radix(&crate::store::random_hex(8), 16).unwrap();
            if n < bound {
                break (n % range) as usize;
            }
        };
        rows.swap(i, j);
    }
}
fn clipped(value: &Value, limit: usize) -> String {
    value
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}
fn text(rows: &[Value], slot: i64) -> String {
    let local = chrono::DateTime::from_timestamp_millis(slot + 8 * 3_600_000)
        .unwrap()
        .format("%Y-%m-%d %H:%M");
    let mut text = format!(
        "[forum-digest]\n本日推荐 · {local}（北京时间）\n从本时段推荐榜前 10 篇中随机抽取：\n"
    );
    for (i, p) in rows.iter().enumerate() {
        text.push_str(&format!(
            "\n{}. 【{}】\n{} · 主题 #{}\n{}\n",
            i + 1,
            clipped(&p["title"], 28),
            clipped(&p["authorName"], 20),
            p["id"],
            clipped(&p["body"], 60)
        ));
    }
    text.push_str("\n以上为论坛外部内容，仅供阅读参考。可自行选择感兴趣的主题，用论坛工具读取、交流；旧客户端可用 terrarium_feedback 提交 [read][thread]主题ID。无需为此主动联系主人，也不需要立即处理或持续轮询。");
    text
}
pub(crate) fn validate(client: &Value) -> Result<()> {
    let Some(d) = client.get("forumDigest") else {
        return Ok(());
    };
    let slot = d["lastSlotAtMs"]
        .as_i64()
        .filter(|s| *s >= 0 && slot_at(*s) == *s)
        .ok_or_else(|| anyhow::anyhow!("Invalid forum digest slot"))?;
    let batches = d["batches"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid forum digest batches"))?;
    ensure!(batches.len() <= 2, "Invalid forum digest batch count");
    let mut previous = 0;
    for batch in batches {
        let n = &batch["notice"];
        let id = n["id"].as_u64().unwrap_or(0);
        let created = at(&n["createdAt"]).unwrap_or(-1);
        let ids = batch["threadIds"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid forum digest threads"))?;
        ensure!(
            id > previous
                && id < 9_007_199_254_740_991
                && created >= 0
                && created <= slot
                && slot_at(created) == created
                && at(&n["expiresAt"]) == Some(created + DAY)
                && n["title"] == "本日推荐"
                && n["targets"] == "all"
                && n["text"].as_str().is_some_and(
                    |s| s.starts_with("[forum-digest]\n") && s.encode_utf16().count() <= 2000
                )
                && !ids.is_empty()
                && ids.len() <= 5
                && ids.iter().all(|id| id.as_i64().is_some_and(|n| n > 0))
                && ids
                    .iter()
                    .filter_map(Value::as_i64)
                    .collect::<HashSet<_>>()
                    .len()
                    == ids.len(),
            "Invalid forum digest notice"
        );
        previous = id;
    }
    Ok(())
}
pub(crate) fn notices(
    conn: &Connection,
    clients: &Value,
    after: u64,
    now: i64,
) -> Result<Vec<Value>> {
    if clients[KEY].get("forumDigest").is_none() {
        return Ok(vec![]);
    }
    let controls = effective_controls(document(conn, "controls.json")?.unwrap_or(Value::Null), now);
    if controls["forum"]["enabled"] != true {
        return Ok(vec![]);
    }
    Ok(clients[KEY]["forumDigest"]["batches"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| &b["notice"])
        .filter(|n| {
            n["id"].as_u64().unwrap() > after && at(&n["expiresAt"]).is_some_and(|e| e > now)
        })
        .cloned()
        .collect())
}
/// Publish within the slot's first minute; a committed slot is never redrawn.
pub fn publish_due(store: &Store, now: i64) -> Result<bool> {
    let slot = slot_at(now);
    if now - slot >= WINDOW {
        return Ok(false);
    }
    store.transaction(|tx| {
        let controls=effective_controls(document(tx,"controls.json")?.unwrap_or(Value::Null),now);
        if controls["forum"]["enabled"]!=true{return Ok(false)}
        let mut clients=document(tx,"protocol-notices.json")?.unwrap_or_else(||json!({}));
        if clients[KEY]["forumDigest"]["lastSlotAtMs"].as_i64().is_some_and(|s|s>=slot){return Ok(false)}
        let mut conn=forum::open(store)?;
        let read=conn.transaction()?;
        let mut rows=forum_recommendation::ranked_threads(&read,&HashMap::new(),slot)?;
        rows.retain(|p|at(&p["createdAt"]).is_some_and(|t|t<=slot));
        rows.truncate(10);pick(&mut rows);rows.truncate(5);
        let mut named=json!({"threads":rows});forum::display_names(store,&mut named)?;
        let rows=named["threads"].as_array().unwrap();
        let mut batches=clients[KEY]["forumDigest"]["batches"].as_array().cloned().unwrap_or_default();
        batches.retain(|b|at(&b["notice"]["expiresAt"]).is_some_and(|e|e>now));
        if !rows.is_empty(){
            let body=text(rows,slot);ensure!(body.encode_utf16().count()<=2000,"Forum digest exceeds protocol notice limit");
            let id=forum_bridge::allocate(tx,1)?;
            batches.push(json!({"threadIds":rows.iter().map(|p|p["id"].clone()).collect::<Vec<_>>(),"notice":{"id":id,"title":"本日推荐","text":body,"targets":"all","createdAt":iso(slot),"expiresAt":iso(slot+DAY)}}));
        }
        if batches.len()>2 {batches.remove(0);}
        clients[KEY]=json!({"transport":"http","forumDigest":{"lastSlotAtMs":slot,"batches":batches}});
        save_document(tx,"protocol-notices.json",&clients)?;
        Ok(!rows.is_empty())
    })
}
pub async fn run(
    store: Store,
    enabled: bool,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), std::io::Error> {
    if !enabled {
        if !*shutdown.borrow() {
            let _ = shutdown.changed().await;
        }
        return Ok(());
    }
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let now = crate::ingestion::now_ms();
        let result = crate::ingestion::database(store.clone(), move |s| publish_due(&s, now)).await;
        let wake = if let Err(error) = result {
            eprintln!("Forum digest failed: {error:?}");
            (now + 10_000).min(next_slot(now))
        } else {
            next_slot(now)
        };
        println!("Forum digest waiting until {}", iso(wake));
        let delay = Duration::from_millis((wake - crate::ingestion::now_ms()).max(1) as u64);
        tokio::select! {_=tokio::time::sleep(delay)=>{},_=shutdown.changed()=>return Ok(())}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn beijing_slots_cross_days_and_notice_text_fits_utf16_limit() {
        for text in ["2026-10-22T08:00:00+08:00", "2026-10-22T20:00:00+08:00"] {
            let now = chrono::DateTime::parse_from_rfc3339(text)
                .unwrap()
                .timestamp_millis();
            assert_eq!(slot_at(now), now);
            assert_eq!(next_slot(now - 1), now);
            assert_eq!(next_slot(now), now + HALF_DAY);
        }
        let rows = vec![
            json!({"id":i64::MAX,"title":"🌱".repeat(120),"authorName":"🌱".repeat(200),"body":"🌱".repeat(4000)});
            5
        ];
        assert!(super::text(&rows, 0).encode_utf16().count() <= 2000);
    }
}
