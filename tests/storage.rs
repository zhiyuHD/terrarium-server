use serde_json::{Value, json};
use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
};
use tempfile::TempDir;
use terrarium_server::{
    scoring::{score, score_parts},
    store::{LegacySnapshot, Store},
};
fn entry(id: &str) -> Value {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/scores.json")).unwrap();
    json!({"id":id,"kind":"land","state":cases[0]["state"],"phase":"day","score":cases[0]["score"],"reportedAt":"2026-10-20T00:00:00.000Z","reportRevision":"initial","clientVersion":"0.8.5"})
}
fn fixture() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let old = dir.path().join("old");
    fs::create_dir(&old).unwrap();
    fs::write(
        old.join("leaderboard.json"),
        json!({"box-first":entry("box-first"),"box-second":entry("box-second")}).to_string(),
    )
    .unwrap();
    fs::write(
        old.join("credentials.json"),
        b"{\"box-first\":\"fixture-token\",\"box-empty\":\"unclaimed-token\"}",
    )
    .unwrap();
    for (name, bytes) in [
        ("admin-token", "fixture-admin"),
        ("controls.json", "{\"httpPush\":{\"enabled\":false}}"),
        ("announcements.json", "{\"nextId\":9,\"announcements\":[]}"),
        ("feedback.json", "{\"nextId\":5,\"feedback\":[]}"),
        ("protocol-notices.json", "{}"),
    ] {
        fs::write(old.join(name), bytes).unwrap();
    }
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    store.import(LegacySnapshot::read(&old).unwrap()).unwrap();
    (dir, store)
}
#[test]
fn preserves_all_legacy_documents_and_tokens_on_roundtrip_and_backup() {
    let (dir, store) = fixture();
    let output = dir.path().join("export");
    store.export(&output).unwrap();
    for item in fs::read_dir(dir.path().join("old")).unwrap() {
        let path = item.unwrap().path();
        assert_eq!(
            fs::read(&path).unwrap(),
            fs::read(output.join(path.file_name().unwrap())).unwrap()
        );
    }
    assert!(
        store
            .import(LegacySnapshot::read(&output).unwrap())
            .is_err()
    );
    let backup = dir.path().join("backup.sqlite");
    store.backup(&backup).unwrap();
    let restored = Store::existing(backup).unwrap();
    restored.check().unwrap();
    assert_eq!(
        restored.get("box-first").unwrap(),
        store.get("box-first").unwrap()
    );
    assert!(store.export(&output).is_err());
}
#[test]
fn one_box_update_keeps_other_boxes_and_tokens_and_rejects_stale_revisions() {
    let (dir, store) = fixture();
    let other = store.get("box-second").unwrap();
    let mut changed = entry("box-first");
    changed["reportRevision"] = json!("new");
    changed["state"]["name"] = json!("Updated");
    store
        .replace_report("box-first", Some("initial"), &changed)
        .unwrap();
    assert_eq!(other, store.get("box-second").unwrap());
    assert!(
        store
            .replace_report("box-first", Some("initial"), &changed)
            .is_err()
    );
    let output = dir.path().join("export");
    store.export(&output).unwrap();
    let exported: Value =
        serde_json::from_slice(&fs::read(output.join("leaderboard.json")).unwrap()).unwrap();
    assert_eq!(exported["box-first"]["state"]["name"], "Updated");
    assert_eq!(
        fs::read(output.join("credentials.json")).unwrap(),
        fs::read(dir.path().join("old/credentials.json")).unwrap()
    );
}
#[test]
fn simultaneous_revisions_commit_only_one_and_readers_keep_a_snapshot() {
    let (_dir, store) = fixture();
    let mut reader = store.reader().unwrap();
    let tx = reader.transaction().unwrap();
    let initial: String = tx
        .query_row("SELECT revision FROM boxes WHERE id='box-first'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let threads: Vec<_> = (0..2)
        .map(|i| {
            let store = store.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                let mut changed = entry("box-first");
                changed["reportRevision"] = json!(format!("new-{i}"));
                barrier.wait();
                store
                    .replace_report("box-first", Some("initial"), &changed)
                    .is_ok()
            })
        })
        .collect();
    barrier.wait();
    assert_eq!(
        threads
            .into_iter()
            .map(|t| usize::from(t.join().unwrap()))
            .sum::<usize>(),
        1
    );
    let retained: String = tx
        .query_row("SELECT revision FROM boxes WHERE id='box-first'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(initial, retained);
    tx.commit().unwrap();
    store.check().unwrap();
}
#[test]
fn scoring_matches_64_typescript_reference_states() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/scores.json")).unwrap();
    for case in cases {
        assert_eq!(
            score(&case["state"]).unwrap(),
            case["score"].as_f64().unwrap()
        );
        let actual = score_parts(&case["state"]).unwrap();
        for (key, value) in actual.as_object().unwrap() {
            assert_eq!(value.as_f64(), case["parts"][key].as_f64());
        }
    }
}
#[test]
fn refuses_invalid_snapshot_before_import_and_leaves_existing_rows_unchanged() {
    let (dir, store) = fixture();
    let old = dir.path().join("old");
    fs::write(old.join("credentials.json"), "{broken").unwrap();
    assert!(LegacySnapshot::read(&old).is_err());
    assert_eq!(store.ranked(None).unwrap().len(), 2);
}

#[test]
fn opening_old_scores_rebuilds_all_ranks_without_changing_state_tokens_or_revisions() {
    use rusqlite::{Connection, params};
    let (dir, store) = fixture();
    for index in 0..24 {
        store
            .commit_entry(&entry(&format!("box-extra-{index:02}")), None)
            .unwrap();
    }
    let mut initial = entry("box-first");
    initial["state"]["plants"] = json!(
        (0..12)
            .map(|i| json!({"species":format!("plant-{i}"),"stage":3,"health":100}))
            .collect::<Vec<_>>()
    );
    initial["state"]["bugs"] = json!([]);
    store
        .replace_report("box-first", Some("initial"), &initial)
        .unwrap();
    let before = store.get("box-first").unwrap().unwrap();
    let path = dir.path().join("data.sqlite");
    drop(store);
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE boxes SET score=480,entry=json_set(entry,'$.score',480) WHERE id=?1",
        ["box-first"],
    )
    .unwrap();
    conn.execute("UPDATE box_score_rules SET version=?1", params![2])
        .unwrap();
    drop(conn);
    let store = Store::open(&path).unwrap();
    let after = store.get("box-first").unwrap().unwrap();
    assert_eq!(after["state"], before["state"]);
    assert_eq!(after["reportRevision"], before["reportRevision"]);
    assert_eq!(after["reportedAt"], before["reportedAt"]);
    assert_eq!(
        after["score"].as_f64().unwrap(),
        terrarium_server::scoring::curved_score(480.)
    );
    let summary = store.summary("land", 100, None).unwrap();
    assert_eq!(summary["entries"].as_array().unwrap().len(), 26);
    assert_eq!(summary["entries"][0]["id"], "box-first");
    let raw: String = store
        .reader()
        .unwrap()
        .query_row("SELECT entry FROM boxes WHERE id='box-first'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&raw).unwrap()["score"], 480);
    store.check().unwrap();
    let export = dir.path().join("curve-export");
    store.export(&export).unwrap();
    let exported: Value =
        serde_json::from_slice(&fs::read(export.join("leaderboard.json")).unwrap()).unwrap();
    assert_eq!(exported["box-first"]["score"], after["score"]);
    assert_eq!(exported["box-first"]["state"], before["state"]);
    assert_eq!(
        fs::read(export.join("credentials.json")).unwrap(),
        fs::read(dir.path().join("old/credentials.json")).unwrap()
    );
    assert_eq!(
        fs::read(export.join("announcements.json")).unwrap(),
        fs::read(dir.path().join("old/announcements.json")).unwrap()
    );
    drop(store);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.get("box-first").unwrap().unwrap(), after);
}

#[test]
fn failed_score_rebuild_rolls_back_formula_marker_and_every_score() {
    use rusqlite::Connection;
    let (dir, store) = fixture();
    let path = dir.path().join("data.sqlite");
    drop(store);
    let conn = Connection::open(&path).unwrap();
    conn.execute("UPDATE box_score_rules SET version=2", [])
        .unwrap();
    conn.execute("UPDATE boxes SET score=999", []).unwrap();
    conn.execute("UPDATE boxes SET entry=json_set(entry,'$.state.plants',json('[{}]')) WHERE id='box-second'", []).unwrap();
    drop(conn);
    assert!(Store::open(&path).is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT version FROM box_score_rules", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM boxes WHERE score=999", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}
