use axum::{body::Body, http::Request};
use futures_util::{SinkExt, StreamExt};
use tempfile::TempDir;
use terrarium_server::{
    http::{App, router},
    ingestion::{Ingestion, REPORT_BUFFERS},
    store::Store,
};
use tower::ServiceExt;
#[tokio::test]
async fn exhausted_shared_report_buffers_reject_http_and_close_ws_without_writing_then_recover() {
    let dir = TempDir::new().unwrap();
    let s = Store::open(dir.path().join("data.sqlite")).unwrap();
    let credentials = s.register(Some("garden-pressure")).unwrap().unwrap();
    let token = credentials["token"].as_str().unwrap();
    let app = router(App {
        store: s.clone(),
        ingestion: Some(Ingestion::new(s.clone())),
        release_id: None,
        release_root: None,
    });
    let permit = REPORT_BUFFERS
        .clone()
        .acquire_many_owned(24 * 1024 * 1024)
        .await
        .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/reports")
                .method("PUT")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
    let mut request = format!("ws://{addr}/api/reports/ws?id=garden-pressure")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    request
        .headers_mut()
        .insert("x-cortico-client-version", "0.8.7".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request.clone())
        .await
        .unwrap();
    socket.send(Message::Text("{}".into())).await.unwrap();
    let response = socket.next().await.unwrap().unwrap();
    assert!(matches!(response,Message::Close(Some(frame)) if u16::from(frame.code)==1013));
    assert!(s.get("garden-pressure").unwrap().is_none());
    drop(permit);
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket.send(Message::Text("{}".into())).await.unwrap();
    let response = socket.next().await.unwrap().unwrap();
    assert!(matches!(response, Message::Text(_)));
    socket.close(None).await.unwrap();
    task.abort();
}
