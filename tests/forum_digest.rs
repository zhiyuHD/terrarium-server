use axum::http::Method;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use tempfile::TempDir;
use terrarium_server::{
    forum, forum_digest,
    store::{LegacySnapshot, Store},
};
fn at(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s)
        .unwrap()
        .timestamp_millis()
}
fn fixture(count: usize, slot: i64) -> (TempDir, Store) {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("data.sqlite")).unwrap();
    for id in ["garden-a", "water-b"] {
        s.register(Some(id)).unwrap();
    }
    s.update_named_control("forum", &json!({"enabled":true}), 0)
        .unwrap();
    let c = forum::open(&s).unwrap();
    for i in 0..count {
        c.execute(
            "INSERT INTO posts(thread,author,title,body,created) VALUES(NULL,'garden-a',?1,?2,?3)",
            rusqlite::params![
                format!("Leaf {i}"),
                format!("Observation {i}"),
                slot - 10000 - i as i64 * 3_600_000
            ],
        )
        .unwrap();
    }
    (d, s)
}
fn batches(s: &Store) -> Value {
    s.observed_client("@forum-digest").unwrap()["forumDigest"]["batches"].clone()
}
#[test]
fn selects_five_unique_top_ten_posts_once_and_delivers_only_authenticated_protocol_notices() {
    let slot = at("2026-10-22T08:00:00+08:00");
    let (_d, s) = fixture(12, slot);
    let announcement = s
        .publish_announcement(&json!({"text":"Garden update"}), slot - 1000)
        .unwrap();
    let top = forum::execute(
        &s,
        &Method::GET,
        "threads",
        &HashMap::from([
            ("sort".to_owned(), "recommended".to_owned()),
            ("limit".to_owned(), "10".to_owned()),
        ]),
        &Value::Null,
        "",
        slot,
    )
    .unwrap();
    let top: HashSet<i64> = top["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_i64().unwrap())
        .collect();
    assert!(!forum_digest::publish_due(&s, slot - 1).unwrap());
    assert!(forum_digest::publish_due(&s, slot).unwrap());
    assert!(!forum_digest::publish_due(&s, slot + 1000).unwrap());
    let digest = batches(&s);
    let ids = digest[0]["threadIds"].as_array().unwrap();
    assert_eq!(ids.len(), 5);
    assert_eq!(
        ids.iter()
            .filter_map(Value::as_i64)
            .collect::<HashSet<_>>()
            .len(),
        5
    );
    assert!(ids.iter().all(|id| top.contains(&id.as_i64().unwrap())));
    let a = s
        .announcement_page(
            Some("garden-a"),
            announcement["id"].as_u64().unwrap(),
            false,
            None,
            slot,
        )
        .unwrap();
    let b = s
        .announcement_page(
            Some("water-b"),
            announcement["id"].as_u64().unwrap(),
            false,
            None,
            slot,
        )
        .unwrap();
    assert_eq!(a, b);
    assert_eq!(a["announcements"].as_array().unwrap().len(), 1);
    assert_eq!(a["nextCursor"], announcement["id"].as_u64().unwrap() + 1);
    let notice = &a["announcements"][0];
    assert_eq!(notice["title"], "本日推荐");
    assert!(
        notice["text"]
            .as_str()
            .unwrap()
            .contains("2026-10-22 08:00")
    );
    assert!(
        notice["text"]
            .as_str()
            .unwrap()
            .starts_with("[forum-digest]")
    );
    assert!(
        s.announcement_page(
            Some("garden-a"),
            a["nextCursor"].as_u64().unwrap(),
            false,
            None,
            slot + 1
        )
        .unwrap()["announcements"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        s.list_announcements().unwrap()["announcements"],
        json!([announcement])
    );
    assert_eq!(
        s.announcement_page(None, 0, true, None, slot).unwrap()["announcements"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        s.list_feedback(0).unwrap()["feedback"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn restart_export_and_concurrent_ticks_do_not_redraw_and_evening_is_a_new_bounded_batch() {
    let slot = at("2026-10-22T08:00:00+08:00");
    let (d, s) = fixture(12, slot);
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let s = s.clone();
            std::thread::spawn(move || forum_digest::publish_due(&s, slot).unwrap())
        })
        .collect();
    assert_eq!(
        workers
            .into_iter()
            .filter_map(|w| w.join().ok())
            .filter(|v| *v)
            .count(),
        1
    );
    let first = batches(&s);
    drop(s);
    let s = Store::existing(d.path().join("data.sqlite")).unwrap();
    assert!(!forum_digest::publish_due(&s, slot + 500).unwrap());
    assert_eq!(batches(&s), first);
    let exported = d.path().join("export");
    s.export(&exported).unwrap();
    let imported = Store::open(d.path().join("imported.sqlite")).unwrap();
    imported
        .import(LegacySnapshot::read(&exported).unwrap())
        .unwrap();
    assert!(!forum_digest::publish_due(&imported, slot + 600).unwrap());
    assert_eq!(batches(&imported), first);
    let evening = slot + 12 * 3_600_000;
    assert!(forum_digest::publish_due(&s, evening).unwrap());
    assert_eq!(batches(&s).as_array().unwrap().len(), 2);
    assert!(
        batches(&s)[1]["notice"]["text"]
            .as_str()
            .unwrap()
            .contains("20:00")
    );
    assert!(forum_digest::publish_due(&s, slot + 86_400_000).unwrap());
    assert_eq!(batches(&s).as_array().unwrap().len(), 2);
    assert!(
        s.announcement_page(Some("garden-a"), 0, false, None, slot + 2 * 86_400_000)
            .unwrap()["announcements"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn closed_empty_small_forums_and_missed_slots_do_not_generate_extra_notices() {
    let slot = at("2026-10-22T20:00:00+08:00");
    let (_d, s) = fixture(3, slot);
    s.update_named_control("forum", &json!({"enabled":false}), slot)
        .unwrap();
    assert!(!forum_digest::publish_due(&s, slot).unwrap());
    s.update_named_control("forum", &json!({"enabled":true}), slot)
        .unwrap();
    assert!(!forum_digest::publish_due(&s, slot + 60_000).unwrap());
    assert!(forum_digest::publish_due(&s, slot + 500).unwrap());
    assert_eq!(batches(&s)[0]["threadIds"].as_array().unwrap().len(), 3);
    s.update_named_control("forum", &json!({"enabled":false}), slot)
        .unwrap();
    assert!(
        s.announcement_page(Some("garden-a"), 0, false, None, slot + 1000)
            .unwrap()["announcements"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let (_d, empty) = fixture(0, slot);
    assert!(!forum_digest::publish_due(&empty, slot).unwrap());
    assert!(!forum_digest::publish_due(&empty, slot + 1).unwrap());
    assert!(batches(&empty).as_array().unwrap().is_empty());
    assert_eq!(
        empty.announcement_page(None, 0, false, None, slot).unwrap()["nextCursor"],
        0
    );
}
