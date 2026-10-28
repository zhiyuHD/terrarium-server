use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tempfile::TempDir;
use terrarium_server::{
    http::{App, router},
    ingestion::{Ingestion, now_ms},
    store::Store,
};
use tower::ServiceExt;
fn state() -> Value {
    let mut s =
        serde_json::from_str::<Value>(include_str!("fixtures/reports.json")).unwrap()["land"]
            .clone();
    s["updatedAtMs"] = json!(now_ms());
    s
}
fn entry(id: &str) -> Value {
    json!({"id":id,"kind":"land","state":state(),"phase":"day","reportedAt":"2026-10-28T00:00:00Z","reportRevision":"first"})
}
#[test]
fn summaries_rebuild_after_old_binary_writes_and_follow_failed_transactions_atomically() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("data.sqlite");
    let store = Store::open(&path).unwrap();
    let first = entry("garden-cache");
    store.commit_entry(&first, None).unwrap();
    let saved = store.get("garden-cache").unwrap();
    let previous = store.summary("land", 20, Some("garden-cache")).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_summary BEFORE INSERT ON box_summaries BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    let mut next = first.clone();
    next["state"]["name"] = json!("Updated garden");
    next["reportRevision"] = json!("second");
    assert!(store.commit_entry(&next, Some(&first)).is_err());
    assert_eq!(store.get("garden-cache").unwrap(), saved);
    assert_eq!(
        store.summary("land", 20, Some("garden-cache")).unwrap(),
        previous
    );
    conn.execute_batch("DROP TRIGGER reject_summary").unwrap();
    // Older releases update boxes without maintaining this optional cache.
    conn.execute(
        "UPDATE boxes SET entry=?1,revision='second' WHERE id='garden-cache'",
        [next.to_string()],
    )
    .unwrap();
    drop(store);
    drop(conn);
    let restarted = Store::open(&path).unwrap();
    let actual = restarted.summary("land", 20, Some("garden-cache")).unwrap();
    assert_eq!(actual["own"]["name"], "Updated garden");
    assert_eq!(
        actual["own"]["scoreParts"],
        terrarium_server::scoring::score_parts(&next["state"]).unwrap()
    );
    let export = dir.path().join("export");
    restarted.export(&export).unwrap();
    assert!(!export.join("box_summaries").exists());
}
#[tokio::test]
async fn twenty_four_small_http_reports_share_actual_bytes_instead_of_three_maximum_reservations() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    let ingestion = Ingestion::new(store.clone());
    ingestion.ready().await.unwrap();
    let metrics = ingestion.metrics();
    let app = router(App {
        store: store.clone(),
        ingestion: Some(ingestion),
        release_id: None,
        release_root: None,
    });
    let mut tasks = Vec::new();
    for i in 0..24 {
        let id = format!("garden-{i}");
        let credentials = store.register(Some(&id)).unwrap().unwrap();
        let app = app.clone();
        let body = json!({"id":id,"state":state(),"phase":"day","clientVersion":"0.9.0"});
        tasks.push(tokio::spawn(async move {
            app.oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/reports")
                    .header(
                        "authorization",
                        format!("Bearer {}", credentials["token"].as_str().unwrap()),
                    )
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap(), StatusCode::OK);
    }
    let counts = metrics.processing_status();
    assert_eq!(counts["bufferBusy"], 0);
    assert_eq!(counts["admissionBusy"], 0);
    assert_eq!(counts["active"], 0);
    assert_eq!(counts["waiting"], 0);
    assert_eq!(counts["accepted"]["http"], 24);
    assert!(counts["peakActive"].as_u64().unwrap() <= 4);
}
