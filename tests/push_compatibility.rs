use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tempfile::TempDir;
use terrarium_server::{
    http::{App, router},
    ingestion::{Ingestion, now_ms},
    store::Store,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
#[tokio::test]
async fn idle_authenticated_sockets_do_not_block_other_upgrades_or_require_an_initial_snapshot() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    let credentials = store.register(Some("garden-idle")).unwrap().unwrap();
    let token = credentials["token"].as_str().unwrap();
    let ingestion = Ingestion::new(store.clone());
    ingestion.ready().await.unwrap();
    let app: Router = router(App {
        store,
        ingestion: Some(ingestion),
        release_id: None,
        release_root: None,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut sockets = tokio::time::timeout(Duration::from_secs(5), async {
        let mut sockets = Vec::new();
        for _ in 0..6 {
            let mut req = format!("ws://{addr}/api/reports/ws?id=garden-idle")
                .into_client_request()
                .unwrap();
            req.headers_mut()
                .insert("authorization", format!("Bearer {token}").parse().unwrap());
            req.headers_mut()
                .insert("x-cortico-client-version", "0.9.0".parse().unwrap());
            sockets.push(connect_async(req).await.unwrap().0);
        }
        sockets
    })
    .await
    .expect("Idle peers must not reserve initial-report slots");
    let mut state: Value = serde_json::from_str::<Value>(include_str!("fixtures/reports.json"))
        .unwrap()["land"]
        .clone();
    state["updatedAtMs"] = json!(now_ms());
    sockets[5].send(Message::Text(json!({"protocol":1,"seq":1,"kind":"full","clientVersion":"0.9.0","phase":"day","state":state}).to_string().into())).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), sockets[5].next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(reply,Message::Text(text) if serde_json::from_str::<Value>(&text).unwrap()["kind"]=="ack")
    );
    for mut socket in sockets {
        let _ = socket.close(None).await;
    }
    server.abort();
}
