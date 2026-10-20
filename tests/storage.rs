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
        ("feedback.json", "{\"nextId\":5,\"entries\":[]}"),
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
