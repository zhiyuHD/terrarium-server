use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use std::{fs, net::SocketAddr};
use tempfile::TempDir;
use terrarium_server::{
    admin::{self, AdminNetwork},
    assets,
    http::{App, router},
    store::Store,
};
use tower::ServiceExt;
#[tokio::test]
async fn released_pages_pin_assets_and_listeners_separate_historical_files() {
    let dir = TempDir::new().unwrap();
    let release = dir.path().join("versions/fixture-old");
    fs::create_dir_all(release.join("console/assets")).unwrap();
    fs::write(release.join("scene.js"), "public-old-scene").unwrap();
    fs::write(release.join("console/client.js"), "private-old-client").unwrap();
    fs::write(
        release.join("console/assets/MiSans-Regular.woff2"),
        b"fixture-font",
    )
    .unwrap();
    fs::write(release.join("admin-token"), "never-serve").unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    let app = App {
        store,
        release_id: Some("fixture-new".into()),
        release_root: Some(dir.path().into()),
        ingestion: None,
    };
    let public = router(app.clone());
    let private = admin::router(
        app,
        AdminNetwork::new("127.0.0.1:8788".parse().unwrap(), "127.0.0.1/32").unwrap(),
    );
    for (api, path, status, content) in [
        (
            public.clone(),
            "/",
            StatusCode::OK,
            "/releases/fixture-new/scene.js",
        ),
        (
            private.clone(),
            "/",
            StatusCode::OK,
            "/releases/fixture-new/client.js",
        ),
        (
            public.clone(),
            "/releases/fixture-old/scene.js",
            StatusCode::OK,
            "public-old-scene",
        ),
        (
            private.clone(),
            "/releases/fixture-old/client.js",
            StatusCode::OK,
            "private-old-client",
        ),
        (
            private.clone(),
            "/releases/fixture-old/assets/MiSans-Regular.woff2",
            StatusCode::OK,
            "fixture-font",
        ),
        (
            public.clone(),
            "/releases/fixture-old/client.js",
            StatusCode::NOT_FOUND,
            "",
        ),
        (
            private.clone(),
            "/releases/fixture-old/scene.js",
            StatusCode::NOT_FOUND,
            "",
        ),
        (
            public.clone(),
            "/releases/fixture-old/admin-token",
            StatusCode::NOT_FOUND,
            "",
        ),
        (
            private.clone(),
            "/releases/fixture-old/admin-token",
            StatusCode::NOT_FOUND,
            "",
        ),
        (
            public.clone(),
            "/releases/../scene.js",
            StatusCode::NOT_FOUND,
            "",
        ),
        (
            public.clone(),
            "/releases/fixture-old/../admin-token",
            StatusCode::NOT_FOUND,
            "",
        ),
        (
            public.clone(),
            "/releases/missing/scene.js",
            StatusCode::NOT_FOUND,
            "",
        ),
    ] {
        let response = api
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("host", "127.0.0.1:8788")
                    .extension(ConnectInfo(
                        "127.0.0.1:45678".parse::<SocketAddr>().unwrap(),
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{path}");
        if status == StatusCode::OK {
            assert_eq!(
                response.headers()["cache-control"],
                if path == "/" {
                    "no-store"
                } else {
                    "public, max-age=31536000, immutable"
                }
            );
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&bytes).contains(content));
    }
    assert!(
        assets::historical(
            Some(dir.path().into()),
            "/releases/fixture-old/%2e%2e/admin-token".into(),
            true
        )
        .await
        .unwrap()
        .is_none()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            release.join("admin-token"),
            release.join("console/client.css"),
        )
        .unwrap();
        assert!(
            assets::historical(
                Some(dir.path().into()),
                "/releases/fixture-old/client.css".into(),
                true
            )
            .await
            .is_err()
        );
    }
}
