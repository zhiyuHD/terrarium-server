use axum::http::Method;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Barrier},
    thread,
};
use tempfile::TempDir;
use terrarium_server::{
    forum,
    ingestion::now_ms,
    policy::iso,
    store::{LegacySnapshot, Store},
};

fn entry(id: &str, kind: &str) -> Value {
    let mut state: Value =
        serde_json::from_str::<Value>(include_str!("fixtures/reports.json")).unwrap()[kind].clone();
    state["updatedAtMs"] = json!(now_ms());
    if kind == "land" {
        let p = &mut state["plants"][0];
        p["species"] = json!("sunflower");
        p["x"] = json!(4);
        p["y"] = json!(1);
        p["growthHours"] = json!(18);
        p["stage"] = json!(3);
    }
    json!({"id":id,"kind":kind,"state":state,"score":0,"reportedAt":iso(now_ms()),"reportRevision":"first"})
}
fn fixture() -> (TempDir, Store, String, i64) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    store
        .update_named_control("forum", &json!({"enabled":true}), now_ms())
        .unwrap();
    let token = store.register(Some("story-author")).unwrap().unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    store.register(Some("garden-first")).unwrap();
    store.register(Some("garden-second")).unwrap();
    let post = forum::execute(&store,&Method::POST,"threads",&HashMap::new(),&json!({"id":"story-author","title":"A garden note","body":"An optional story","clientVersion":"0.8.7"}),&token,now_ms()).unwrap();
    (dir, store, token, post["id"].as_i64().unwrap())
}
fn participate(s: &Store, root: i64, id: &str) {
    let conn = forum::open(s).unwrap();
    conn.execute("INSERT INTO posts(thread,author,title,body,created) VALUES(?1,?2,'','I will take part',?3)",params![root,id,now_ms()-1000]).unwrap();
}
fn notices(s: &Store) -> Vec<Value> {
    s.list_announcements().unwrap()["announcements"]
        .as_array()
        .unwrap()
        .clone()
}
fn configure(s: &Store, root: i64) {
    s.update_story(&json!({"enabled":true,"threadId":root}))
        .unwrap();
}

#[test]
fn opt_in_position_species_and_growth_threshold_produce_one_private_notice_across_restart_and_export()
 {
    let (dir, s, _, root) = fixture();
    configure(&s, root);
    let mut first = entry("garden-first", "land");
    assert!(s.commit_entry(&first, None).unwrap());
    assert!(notices(&s).is_empty());
    participate(&s, root, "garden-first");
    for (key, value) in [
        ("x", json!(3)),
        ("species", json!("mint")),
        ("growthHours", json!(17.99)),
    ] {
        let mut next = first.clone();
        next["state"]["plants"][0][key] = value;
        next["reportRevision"] = json!(format!("wrong-{key}"));
        assert!(s.commit_entry(&next, Some(&first)).unwrap());
        first = next;
        assert!(notices(&s).is_empty());
        let mut reset = entry("garden-first", "land");
        reset["state"]["plants"][0]["growthHours"] = json!(0);
        reset["reportRevision"] = json!(format!("reset-{key}"));
        assert!(s.commit_entry(&reset, Some(&first)).unwrap());
        first = reset;
    }
    let mut completed = entry("garden-first", "land");
    completed["reportRevision"] = json!("complete");
    assert!(s.commit_entry(&completed, Some(&first)).unwrap());
    let sent = notices(&s);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["targets"], json!(["garden-first"]));
    assert_eq!(s.story_status().unwrap()["completedBoxes"], 1);
    assert!(!s.commit_entry(&completed, Some(&first)).unwrap());
    let restored = Store::existing(dir.path().join("data.sqlite")).unwrap();
    restored
        .revoke_announcement(sent[0]["id"].as_u64().unwrap())
        .unwrap();
    let mut again = completed.clone();
    again["reportRevision"] = json!("after-restart");
    assert!(restored.commit_entry(&again, Some(&completed)).unwrap());
    assert!(notices(&restored).is_empty());
    let output = dir.path().join("export");
    restored.export(&output).unwrap();
    assert!(output.join("story.json").is_file());
    let imported = Store::open(dir.path().join("imported.sqlite")).unwrap();
    imported
        .import(LegacySnapshot::read(&output).unwrap())
        .unwrap();
    assert_eq!(
        imported.story_status().unwrap(),
        restored.story_status().unwrap()
    );
    let backup = dir.path().join("backup.sqlite");
    restored.backup(&backup).unwrap();
    assert_eq!(
        Store::existing(backup).unwrap().story_status().unwrap(),
        restored.story_status().unwrap()
    );
}
#[test]
fn disabled_water_and_stale_reports_do_not_receive_story_notices() {
    let (_dir, s, _, root) = fixture();
    participate(&s, root, "garden-first");
    let first = entry("garden-first", "land");
    assert!(s.commit_entry(&first, None).unwrap());
    assert!(notices(&s).is_empty());
    configure(&s, root);
    let mut second = first.clone();
    second["reportRevision"] = json!("fresh");
    second["state"]["updatedAtMs"] = json!(0);
    assert!(s.commit_entry(&second, Some(&first)).unwrap());
    assert!(notices(&s).is_empty());
    let water = entry("garden-second", "water");
    participate(&s, root, "garden-second");
    assert!(s.commit_entry(&water, None).unwrap());
    assert!(notices(&s).is_empty());
    s.update_story(&json!({"enabled":false})).unwrap();
    let mut later = entry("garden-first", "land");
    later["reportRevision"] = json!("off");
    assert!(s.commit_entry(&later, Some(&second)).unwrap());
    assert!(notices(&s).is_empty());
}
#[test]
fn failed_story_write_rolls_back_both_report_and_notice_and_can_retry() {
    let (dir, s, _, root) = fixture();
    configure(&s, root);
    participate(&s, root, "garden-first");
    let conn = Connection::open(dir.path().join("data.sqlite")).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_story BEFORE INSERT ON legacy_documents WHEN new.name='story.json' BEGIN SELECT RAISE(ABORT,'story write failed'); END;").unwrap();
    let e = entry("garden-first", "land");
    assert!(s.commit_entry(&e, None).is_err());
    assert!(s.get("garden-first").unwrap().is_none());
    assert!(notices(&s).is_empty());
    assert_eq!(s.story_status().unwrap()["completedBoxes"], 0);
    conn.execute_batch("DROP TRIGGER fail_story;").unwrap();
    assert!(s.commit_entry(&e, None).unwrap());
    assert_eq!(notices(&s).len(), 1);
}
#[test]
fn concurrent_completions_share_the_global_cursor_and_do_not_repeat_per_box() {
    let (_dir, s, _, root) = fixture();
    configure(&s, root);
    for id in ["garden-first", "garden-second"] {
        participate(&s, root, id);
    }
    let barrier = Arc::new(Barrier::new(4));
    let workers: Vec<_> = (0..4)
        .map(|i| {
            let s = s.clone();
            let b = barrier.clone();
            thread::spawn(move || {
                let e = entry(
                    if i % 2 == 0 {
                        "garden-first"
                    } else {
                        "garden-second"
                    },
                    "land",
                );
                b.wait();
                s.commit_entry(&e, None).unwrap()
            })
        })
        .collect();
    assert_eq!(
        workers
            .into_iter()
            .map(|w| w.join().unwrap())
            .filter(|accepted| *accepted)
            .count(),
        2
    );
    let sent = notices(&s);
    assert_eq!(sent.len(), 2);
    assert!(sent[0]["id"].as_u64().unwrap() < sent[1]["id"].as_u64().unwrap());
    assert_eq!(s.story_status().unwrap()["completedBoxes"], 2);
}
#[test]
fn configuration_requires_a_real_root_and_cannot_retarget_after_delivery() {
    let (_dir, s, _, root) = fixture();
    for input in [
        json!({"enabled":true}),
        json!({"threadId":999}),
        json!({"threadId":-1}),
        json!({"enabled":"yes"}),
        json!({"x":4}),
    ] {
        assert!(s.update_story(&input).is_err());
    }
    configure(&s, root);
    participate(&s, root, "garden-first");
    assert!(
        s.commit_entry(&entry("garden-first", "land"), None)
            .unwrap()
    );
    assert!(
        s.update_story(&json!({"enabled":false,"threadId":null}))
            .is_err()
    );
    assert_eq!(
        s.update_story(&json!({"enabled":false})).unwrap()["enabled"],
        false
    );
}
