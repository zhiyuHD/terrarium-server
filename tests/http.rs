use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::fs;
use tempfile::TempDir;
use terrarium_server::{
    http::{App, router},
    store::{LegacySnapshot, Store},
};
use tower::ServiceExt;
async fn query(app: axum::Router, path: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}
#[tokio::test]
async fn serves_global_pagination_search_summary_replays_and_rejects_writes() {
    let dir = TempDir::new().unwrap();
    let old = dir.path().join("old");
    fs::create_dir(&old).unwrap();
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/scores.json")).unwrap();
    let mut entries = serde_json::Map::new();
    for i in 0..121 {
        let id = format!("box-{i:03}");
        let mut state = cases[0]["state"].clone();
        state["name"] = json!(if i == 120 {
            "Far garden".to_owned()
        } else {
            format!("Garden {i}")
        });
        entries.insert(id.clone(),json!({"id":id,"kind":"land","state":state,"score":0,"phase":"day","reportedAt":"2026-10-20T00:00:00Z"}));
    }
    fs::write(
        old.join("leaderboard.json"),
        Value::Object(entries).to_string(),
    )
    .unwrap();
    fs::write(old.join("credentials.json"), "{}").unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    store.import(LegacySnapshot::read(&old).unwrap()).unwrap();
    let app = router(App {
        store,
        release_id: Some("fixture".into()),
        ingestion: None,
    });
    let (status, page) = query(app.clone(), "/api/leaderboard?kind=land&view=gallery").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["entries"].as_array().unwrap().len(), 4);
    assert_eq!(page["pages"], 31);
    assert_eq!(page["counts"]["boxes"], 121);
    assert_eq!(page["entries"][0]["rank"], 1);
    assert!(page["entries"][0]["state"].get("history").is_none());
    let (_, last) = query(
        app.clone(),
        "/api/leaderboard?kind=land&view=gallery&page=999",
    )
    .await;
    assert_eq!(last["page"], 31);
    assert_eq!(last["entries"][0]["rank"], 121);
    let (_, search) = query(app.clone(), "/api/leaderboard?kind=land&view=gallery&q=FAR").await;
    assert_eq!(search["total"], 1);
    assert_eq!(search["entries"][0]["rank"], 121);
    let (_, summary) = query(
        app.clone(),
        "/api/leaderboard?kind=land&view=summary&limit=2&ownId=box-120",
    )
    .await;
    assert_eq!(summary["own"]["rank"], 121);
    assert_eq!(summary["entries"].as_array().unwrap().len(), 2);
    let (_, legacy) = query(app.clone(), "/api/leaderboard").await;
    assert_eq!(legacy.as_array().unwrap().len(), 100);
    let (_, replays) = query(app.clone(), "/api/boxes/box-000?view=replays").await;
    assert!(replays["replays"].is_array());
    for path in [
        "/api/leaderboard?view=gallery",
        "/api/leaderboard?kind=land&view=gallery&page=0",
        "/api/leaderboard?kind=land&view=gallery&pageSize=13",
        "/api/leaderboard?kind=invalid",
    ] {
        assert_eq!(query(app.clone(), path).await.0, StatusCode::BAD_REQUEST);
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/reports")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    assert_eq!(
        query(app.clone(), "/api/boxes/missing").await.0,
        StatusCode::NOT_FOUND
    );
    let (_, health) = query(app, "/health").await;
    assert_eq!(health["readOnly"], true);
    assert_eq!(health["releaseId"], "fixture");
}
