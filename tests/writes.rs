use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{fs, time::Duration};
use tempfile::TempDir;
use terrarium_server::{
    http::{App, router},
    ingestion::{Ingestion, now_ms},
    policy::iso,
    protocol::{MAX_BYTES, state_hash},
    store::{LegacySnapshot, Store},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use tower::ServiceExt;
fn fixture(kind: &str) -> Value {
    let mut v: Value =
        serde_json::from_str::<Value>(include_str!("fixtures/reports.json")).unwrap()[kind].clone();
    v["updatedAtMs"] = json!(now_ms());
    v
}
fn app(store: Store) -> Router {
    router(App {
        ingestion: Some(Ingestion::new(store.clone())),
        store,
        release_id: None,
    })
}
async fn http(
    app: Router,
    method: &str,
    path: &str,
    body: Value,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let response = app
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
#[tokio::test]
async fn http_registration_authentication_full_reports_bounds_and_restart_export_preserve_credentials()
 {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("data.sqlite");
    let store = Store::open(&path).unwrap();
    let api = app(store.clone());
    for body in [
        json!({"id":"https://invalid.test"}),
        json!({"kind":"invalid"}),
        json!({"id":"aa","unknown":1}),
        Value::Null,
    ] {
        assert_eq!(
            http(api.clone(), "POST", "/api/registrations", body, None)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let (status, credentials) = http(
        api.clone(),
        "POST",
        "/api/registrations",
        json!({"kind":"water"}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = credentials["id"].as_str().unwrap();
    let token = credentials["token"].as_str().unwrap();
    assert_eq!(token.len(), 64);
    assert_eq!(
        http(
            api.clone(),
            "POST",
            "/api/registrations",
            json!({"id":id}),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let state = fixture("water");
    let input = json!({"id":id,"state":state,"clientVersion":"0.8.5","phase":"day"});
    assert_eq!(
        http(
            api.clone(),
            "PUT",
            "/api/reports",
            input.clone(),
            Some("incorrect")
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(store.get(id).unwrap().is_none());
    assert_eq!(
        http(
            api.clone(),
            "PUT",
            "/api/reports",
            input.clone(),
            Some(token)
        )
        .await
        .0,
        StatusCode::OK
    );
    let first = store.get(id).unwrap().unwrap();
    let mut invalid = input.clone();
    invalid["state"]["fish"] = json!([{}]);
    assert_eq!(
        http(api.clone(), "PUT", "/api/reports", invalid, Some(token))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let mut stale = input.clone();
    stale["state"]["updatedAtMs"] = json!(state["updatedAtMs"].as_i64().unwrap() - 1);
    assert_eq!(
        http(api.clone(), "PUT", "/api/reports", stale, Some(token))
            .await
            .0,
        StatusCode::CONFLICT
    );
    for timestamp in [now_ms() + 120_000, now_ms() - 32 * 86_400_000] {
        let mut invalid = input.clone();
        invalid["state"]["updatedAtMs"] = json!(timestamp);
        assert_eq!(
            http(api.clone(), "PUT", "/api/reports", invalid, Some(token))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let invalid = json!({"id":id,"state":fixture("land")});
    assert_eq!(
        http(api.clone(), "PUT", "/api/reports", invalid, Some(token))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(store.get(id).unwrap().unwrap(), first);
    let oversized = api
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/reports")
                .body(Body::from(vec![b' '; MAX_BYTES + 1]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let oversized = api
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/registrations")
                .body(Body::from(vec![b' '; 1025]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    drop(api);
    drop(store);
    let restored = Store::existing(&path).unwrap();
    assert!(restored.authenticated(id, token).unwrap());
    assert_eq!(restored.get(id).unwrap().unwrap(), first);
    let output = dir.path().join("export");
    restored.export(&output).unwrap();
    let saved: Value =
        serde_json::from_slice(&fs::read(output.join("credentials.json")).unwrap()).unwrap();
    assert_eq!(saved[id], token);
    assert_eq!(LegacySnapshot::read(&output).unwrap().counts().0, 1);
    let backup = dir.path().join("backup.sqlite");
    restored.backup(&backup).unwrap();
    let copy = Store::existing(&backup).unwrap();
    copy.check().unwrap();
    assert!(copy.authenticated(id, token).unwrap());
    assert_eq!(copy.get(id).unwrap(), restored.get(id).unwrap());
}
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn send(ws: &mut Socket, value: Value) -> Value {
    ws.send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
    receive(ws).await
}
async fn receive(ws: &mut Socket) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}
fn full(seq: u64, state: &Value) -> Value {
    json!({"protocol":1,"kind":"full","seq":seq,"phase":"day","clientVersion":"0.8.5","state":state,"stateHash":state_hash(state)})
}
async fn listen(api: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, api).await.unwrap();
    });
    (format!("ws://{address}"), task)
}
async fn connect(
    url: &str,
    id: &str,
    token: &str,
    version: &str,
) -> Result<Socket, tokio_tungstenite::tungstenite::Error> {
    let mut req = format!("{url}/api/reports/ws?id={id}")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    req.headers_mut()
        .insert("x-cortico-client-version", version.parse().unwrap());
    req.headers_mut()
        .insert("x-cortico-protocol-notices", "1".parse().unwrap());
    connect_async(req).await.map(|(s, _)| s)
}
#[tokio::test]
async fn websocket_full_delta_checksum_recovery_sequence_and_reconnect_never_commit_rejected_reports()
 {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    let credentials = store.register(Some("test-box")).unwrap().unwrap();
    let token = credentials["token"].as_str().unwrap();
    let (url, server) = listen(app(store.clone())).await;
    let error = connect(&url, "test-box", "wrong", "0.8.5")
        .await
        .unwrap_err();
    assert!(
        matches!(error,tokio_tungstenite::tungstenite::Error::Http(response) if response.status()==StatusCode::UNAUTHORIZED)
    );
    let mut ws = connect(&url, "test-box", token, "0.8.5").await.unwrap();
    let state = fixture("land");
    let initial_delta = json!({"protocol":1,"kind":"delta","seq":1,"phase":"day","clientVersion":"0.8.5","baseRevision":"missing","changes":[]});
    assert_eq!(
        send(&mut ws, initial_delta.clone()).await,
        json!({"kind":"resync","seq":1})
    );
    assert!(store.get("test-box").unwrap().is_none());
    let ack = send(&mut ws, full(2, &state)).await;
    assert_eq!(ack["kind"], "ack");
    assert_eq!(ack["stateHash"], state_hash(&state));
    assert_eq!(
        store.get("test-box").unwrap().unwrap()["reportRevision"],
        ack["revision"]
    );
    let mut changed = state.clone();
    changed["name"] = json!("Another garden");
    let delta = json!({"protocol":1,"kind":"delta","seq":3,"phase":"night","clientVersion":"0.8.5","baseRevision":ack["revision"],"baseHash":state_hash(&state),"stateHash":state_hash(&changed),"changes":[{"op":"add","path":"/name","value":"Another garden"}]});
    let ack = send(&mut ws, delta.clone()).await;
    assert_eq!(ack["kind"], "ack");
    assert_eq!(
        store.get("test-box").unwrap().unwrap()["state"]["name"],
        changed["name"]
    );
    let accepted = store.get("test-box").unwrap().unwrap();
    let mut corrupt = delta.clone();
    corrupt["seq"] = json!(4);
    corrupt["baseRevision"] = ack["revision"].clone();
    corrupt["baseHash"] = json!("0".repeat(64));
    assert_eq!(send(&mut ws, corrupt).await["kind"], "resync");
    assert_eq!(store.get("test-box").unwrap().unwrap(), accepted);
    let mut correct = delta.clone();
    correct["seq"] = json!(5);
    correct["baseRevision"] = ack["revision"].clone();
    correct["baseHash"] = json!(state_hash(&changed));
    correct["changes"] = json!([]);
    assert_eq!(send(&mut ws, correct).await["kind"], "resync");
    assert_eq!(send(&mut ws, full(6, &changed)).await["kind"], "ack");
    let mut missing = full(7, &changed);
    missing.as_object_mut().unwrap().remove("stateHash");
    assert_eq!(send(&mut ws, missing).await["kind"], "resync");
    let accepted = store.get("test-box").unwrap().unwrap();
    let mut corrupt = full(8, &changed);
    corrupt["stateHash"] = json!("0".repeat(64));
    assert_eq!(send(&mut ws, corrupt).await["kind"], "resync");
    assert_eq!(store.get("test-box").unwrap().unwrap(), accepted);
    let mut invalid = full(9, &changed);
    invalid["state"]["plants"] = json!([{}]);
    assert_eq!(send(&mut ws, invalid).await["status"], 400);
    assert_eq!(store.get("test-box").unwrap().unwrap(), accepted);
    assert_eq!(send(&mut ws, full(10, &changed)).await["kind"], "ack");
    let duplicate = send(&mut ws, full(10, &changed)).await;
    assert_eq!(duplicate["status"], 400);
    assert_eq!(duplicate["seq"], 0);
    ws.send(Message::Binary(vec![1, 2].into())).await.unwrap();
    assert_eq!(receive(&mut ws).await["status"], 400);
    let mut invalid = full(11, &changed);
    invalid.as_object_mut().unwrap().remove("clientVersion");
    assert_eq!(send(&mut ws, invalid).await["status"], 400);
    ws.close(None).await.unwrap();
    let mut ws = connect(&url, "test-box", token, "0.8.5").await.unwrap();
    let mut delta = initial_delta;
    delta["baseRevision"] = store.get("test-box").unwrap().unwrap()["reportRevision"].clone();
    assert_eq!(send(&mut ws, delta).await["kind"], "resync");
    assert_eq!(send(&mut ws, full(2, &changed)).await["kind"], "ack");
    ws.close(None).await.unwrap();
    server.abort();
    let _ = server.await;
}
#[tokio::test]
async fn unsupported_clients_receive_private_protocol_notices_and_cannot_report_over_http_or_ws() {
    let dir = TempDir::new().unwrap();
    let old = dir.path().join("old");
    fs::create_dir(&old).unwrap();
    for (name, value) in [
        ("leaderboard.json", json!({})),
        ("credentials.json", json!({})),
        (
            "controls.json",
            json!({"httpPush":{"enabled":false,"scheduledChange":null},"minimumClientVersion":{"version":"0.7.9","scheduledChange":null}}),
        ),
    ] {
        fs::write(old.join(name), value.to_string()).unwrap();
    }
    let store = Store::open(dir.path().join("db.sqlite")).unwrap();
    store.import(LegacySnapshot::read(&old).unwrap()).unwrap();
    let credentials = store.register(Some("test-box")).unwrap().unwrap();
    let token = credentials["token"].as_str().unwrap();
    let api = app(store.clone());
    let body = json!({"id":"test-box","state":fixture("land"),"clientVersion":"0.7.3"});
    let (status, reply) = http(api.clone(), "PUT", "/api/reports", body, Some(token)).await;
    assert_eq!(status, StatusCode::UPGRADE_REQUIRED);
    assert_eq!(reply["accepted"], false);
    assert_eq!(reply["ignored"], true);
    assert_eq!(reply["upgradeRequired"], true);
    assert!(store.get("test-box").unwrap().is_none());
    let (status, reply) = http(
        api.clone(),
        "PUT",
        "/api/reports",
        json!({"id":"test-box","state":fixture("land"),"clientVersion":"0.8.5"}),
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(reply["accepted"], false);
    let (url, server) = listen(api.clone()).await;
    let mut ws = connect(&url, "test-box", token, "0.7.3").await.unwrap();
    let notice = receive(&mut ws).await;
    assert_eq!(notice["kind"], "notice");
    assert_eq!(notice["notice"]["targets"], json!(["test-box"]));
    let original_id = notice["notice"]["id"].clone();
    let mut report = full(1, &fixture("land"));
    report["clientVersion"] = json!("0.7.3");
    let reply = send(&mut ws, report).await;
    assert_eq!(reply["status"], 426);
    assert_eq!(reply["ignored"], true);
    assert!(store.get("test-box").unwrap().is_none());
    let (_, page) = http(
        api.clone(),
        "GET",
        "/api/announcements?id=test-box",
        Value::Null,
        Some(token),
    )
    .await;
    assert_eq!(page["announcements"][0]["id"], original_id);
    let (_, public) = http(
        api.clone(),
        "GET",
        "/api/announcements/public",
        Value::Null,
        None,
    )
    .await;
    assert!(public["announcements"].as_array().unwrap().is_empty());
    ws.close(None).await.unwrap();
    server.abort();
    let _ = server.await;
    drop(api);
    let restored = Store::existing(dir.path().join("db.sqlite")).unwrap();
    let notice = restored
        .gate("test-box", &json!("0.7.3"), "websocket", now_ms())
        .unwrap()
        .notice
        .unwrap();
    assert_eq!(notice["id"], original_id);
    let output = dir.path().join("export");
    restored.export(&output).unwrap();
    let announcements: Value =
        serde_json::from_slice(&fs::read(output.join("announcements.json")).unwrap()).unwrap();
    assert_eq!(announcements["announcements"], json!([]));
    assert!(announcements["nextId"].as_u64().unwrap() > original_id.as_u64().unwrap());
}
#[test]
fn scheduled_policy_and_announcement_paging_survive_restart_with_shared_notice_identity() {
    let dir = TempDir::new().unwrap();
    let old = dir.path().join("old");
    fs::create_dir(&old).unwrap();
    let now = now_ms();
    let notices:Vec<_>=(1..=22).map(|id|json!({"id":id,"title":"Update","text":"Synthetic announcement","targets":if id==1{json!(["other-box"])}else{json!("all")},"createdAt":iso(now-1000),"expiresAt":iso(now+86400000)})).collect();
    for (name, value) in [
        ("leaderboard.json", json!({})),
        ("credentials.json", json!({})),
        (
            "announcements.json",
            json!({"nextId":23,"announcements":notices}),
        ),
        (
            "controls.json",
            json!({"httpPush":{"enabled":true,"scheduledChange":{"enabled":false,"at":iso(now+1000)}},"minimumClientVersion":{"version":null,"scheduledChange":{"version":"0.7.9","at":iso(now+1000)}}}),
        ),
    ] {
        fs::write(old.join(name), value.to_string()).unwrap();
    }
    let store = Store::open(dir.path().join("db.sqlite")).unwrap();
    store.import(LegacySnapshot::read(&old).unwrap()).unwrap();
    let warning = store
        .gate("test-box", &json!("0.7.3"), "http", now)
        .unwrap();
    assert!(warning.rejection.is_none());
    assert!(
        warning.notice.as_ref().unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("升级预告")
    );
    let page = store
        .announcement_page(Some("test-box"), 0, false, warning.notice.clone(), now)
        .unwrap();
    assert_eq!(page["announcements"].as_array().unwrap().len(), 20);
    assert_eq!(page["nextCursor"], 21);
    let page = store
        .announcement_page(Some("test-box"), 21, false, warning.notice, now)
        .unwrap();
    assert_eq!(page["announcements"].as_array().unwrap().len(), 2);
    let denied = store
        .gate("test-box", &json!("0.7.3"), "http", now + 1001)
        .unwrap();
    assert_eq!(denied.rejection, Some(426));
    let shared = store
        .gate("other-box", &json!("0.7.3"), "http", now + 1001)
        .unwrap();
    assert_eq!(
        denied.notice.as_ref().unwrap()["id"],
        shared.notice.as_ref().unwrap()["id"]
    );
    drop(store);
    let store = Store::existing(dir.path().join("db.sqlite")).unwrap();
    let restored = store
        .gate("test-box", &json!("0.7.3"), "http", now + 1002)
        .unwrap();
    assert_eq!(restored.notice, denied.notice);
    assert_eq!(
        store
            .gate("current-box", &json!("0.8.5"), "websocket", now + 1002)
            .unwrap()
            .rejection,
        None
    );
}

#[tokio::test]
async fn sqlite_write_failure_returns_error_and_next_full_report_can_recover() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db.sqlite");
    let store = Store::open(&path).unwrap();
    store.register(Some("test-box")).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_report BEFORE INSERT ON boxes BEGIN SELECT RAISE(ABORT,'fixture write failure'); END;").unwrap();
    let ingestion = Ingestion::new(store.clone());
    let mut session = terrarium_server::ingestion::ReportSession::new();
    let state = fixture("land");
    let reply = session
        .receive(&ingestion, "test-box", Some(&full(1, &state).to_string()))
        .await;
    assert_eq!(reply["kind"], "error");
    assert_eq!(reply["status"], 500);
    assert!(store.get("test-box").unwrap().is_none());
    connection
        .execute_batch("DROP TRIGGER reject_report")
        .unwrap();
    let reply = session
        .receive(&ingestion, "test-box", Some(&full(2, &state).to_string()))
        .await;
    assert_eq!(reply["kind"], "ack");
    assert_eq!(
        store.get("test-box").unwrap().unwrap()["reportRevision"],
        reply["revision"]
    );
}
#[test]
fn rejects_malformed_imported_policy_documents_before_writing_any_database_rows() {
    let dir = TempDir::new().unwrap();
    let old = dir.path().join("old");
    fs::create_dir(&old).unwrap();
    for name in ["leaderboard.json", "credentials.json"] {
        fs::write(old.join(name), "{}").unwrap();
    }
    for (name, bad) in [
        ("controls.json", json!({"httpPush":false})),
        (
            "announcements.json",
            json!({"nextId":1,"announcements":[null]}),
        ),
        ("protocol-notices.json", json!({"test-box":{"id":1}})),
    ] {
        fs::write(old.join(name), bad.to_string()).unwrap();
        assert!(LegacySnapshot::read(&old).is_err());
        fs::remove_file(old.join(name)).unwrap();
    }
}
