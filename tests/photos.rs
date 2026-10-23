use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tempfile::TempDir;
use terrarium_server::{
    http::{App, router},
    protocol::state_hash,
    store::Store,
};
use tower::ServiceExt;
async fn post(app: axum::Router, input: Value, token: &str) -> Response {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/api/photos")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(input.to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn photos_authenticate_and_capture_only_the_acknowledged_land_or_water_snapshot() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/reports.json")).unwrap();
    let assets = dir.path().join("versions/photo-test/console/assets");
    std::fs::create_dir_all(&assets).unwrap();
    for name in [
        "photo-worker.mjs",
        "photo-canvas.node",
        "MiSans-Regular.woff2",
        "MiSans-Semibold.woff2",
    ] {
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("photo/dist")
                .join(name),
            assets.join(name),
        )
        .unwrap();
    }
    let api = router(App {
        store: store.clone(),
        release_id: Some("photo-test".into()),
        release_root: Some(dir.path().to_owned()),
        ingestion: None,
    });
    for kind in ["land", "water"] {
        let id = format!("photo-{kind}");
        let token = store.register(Some(&id)).unwrap().unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        let entry = json!({"id":id,"kind":kind,"score":120,"state":fixtures[kind],"reportedAt":"2026-10-22T12:00:00.000Z","phase":"day","reportRevision":"photo-1"});
        assert!(store.commit_entry(&entry, None).unwrap());
        let entry = store.get(&id).unwrap().unwrap();
        let hash = state_hash(&entry["state"]);
        let input = json!({"id":id,"stateHash":hash,"caption":"一个有花的下午","phase":"day","clientVersion":"0.8.9"});
        assert_eq!(
            post(api.clone(), input.clone(), "incorrect").await.status(),
            StatusCode::UNAUTHORIZED
        );
        let mut stale = input.clone();
        stale["stateHash"] = json!("0".repeat(64));
        assert_eq!(
            post(api.clone(), stale, &token).await.status(),
            StatusCode::CONFLICT
        );
        let response = post(api.clone(), input.clone(), &token).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-terrarium-state-hash"], hash);
        assert_eq!(response.headers()["content-type"], "image/png");
        let photo_url = response.headers()["x-terrarium-photo-url"]
            .to_str()
            .unwrap()
            .to_owned();
        let expiry = response.headers()["x-terrarium-photo-expires-at"]
            .to_str()
            .unwrap()
            .to_owned();
        let uri: axum::http::Uri = photo_url.parse().unwrap();
        assert_eq!(uri.scheme_str(), Some("https"));
        let expiry_ms = terrarium_server::policy::at(&json!(expiry)).unwrap();
        assert_eq!(
            terrarium_server::photo_cache::parse_key(uri.path().rsplit('/').next().unwrap()),
            Some(expiry_ms)
        );
        let original = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&original[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            u32::from_be_bytes(original[16..20].try_into().unwrap()),
            1280
        );
        assert_eq!(
            u32::from_be_bytes(original[20..24].try_into().unwrap()),
            1600
        );
        let download = api
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri.path())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(download.status(), StatusCode::OK);
        assert_eq!(download.headers()["cache-control"], "no-store");
        assert_eq!(download.headers()["content-type"], "image/png");
        assert_eq!(
            download.into_body().collect().await.unwrap().to_bytes(),
            original
        );
        let now = terrarium_server::ingestion::now_ms();
        let page = store
            .announcement_page(Some(&id), 0, false, None, now)
            .unwrap();
        let receipt = page["announcements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["title"] == "拍照链接回执")
            .unwrap();
        let data: Value = serde_json::from_str(
            receipt["text"]
                .as_str()
                .unwrap()
                .strip_prefix("[photo-result]")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(data["url"], photo_url);
        assert_eq!(data["stateHash"], hash);
        assert_eq!(data["expiresAt"], expiry);
        assert!(
            store.list_announcements().unwrap()["announcements"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            store.announcement_page(None, 0, false, None, now).unwrap()["announcements"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let other = if kind == "land" {
            "photo-water"
        } else {
            "photo-land"
        };
        assert!(
            store
                .announcement_page(Some(other), 0, false, None, now)
                .unwrap()["announcements"]
                .as_array()
                .unwrap()
                .iter()
                .all(|n| n["text"].as_str().unwrap().find(&photo_url).is_none())
        );
        terrarium_server::forum_bridge::prepare_poll(
            &store,
            &id,
            &json!("0.8.9"),
            page["nextCursor"].as_u64().unwrap(),
            now,
        )
        .unwrap();
        assert!(
            store.observed_client(&id).unwrap()["photoReplies"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.get(&id).unwrap().unwrap(), entry);
        let mut next = entry.clone();
        next["state"]["name"] = json!("变了的院子");
        next["reportRevision"] = json!("photo-2");
        assert!(store.commit_entry(&next, Some(&entry)).unwrap());
        assert_eq!(
            post(api.clone(), input, &token).await.status(),
            StatusCode::CONFLICT
        );
        let preserved = api
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri.path())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            preserved.into_body().collect().await.unwrap().to_bytes(),
            original
        );
        let updated = post(
            api.clone(),
            json!({"id":id,"stateHash":state_hash(&next["state"]),"caption":"一个有花的下午","phase":"day","clientVersion":"0.8.9"}),
            &token,
        )
        .await;
        assert_eq!(updated.status(), StatusCode::OK);
        assert_ne!(
            updated.into_body().collect().await.unwrap().to_bytes(),
            original
        );
    }
    for input in [
        Value::Null,
        json!({"id":"https://example.test","stateHash":"0".repeat(64)}),
        json!({"id":"photo-land","stateHash":"bad"}),
        json!({"id":"photo-land","stateHash":"0".repeat(64),"caption":"花".repeat(121)}),
        json!({"id":"photo-land","stateHash":"0".repeat(64),"caption":12}),
    ] {
        assert_eq!(
            post(api.clone(), input, "").await.status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn expired_and_unknown_photo_urls_never_serve_bytes() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    let folder = store.photo_dir();
    std::fs::create_dir_all(&folder).unwrap();
    let key = format!("1-{}.png", "a".repeat(64));
    std::fs::write(folder.join(&key), b"expired").unwrap();
    let api = router(App {
        store,
        release_id: None,
        release_root: None,
        ingestion: None,
    });
    for path in [
        format!("/api/photos/{key}"),
        format!("/api/photos/9999999999999-{}.png", "b".repeat(64)),
        "/api/photos/not-a-photo.png".into(),
    ] {
        let response = api
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    assert!(!folder.join(key).exists());
}
