use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{fs, net::SocketAddr, time::Duration};
use tempfile::TempDir;
use terrarium_server::{
    admin::{self, AdminNetwork},
    http::{self, App},
    ingestion::{Ingestion, now_ms},
    operations::{FEEDBACK_ANONYMOUS_LIMIT, FEEDBACK_BOX_LIMIT, FEEDBACK_RATE_MS, OperationError},
    policy::iso,
    store::{LegacySnapshot, Store},
};
use tower::ServiceExt;
fn fixture() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    store.admin_token(true).unwrap();
    (dir, store)
}
fn application(store: Store, writes: bool) -> App {
    App {
        ingestion: writes.then(|| Ingestion::new(store.clone())),
        store,
        release_id: None,
    }
}
fn private(app: App) -> Router {
    admin::router(
        app,
        AdminNetwork::new("127.0.0.1:8791".parse().unwrap(), "127.0.0.1/32").unwrap(),
    )
}
async fn request(
    api: Router,
    method: &str,
    path: &str,
    body: Value,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:8791")
        .extension(ConnectInfo(
            "127.0.0.1:45000".parse::<SocketAddr>().unwrap(),
        ));
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let response = api
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}
fn status(error: anyhow::Error) -> u16 {
    error.downcast::<OperationError>().unwrap().status
}
#[test]
fn private_admin_network_enforces_ipv4_cidr_host_and_origin_without_forwarded_header_trust() {
    for bind in ["0.0.0.0:80", "8.8.8.8:80", "[::1]:80"] {
        assert!(AdminNetwork::new(bind.parse().unwrap(), "127.0.0.1/32").is_err());
    }
    for cidr in [
        "0.0.0.0/0",
        "192.168.1.0/24",
        "127.0.0.1/33",
        "127.0.0.1/7",
        "127.0.0.1/32/32",
    ] {
        assert!(AdminNetwork::new("127.0.0.1:8791".parse().unwrap(), cidr).is_err());
    }
    let guard =
        AdminNetwork::new("192.168.22.219:8788".parse().unwrap(), "192.168.22.0/24").unwrap();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("host", "192.168.22.219:8788".parse().unwrap());
    headers.insert("origin", "http://192.168.22.219:8788".parse().unwrap());
    assert!(guard.trusted("192.168.22.140:4500".parse().unwrap(), &headers));
    assert!(guard.trusted("[::ffff:192.168.22.140]:4500".parse().unwrap(), &headers));
    headers.insert("x-forwarded-for", "192.168.22.140".parse().unwrap());
    assert!(!guard.trusted("203.0.113.5:4500".parse().unwrap(), &headers));
    assert!(!guard.trusted("192.168.23.140:4500".parse().unwrap(), &headers));
    headers.insert("origin", "https://invalid.test".parse().unwrap());
    assert!(!guard.trusted("192.168.22.140:4500".parse().unwrap(), &headers));
    headers.remove("origin");
    headers.insert("host", "invalid.test:8788".parse().unwrap());
    assert!(!guard.trusted("192.168.22.140:4500".parse().unwrap(), &headers));
}
#[tokio::test]
async fn public_listener_hides_administration_and_private_read_only_listener_rejects_mutations() {
    let (_dir, store) = fixture();
    let token = store.admin_token(false).unwrap();
    let app = application(store, false);
    let public = http::router(app.clone());
    let api = private(app);
    for path in [
        "/api/admin/boxes",
        "/api/admin/controls",
        "/api/admin/feedback",
        "/api/admin/announcements",
    ] {
        assert_eq!(
            request(public.clone(), "GET", path, Value::Null, Some(&token))
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            request(api.clone(), "GET", path, Value::Null, None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(api.clone(), "GET", path, Value::Null, Some(&token))
                .await
                .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        request(
            api.clone(),
            "PUT",
            "/api/admin/controls/http-push",
            json!({"enabled":false}),
            Some(&token)
        )
        .await
        .0,
        StatusCode::NOT_IMPLEMENTED
    );
    assert_eq!(
        request(
            public,
            "POST",
            "/api/feedback",
            json!({"text":"Test"}),
            None
        )
        .await
        .0,
        StatusCode::NOT_IMPLEMENTED
    );
    let response = api
        .oneshot(
            Request::builder()
                .uri("/")
                .header("host", "untrusted.test:8791")
                .extension(ConnectInfo(
                    "127.0.0.1:45000".parse::<SocketAddr>().unwrap(),
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(response.headers().contains_key("content-security-policy"));
}
#[tokio::test]
async fn feedback_authentication_validation_paging_and_resolution_remain_private_and_persistent() {
    let (dir, store) = fixture();
    let credentials = store.register(Some("test-box")).unwrap().unwrap();
    let token = credentials["token"].as_str().unwrap();
    let admin_token = store.admin_token(false).unwrap();
    let app = application(store.clone(), true);
    let public = http::router(app.clone());
    let api = private(app);
    for input in [
        Value::Null,
        json!({"text":" \t "}),
        json!({"text":"ok","contact":"x\ny"}),
        json!({"text":"😀".repeat(1001)}),
        json!({"text":"ok","source":"invalid"}),
    ] {
        assert_eq!(
            request(public.clone(), "POST", "/api/feedback", input, None)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        request(
            public.clone(),
            "POST",
            "/api/feedback",
            json!({"id":"test-box","text":"Test"}),
            Some("wrong")
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            public.clone(),
            "POST",
            "/api/feedback",
            json!({"text":"Test"}),
            Some(token)
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, anonymous) = request(
        public.clone(),
        "POST",
        "/api/feedback",
        json!({"text":"\u{feff} Anonymous \u{feff}","contact":"  local  "}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(anonymous.get("text").is_none());
    let (status, owned) = request(
        public.clone(),
        "POST",
        "/api/feedback",
        json!({"id":"test-box","text":"Owned","source":"ai","clientVersion":"0.8.5"}),
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        request(public.clone(), "GET", "/api/feedback", Value::Null, None)
            .await
            .0,
        StatusCode::METHOD_NOT_ALLOWED
    );
    let (_, page) = request(
        api.clone(),
        "GET",
        "/api/admin/feedback",
        Value::Null,
        Some(&admin_token),
    )
    .await;
    assert_eq!(page["feedback"][0]["text"], "Anonymous");
    assert_eq!(page["feedback"][0]["contact"], "local");
    assert_eq!(page["feedback"][1]["boxId"], "test-box");
    assert_eq!(page["nextCursor"], owned["id"]);
    let id = owned["id"].as_u64().unwrap();
    let (status, reply) = request(
        api.clone(),
        "POST",
        &format!("/api/admin/feedback/{id}/resolve"),
        Value::Null,
        Some(&admin_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["resolved"], true);
    let first = store.list_feedback(0).unwrap()["feedback"][1].clone();
    assert_eq!(
        request(
            api.clone(),
            "POST",
            &format!("/api/admin/feedback/{id}/resolve"),
            Value::Null,
            Some(&admin_token)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(store.list_feedback(0).unwrap()["feedback"][1], first);
    assert_eq!(
        request(
            api.clone(),
            "POST",
            "/api/admin/feedback/999/resolve",
            Value::Null,
            Some(&admin_token)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            api,
            "GET",
            "/api/admin/feedback?after=-1",
            Value::Null,
            Some(&admin_token)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let restored = Store::existing(dir.path().join("data.sqlite")).unwrap();
    assert_eq!(restored.list_feedback(0).unwrap()["feedback"][1], first);
    assert!(restored.admin_authenticated(&admin_token).unwrap());
}
#[test]
fn feedback_rate_limits_apply_per_box_and_shared_anonymous_bucket_with_exact_minute_boundary() {
    let (_dir, store) = fixture();
    let now = now_ms();
    for _ in 0..FEEDBACK_BOX_LIMIT {
        store
            .submit_feedback(&json!({"text":"Test"}), Some("test-box"), now)
            .unwrap();
    }
    assert_eq!(
        status(
            store
                .submit_feedback(
                    &json!({"text":"Test"}),
                    Some("test-box"),
                    now + FEEDBACK_RATE_MS - 1
                )
                .unwrap_err()
        ),
        429
    );
    assert!(
        store
            .submit_feedback(&json!({"text":"Other"}), Some("other-box"), now)
            .is_ok()
    );
    for _ in 0..FEEDBACK_ANONYMOUS_LIMIT {
        store
            .submit_feedback(&json!({"text":"Test"}), None, now)
            .unwrap();
    }
    assert_eq!(
        status(
            store
                .submit_feedback(&json!({"text":"Test"}), None, now)
                .unwrap_err()
        ),
        429
    );
    assert!(
        store
            .submit_feedback(
                &json!({"text":"After minute"}),
                Some("test-box"),
                now + FEEDBACK_RATE_MS
            )
            .is_ok()
    );
    assert!(
        store
            .submit_feedback(
                &json!({"text":"After minute"}),
                None,
                now + FEEDBACK_RATE_MS
            )
            .is_ok()
    );
}
#[tokio::test]
async fn announcement_crud_targets_public_cursors_and_protocol_notices_share_ids_without_lost_updates()
 {
    let (_dir, store) = fixture();
    store.register(Some("test-box")).unwrap();
    let admin_token = store.admin_token(false).unwrap();
    let app = application(store.clone(), true);
    let public = http::router(app.clone());
    let api = private(app);
    let (status, notice) = request(
        api.clone(),
        "POST",
        "/api/admin/announcements",
        json!({"text":"Targeted","targets":["test-box"]}),
        Some(&admin_token),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(notice["id"], 1);
    assert_eq!(
        request(
            api.clone(),
            "POST",
            "/api/admin/announcements",
            json!({"text":"Unknown","targets":["unknown-box"]}),
            Some(&admin_token)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (_, broadcast) = request(
        api.clone(),
        "POST",
        "/api/admin/announcements",
        json!({"text":"Broadcast"}),
        Some(&admin_token),
    )
    .await;
    assert_eq!(broadcast["id"], 2);
    store
        .update_control(false, &json!({"version":"0.7.9"}), now_ms())
        .unwrap();
    let gate = store
        .gate("test-box", &json!("0.7.3"), "websocket", now_ms())
        .unwrap();
    let id = gate.notice.unwrap()["id"].as_u64().unwrap();
    assert_eq!(id, 3);
    let (_, last) = request(
        api.clone(),
        "POST",
        "/api/admin/announcements",
        json!({"text":"After protocol"}),
        Some(&admin_token),
    )
    .await;
    assert_eq!(last["id"], 4);
    let (_, page) = request(
        public.clone(),
        "GET",
        "/api/announcements/public?after=2",
        Value::Null,
        None,
    )
    .await;
    assert_eq!(page["announcements"].as_array().unwrap().len(), 1);
    assert_eq!(page["announcements"][0]["id"], 4);
    assert_eq!(page["nextCursor"], 4);
    let (_, page) = request(
        public.clone(),
        "GET",
        "/api/announcements/public?view=latest&after=invalid",
        Value::Null,
        None,
    )
    .await;
    assert_eq!(page["announcements"].as_array().unwrap().len(), 2);
    assert!(
        page["announcements"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["targets"] == "all")
    );
    let (_, listed) = request(
        api.clone(),
        "GET",
        "/api/admin/announcements",
        Value::Null,
        Some(&admin_token),
    )
    .await;
    assert_eq!(listed["announcements"].as_array().unwrap().len(), 3);
    let (status, reply) = request(
        api.clone(),
        "DELETE",
        "/api/admin/announcements/1",
        Value::Null,
        Some(&admin_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["removed"], true);
    assert_eq!(
        request(
            api,
            "DELETE",
            "/api/admin/announcements/1",
            Value::Null,
            Some(&admin_token)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn control_updates_validate_and_persist_deadlines_and_cancel_at_effective_state() {
    let (dir, store) = fixture();
    let token = store.admin_token(false).unwrap();
    let api = private(application(store.clone(), true));
    let now = now_ms();
    let future = iso(now + 60_000);
    for body in [
        json!({"enabled":true,"extra":1}),
        json!({"enabled":"true"}),
        json!({"enabled":true,"at":iso(now-1)}),
        json!({"cancel":true,"enabled":false}),
    ] {
        assert_eq!(
            request(
                api.clone(),
                "PUT",
                "/api/admin/controls/http-push",
                body,
                Some(&token)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for version in [
        json!("0.7.9-beta"),
        json!("00.7.9"),
        json!("0.7.9+build"),
        json!(false),
    ] {
        assert_eq!(
            request(
                api.clone(),
                "PUT",
                "/api/admin/controls/minimum-client-version",
                json!({"version":version}),
                Some(&token)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let (status, controls) = request(
        api.clone(),
        "PUT",
        "/api/admin/controls/http-push",
        json!({"enabled":false,"at":future}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(controls["httpPush"]["enabled"], true);
    assert_eq!(controls["httpPush"]["scheduledChange"]["enabled"], false);
    let (status, controls) = request(
        api.clone(),
        "PUT",
        "/api/admin/controls/minimum-client-version",
        json!({"version":"0.7.9","at":future}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        controls["minimumClientVersion"]["scheduledChange"]["version"],
        "0.7.9"
    );
    let restored = Store::existing(dir.path().join("data.sqlite")).unwrap();
    let effective = restored.controls(now + 60_001).unwrap();
    assert_eq!(effective["httpPush"]["enabled"], false);
    assert_eq!(effective["minimumClientVersion"]["version"], "0.7.9");
    let cancelled = restored
        .update_control(true, &json!({"cancel":true}), now + 60_001)
        .unwrap();
    assert_eq!(cancelled["httpPush"]["enabled"], false);
    assert!(cancelled["httpPush"]["scheduledChange"].is_null());
    assert_eq!(
        request(
            api,
            "PUT",
            "/api/admin/controls/minimum-client-version",
            json!({"version":null}),
            Some(&token)
        )
        .await
        .0,
        StatusCode::OK
    );
}
#[test]
fn full_feedback_inbox_only_evicts_oldest_resolved_item_and_preserves_unknown_fields_on_export() {
    let dir = TempDir::new().unwrap();
    let old = dir.path().join("old");
    fs::create_dir(&old).unwrap();
    for name in ["leaderboard.json", "credentials.json"] {
        fs::write(old.join(name), "{}").unwrap();
    }
    let now = now_ms();
    let items:Vec<_>=(1..=2000).map(|id|json!({"id":id,"text":"Synthetic","source":"web","createdAt":iso(now-120000),"futureField":id})).collect();
    fs::write(
        old.join("feedback.json"),
        json!({"nextId":2001,"feedback":items}).to_string(),
    )
    .unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    store.import(LegacySnapshot::read(&old).unwrap()).unwrap();
    assert_eq!(
        status(
            store
                .submit_feedback(&json!({"text":"New"}), None, now)
                .unwrap_err()
        ),
        503
    );
    assert_eq!(
        store.list_feedback(0).unwrap()["feedback"]
            .as_array()
            .unwrap()
            .len(),
        100
    );
    store.resolve_feedback(2, now).unwrap();
    let receipt = store
        .submit_feedback(&json!({"text":"New"}), None, now)
        .unwrap();
    assert_eq!(receipt["id"], 2001);
    let output = dir.path().join("export");
    store.export(&output).unwrap();
    let exported: Value =
        serde_json::from_slice(&fs::read(output.join("feedback.json")).unwrap()).unwrap();
    assert_eq!(exported["feedback"].as_array().unwrap().len(), 2000);
    assert_eq!(exported["feedback"][0]["futureField"], 1);
    assert_eq!(exported["feedback"][1]["id"], 3);
    assert_eq!(exported["nextId"], 2002);
    LegacySnapshot::read(&output).unwrap();
}
#[tokio::test]
async fn failed_admin_transaction_returns_no_success_and_preserves_shared_announcement_cursor() {
    let (dir, store) = fixture();
    let token = store.admin_token(false).unwrap();
    store
        .publish_announcement(&json!({"text":"First"}), now_ms())
        .unwrap();
    let connection = rusqlite::Connection::open(dir.path().join("data.sqlite")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_notice BEFORE UPDATE ON legacy_documents WHEN NEW.name='announcements.json' BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    let api = private(application(store.clone(), true));
    let (status, error) = request(
        api.clone(),
        "POST",
        "/api/admin/announcements",
        json!({"text":"Failed"}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(error["error"], "Database operation failed");
    assert_eq!(
        store.list_announcements().unwrap()["announcements"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    connection
        .execute_batch("DROP TRIGGER reject_notice")
        .unwrap();
    let (_, notice) = request(
        api,
        "POST",
        "/api/admin/announcements",
        json!({"text":"Recovered"}),
        Some(&token),
    )
    .await;
    assert_eq!(notice["id"], 2);
}

async fn network_request(
    address: SocketAddr,
    path: &str,
    token: Option<&str>,
    extra: &str,
) -> (u16, String, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let authorization = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let raw = format!(
        "GET {path} HTTP/1.1\r\nHost: {address}\r\n{authorization}{extra}Connection: close\r\n\r\n"
    );
    stream.write_all(raw.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    let split = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let headers = String::from_utf8(bytes[..split].to_vec()).unwrap();
    let code = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
    (code, headers, bytes[split + 4..].to_vec())
}
#[tokio::test]
async fn real_private_listener_serves_assets_and_reports_live_ws_counts_and_accepted_writes() {
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{Message, client::IntoClientRequest},
    };
    let (_dir, store) = fixture();
    let credentials = store.register(Some("test-box")).unwrap().unwrap();
    let box_token = credentials["token"].as_str().unwrap();
    let admin_token = store.admin_token(false).unwrap();
    let app = application(store.clone(), true);
    let public = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let public_address = public.local_addr().unwrap();
    let public_app = http::router(app.clone());
    let public_task = tokio::spawn(async move {
        axum::serve(public, public_app).await.unwrap();
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let guard = AdminNetwork::new(address, "127.0.0.1/32").unwrap();
    let api = admin::router(app, guard);
    let admin_task = tokio::spawn(async move {
        axum::serve(
            listener,
            api.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    for (path, content_type) in [
        ("/", "text/html"),
        ("/client.js", "text/javascript"),
        ("/assets/icons.svg", "image/svg+xml"),
        ("/assets/MiSans-Regular.woff2", "font/woff2"),
        ("/assets/MiSans-LICENSE.pdf", "application/pdf"),
    ] {
        let (code, headers, bytes) = network_request(address, path, None, "").await;
        assert_eq!(code, 200);
        assert!(headers.to_lowercase().contains(content_type));
        assert!(headers.to_lowercase().contains("content-security-policy:"));
        assert!(!bytes.is_empty());
    }
    assert_eq!(
        network_request(
            address,
            "/api/admin/boxes",
            Some(&admin_token),
            "Origin: http://invalid.test\r\n"
        )
        .await
        .0,
        403
    );
    assert_eq!(
        network_request(public_address, "/api/admin/boxes", Some(&admin_token), "")
            .await
            .0,
        404
    );
    let mut req = format!("ws://{public_address}/api/reports/ws?id=test-box")
        .into_client_request()
        .unwrap();
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {box_token}").parse().unwrap(),
    );
    let (mut ws, _) = connect_async(req).await.unwrap();
    let mut state: Value = serde_json::from_str::<Value>(include_str!("fixtures/reports.json"))
        .unwrap()["land"]
        .clone();
    state["updatedAtMs"] = json!(now_ms());
    ws.send(Message::Text(json!({"kind":"full","protocol":1,"seq":1,"phase":"day","clientVersion":"0.8.5","state":state}).to_string().into())).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(text) = reply else {
        panic!("Expected ACK")
    };
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["kind"], "ack");
    let (code, _, bytes) =
        network_request(address, "/api/admin/boxes", Some(&admin_token), "").await;
    assert_eq!(code, 200);
    let snapshot: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(snapshot["boxes"][0]["wsConnections"], 1);
    assert_eq!(snapshot["boxes"][0]["registered"], true);
    assert_eq!(snapshot["boxes"][0]["clientVersion"], "0.8.5");
    assert_eq!(snapshot["acceptedReports"]["websocket"], 1);
    assert!(snapshot["uptimeSec"].as_f64().unwrap() > 0.0);
    #[cfg(target_os = "linux")]
    assert!(snapshot["memoryBytes"].as_u64().unwrap() > 0);
    ws.close(None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let (_, _, bytes) =
                network_request(address, "/api/admin/boxes", Some(&admin_token), "").await;
            let snapshot: Value = serde_json::from_slice(&bytes).unwrap();
            if snapshot["boxes"][0]["wsConnections"] == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    public_task.abort();
    admin_task.abort();
    let _ = public_task.await;
    let _ = admin_task.await;
}
#[test]
fn concurrent_admin_announcements_and_protocol_notices_allocate_unique_persisted_ids() {
    use std::{
        sync::{Arc, Barrier},
        thread,
    };
    let (_dir, store) = fixture();
    let now = now_ms();
    store
        .update_control(false, &json!({"version":"0.7.9"}), now)
        .unwrap();
    let barrier = Arc::new(Barrier::new(12));
    let workers: Vec<_> = (0..12)
        .map(|i| {
            let store = store.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                if i % 2 == 0 {
                    store
                        .publish_announcement(&json!({"text":format!("Update {i}")}), now)
                        .unwrap()["id"]
                        .as_u64()
                        .unwrap()
                } else {
                    store
                        .gate(&format!("box-{i}"), &json!("0.7.3"), "websocket", now)
                        .unwrap()
                        .notice
                        .unwrap()["id"]
                        .as_u64()
                        .unwrap()
                }
            })
        })
        .collect();
    let ids: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    let notice_ids: std::collections::HashSet<_> = ids.iter().skip(1).step_by(2).collect();
    assert_eq!(notice_ids.len(), 1);
    let operator_ids: std::collections::HashSet<_> = ids.iter().step_by(2).collect();
    assert_eq!(operator_ids.len(), 6);
    assert!(operator_ids.is_disjoint(&notice_ids));
    assert_eq!(
        store.list_announcements().unwrap()["announcements"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
}
