use serde_json::{Value, json};
use tempfile::TempDir;
use terrarium_server::{
    ingestion::{Ingestion, now_ms},
    neighbors::execute,
    store::{LegacySnapshot, Store},
};
#[test]
fn neighbor_permissions_mailbox_retries_and_daily_limits_survive_export() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    let a = store.register(Some("garden-a")).unwrap().unwrap();
    let b = store.register(Some("garden-b")).unwrap().unwrap();
    let at = 1_800_000_000_000i64;
    let mut state: Value = serde_json::from_str::<Value>(include_str!("fixtures/reports.json"))
        .unwrap()["land"]
        .clone();
    state["updatedAtMs"] = json!(at);
    let entry = json!({"id":"garden-b","kind":"land","state":state,"phase":"day","score":0,"reportedAt":terrarium_server::policy::iso(at),"clientVersion":"0.9.0","reportRevision":"initial"});
    store.commit_entry(&entry, None).unwrap();
    let token = a["token"].as_str().unwrap();
    let other = b["token"].as_str().unwrap();
    assert!(execute(&store, &json!({"id":"garden-a"}), "wrong", at).is_err());
    assert!(
        execute(
            &store,
            &json!({"id":"garden-a","action":"gift","target":"garden-b","species":"tomato"}),
            token,
            at
        )
        .is_err()
    );
    execute(
        &store,
        &json!({"id":"garden-a","action":"configure","enabled":true}),
        token,
        at,
    )
    .unwrap();
    execute(
        &store,
        &json!({"id":"garden-b","action":"configure","enabled":true}),
        other,
        at,
    )
    .unwrap();
    for _ in 0..5 {
        execute(
            &store,
            &json!({"id":"garden-a","action":"gift","target":"garden-b","species":"tomato"}),
            token,
            at,
        )
        .unwrap();
    }
    assert!(
        execute(
            &store,
            &json!({"id":"garden-a","action":"gift","target":"garden-b","species":"tomato"}),
            token,
            at
        )
        .is_err()
    );
    let inbox = execute(&store, &json!({"id":"garden-b"}), other, at).unwrap();
    assert_eq!(inbox["mail"].as_array().unwrap().len(), 5);
    let last = inbox["mail"].as_array().unwrap().last().unwrap()["id"].clone();
    execute(
        &store,
        &json!({"id":"garden-b","action":"ack","cursor":last}),
        other,
        at,
    )
    .unwrap();
    assert_eq!(
        execute(&store, &json!({"id":"garden-b"}), other, at).unwrap()["mail"],
        json!([])
    );
    let snapshot = dir.path().join("export");
    store.export(&snapshot).unwrap();
    assert!(snapshot.join("neighbors.json").exists());
    let restored = Store::open(dir.path().join("restored.sqlite")).unwrap();
    restored
        .import(LegacySnapshot::read(&snapshot).unwrap())
        .unwrap();
    assert_eq!(
        execute(&restored, &json!({"id":"garden-b"}), other, at).unwrap()["enabled"],
        true
    );
}
#[tokio::test]
async fn cached_gate_applies_control_changes_and_version_changes_immediately() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    store.register(Some("garden-cache")).unwrap();
    let ingestion = Ingestion::new(store.clone());
    let now = now_ms();
    assert!(
        ingestion
            .gate("garden-cache", &json!("0.9.0"), "websocket")
            .await
            .unwrap()
            .rejection
            .is_none()
    );
    store
        .update_named_control("minimumClientVersion", &json!({"version":"0.9.1"}), now)
        .unwrap();
    assert_eq!(
        ingestion
            .gate("garden-cache", &json!("0.9.0"), "websocket")
            .await
            .unwrap()
            .rejection,
        Some(426)
    );
    assert!(
        ingestion
            .gate("garden-cache", &json!("0.9.1"), "websocket")
            .await
            .unwrap()
            .rejection
            .is_none()
    );
}
#[tokio::test]
async fn a_burst_of_eight_valid_reports_waits_for_processing_slots_and_commits() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    let ingestion = Ingestion::new(store.clone());
    ingestion.ready().await.unwrap();
    let mut tasks = Vec::new();
    for i in 0..8 {
        let id = format!("garden-burst-{i}");
        store.register(Some(&id)).unwrap();
        let worker = ingestion.clone();
        let mut state: Value = serde_json::from_str::<Value>(include_str!("fixtures/reports.json"))
            .unwrap()["land"]
            .clone();
        state["updatedAtMs"] = json!(now_ms());
        tasks.push(tokio::spawn(async move {
            worker
                .commit(
                    &id,
                    &json!({"state":state,"phase":"day","clientVersion":"0.9.0"}),
                    "websocket",
                )
                .await
        }));
    }
    for task in tasks {
        assert!(task.await.unwrap().is_ok());
    }
}
