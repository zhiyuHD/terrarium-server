use crate::{
    http::App,
    ingestion::{Ingestion, ReportSession, database, now_ms},
    protocol::{MAX_BYTES, valid_id},
};
use axum::{
    Json,
    extract::{
        Query, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, mpsc};
pub fn bearer(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .strip_prefix("Bearer ")
        .unwrap_or("")
        .to_owned()
}
pub async fn upgrade(
    State(app): State<App>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(ingestion) = app.ingestion else {
        return StatusCode::NOT_IMPLEMENTED.into_response();
    };
    let id = query.get("id").cloned().unwrap_or_default();
    let auth_id = id.clone();
    let token = bearer(&headers);
    match database(app.store.clone(), move |s| {
        if valid_id(&auth_id) {
            s.authenticated(&auth_id, &token)
        } else {
            Ok(false)
        }
    })
    .await
    {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error":"Unauthorized"})),
            )
                .into_response();
        }
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error.body())).into_response();
        }
    }
    let header = headers
        .get("x-cortico-client-version")
        .map(|v| Value::String(v.to_str().unwrap_or("").to_owned()));
    let old_id = id.clone();
    let version = match header {
        Some(v) => v,
        None => match database(app.store.clone(), move |s| s.get(&old_id)).await {
            Ok(Some(entry)) => entry["clientVersion"].clone(),
            _ => Value::Null,
        },
    };
    let opt_in = headers
        .get("x-cortico-protocol-notices")
        .is_some_and(|v| v == "1");
    ws.max_message_size(MAX_BYTES)
        .max_frame_size(MAX_BYTES)
        .on_upgrade(move |socket| connection(socket, ingestion, id, version, opt_in))
}
async fn connection(
    socket: WebSocket,
    ingestion: Ingestion,
    id: String,
    version: Value,
    opt_in: bool,
) {
    let _connection = ingestion.metrics().connected(&id);
    let (mut sink, mut stream) = socket.split();
    let gate = match ingestion.gate(&id, &version, "websocket").await {
        Ok(gate) => gate,
        Err(_) => return,
    };
    if opt_in
        && let Some(notice) = gate.notice
        && sink
            .send(Message::Text(
                json!({"kind":"notice","notice":notice}).to_string().into(),
            ))
            .await
            .is_err()
    {
        return;
    }
    let permits = Arc::new(Semaphore::new(8));
    let (reports, mut incoming) = mpsc::channel(8);
    let (controls, mut commands) = mpsc::channel(8);
    let alive = Arc::new(AtomicBool::new(true));
    let reader_alive = alive.clone();
    let reader = tokio::spawn(async move {
        while let Some(Ok(message)) = stream.next().await {
            match message {
                Message::Text(_) | Message::Binary(_) => {
                    let Ok(permit) = permits.clone().try_acquire_owned() else {
                        let _ = controls
                            .send(Message::Close(Some(CloseFrame {
                                code: 1008,
                                reason: "Too many pending reports".into(),
                            })))
                            .await;
                        return;
                    };
                    if reports.send((message, permit)).await.is_err() {
                        return;
                    }
                }
                Message::Pong(_) => {
                    reader_alive.store(true, Ordering::Relaxed);
                }
                Message::Ping(data) => {
                    if controls.try_send(Message::Pong(data)).is_err() {
                        break;
                    }
                }
                Message::Close(frame) => {
                    let _ = controls.send(Message::Close(frame)).await;
                    return;
                }
            }
        }
        let _ = controls.send(Message::Close(None)).await;
    });
    let mut session = ReportSession::new();
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(30),
        Duration::from_secs(30),
    );
    loop {
        tokio::select! {
            biased;
            command=commands.recv()=>{
                let Some(command)=command else{break};let close=matches!(command,Message::Close(_));
                if sink.send(command).await.is_err()||close {break;}
            },
            _=heartbeat.tick()=>{
                if !alive.swap(false,Ordering::Relaxed)||sink.send(Message::Ping(Vec::new().into())).await.is_err(){break;}
            },
            report=incoming.recv()=>{
                let Some((message,_permit))=report else{break};
                let text=match &message {Message::Text(t)=>Some(t.as_str()),_=>None};
                let reply=session.receive(&ingestion,&id,text).await;
                if sink.send(Message::Text(reply.to_string().into())).await.is_err(){break;}
            }
        }
    }
    reader.abort();
    let _ = reader.await;
}
// Announcement polling uses the last observed transport, so HTTP deprecation notices survive WS restarts.
pub async fn announcements(
    State(app): State<App>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let Some(ingestion) = app.ingestion else {
        return StatusCode::NOT_IMPLEMENTED.into_response();
    };
    let id = query.get("id").cloned().unwrap_or_default();
    let token = bearer(&headers);
    let auth_id = id.clone();
    match database(app.store.clone(), move |s| {
        s.authenticated(&auth_id, &token)
    })
    .await
    {
        Ok(true) => {}
        Ok(false) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(e.body())).into_response(),
    }
    let after = match query.get("after").map(|s| s.parse::<u64>()).transpose() {
        Ok(n) if n.unwrap_or(0) <= 9_007_199_254_740_991 => n.unwrap_or(0),
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    let client_id = id.clone();
    let client = database(app.store.clone(), move |s| s.observed_client(&client_id)).await;
    let client = match client {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(e.body())).into_response(),
    };
    let version = client["version"].clone();
    let transport = client["transport"].as_str().unwrap_or("http");
    let gate = match ingestion.gate(&id, &version, transport).await {
        Ok(g) => g,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(e.body())).into_response(),
    };
    match database(app.store, move |s| {
        s.announcement_page(Some(&id), after, false, gate.notice, now_ms())
    })
    .await
    {
        Ok(page) => Json(page).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(e.body())).into_response(),
    }
}
pub async fn public_announcements(
    State(app): State<App>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let latest = query.get("view").is_some_and(|s| s == "latest");
    let after = if latest {
        0
    } else {
        match query.get("after").map(|v| v.parse::<u64>()).transpose() {
            Ok(n) if n.unwrap_or(0) <= 9_007_199_254_740_991 => n.unwrap_or(0),
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error":"Invalid announcement cursor"})),
                )
                    .into_response();
            }
        }
    };
    match database(app.store, move |s| {
        s.announcement_page(None, after, latest, None, now_ms())
    })
    .await
    {
        Ok(page) => Json(page).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(e.body())).into_response(),
    }
}
