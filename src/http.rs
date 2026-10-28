use crate::{
    ingestion::{Ingestion, database, now_ms},
    protocol::{MAX_BYTES, valid_id},
    push,
    store::Store,
};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{any, get, post, put},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};
use tokio::sync::Semaphore;
static LEADERBOARD_READS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(2)));
#[derive(Clone)]
pub struct App {
    pub store: Store,
    pub release_id: Option<String>,
    pub release_root: Option<std::path::PathBuf>,
    pub ingestion: Option<Ingestion>,
}
pub fn router(app: App) -> Router {
    let photos = crate::photo_cache::PhotoCache::new(app.store.photo_dir());
    crate::photos::start_cleanup(&photos, app.store.clone());
    Router::new()
        .route("/", get(crate::presentation::index))
        .route("/support-qr.png", get(crate::support::qr))
        .route("/api/presentation", get(crate::presentation::status))
        .route(
            "/scene.js",
            get(|| async {
                (
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "text/javascript; charset=utf-8",
                    )],
                    include_str!("../web/scene.js"),
                )
            }),
        )
        .route(
            "/forum",
            get(|| async {
                Html(crate::support::page(
                    &include_str!("../web/forum.html").replace(
                        "<!-- FORUM_MARKDOWN -->",
                        concat!(
                            "<script>",
                            include_str!("../web/forum-markdown.js"),
                            "</script>"
                        ),
                    ),
                ))
            }),
        )
        .route("/api/forum/status", get(crate::forum::status))
        .route("/api/forum/{*path}", any(crate::forum::handle))
        .route("/api/neighbors", post(crate::neighbors::handle))
        .route("/api/photos", post(crate::photos::take))
        .route("/api/photos/{key}", get(crate::photos::download))
        .route("/health", get(health))
        .route("/api/leaderboard", get(leaderboard))
        .route("/api/boxes/{id}", get(box_detail))
        .route("/api/feedback", any(crate::admin::feedback))
        .route("/api/admin", any(|| async { StatusCode::NOT_FOUND }))
        .route(
            "/api/admin/{*path}",
            any(|| async { StatusCode::NOT_FOUND }),
        )
        .route("/api/registrations", post(register))
        .route("/api/reports", put(report))
        .route("/api/reports/ws", get(push::upgrade))
        .route("/api/announcements", get(push::announcements))
        .route("/api/announcements/public", get(push::public_announcements))
        .fallback(historical_asset)
        .layer(axum::middleware::map_response(
            |mut response: Response| async move {
                if !response.headers().contains_key("cache-control") {
                    response
                        .headers_mut()
                        .insert("cache-control", "no-store".parse().unwrap());
                }
                response
                    .headers_mut()
                    .insert("x-content-type-options", "nosniff".parse().unwrap());
                response
            },
        ))
        .layer(DefaultBodyLimit::disable())
        .layer(axum::Extension(photos))
        .with_state(app)
}
async fn historical_asset(State(app): State<App>, request: Request) -> Response {
    if request.method() != axum::http::Method::GET {
        return error(StatusCode::NOT_FOUND, "Not found");
    }
    match crate::assets::historical(app.release_root, request.uri().path().to_owned(), false).await
    {
        Ok(Some(asset)) => (
            [
                ("content-type", asset.mime),
                ("cache-control", "public, max-age=31536000, immutable"),
            ],
            asset.bytes,
        )
            .into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "Not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Asset read failed"),
    }
}
async fn health(State(app): State<App>) -> Response {
    let controls = match database(app.store, |s| s.controls(now_ms())).await {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(e.body())).into_response(),
    };
    Json(json!({"ok":true,"version":env!("CARGO_PKG_VERSION"),"releaseId":app.release_id,"implementation":"rust","readOnly":app.ingestion.is_none(),"websocketPush":app.ingestion.is_some(),"httpPushEnabled":app.ingestion.is_some()&&controls["httpPush"]["enabled"]==true,"minimumClientVersion":controls["minimumClientVersion"]["version"],"reportProcessing":app.ingestion.as_ref().map(|ingestion|ingestion.metrics().processing_status())})).into_response()
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error":message}))).into_response()
}
async fn box_detail(
    State(app): State<App>,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    match tokio::task::spawn_blocking(move || app.store.get(&id)).await {
        Ok(Ok(Some(entry))) => {
            let result = if query.get("view").is_some_and(|v| v == "replays") {
                json!({"replays":entry["state"].get("replays").cloned().unwrap_or_else(||json!([]))})
            } else if query.get("view").is_some_and(|v| v == "scene") {
                crate::store::scene_entry(&entry)
            } else {
                entry
            };
            Json(result).into_response()
        }
        Ok(Ok(None)) => error(StatusCode::NOT_FOUND, "Unknown box"),
        _ => error(StatusCode::INTERNAL_SERVER_ERROR, "Read failed"),
    }
}
async fn leaderboard(
    State(app): State<App>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let kind = query.get("kind").cloned();
    if kind.as_ref().is_some_and(|v| v != "land" && v != "water") {
        return error(StatusCode::BAD_REQUEST, "Invalid box kind");
    }
    let permit = LEADERBOARD_READS.clone().acquire_owned().await.unwrap();
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let _permit = permit;
        let value = (|| -> anyhow::Result<Value> {
            if query.get("view").is_some_and(|v| v == "coins") {
                anyhow::ensure!(
                    kind.as_deref() == Some("land"),
                    "Coin ranking requires land"
                );
                let limit = query
                    .get("limit")
                    .map(|v| v.parse::<usize>())
                    .transpose()?
                    .unwrap_or(20);
                anyhow::ensure!((1..=100).contains(&limit), "Invalid coin ranking limit");
                return app
                    .store
                    .coins(limit, query.get("ownId").map(String::as_str));
            }
            if query.get("view").is_some_and(|v| v == "gallery") {
                let kind = kind
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Gallery requires a box kind"))?;
                let number = |key: &str, default: usize| -> anyhow::Result<usize> {
                    Ok(query
                        .get(key)
                        .map(|v| v.parse())
                        .transpose()?
                        .unwrap_or(default))
                };
                let page = number("page", 1)?;
                let size = number("pageSize", 4)?;
                let sort = query.get("sort").map(String::as_str).unwrap_or("score");
                let search = query.get("q").map(String::as_str).unwrap_or("").trim();
                anyhow::ensure!(
                    page > 0
                        && page <= 9_007_199_254_740_991
                        && (1..=12).contains(&size)
                        && matches!(sort, "score" | "recent")
                        && search.encode_utf16().count() <= 100,
                    "Invalid gallery pagination, sort or search"
                );
                return app
                    .store
                    .gallery(kind, search, sort == "recent", page, size);
            }
            if query.get("view").is_some_and(|v| v == "summary") {
                let kind = kind.ok_or_else(|| anyhow::anyhow!("Summary requires kind"))?;
                let limit = query
                    .get("limit")
                    .map(|v| v.parse::<usize>())
                    .transpose()?
                    .unwrap_or(20);
                anyhow::ensure!((1..=100).contains(&limit), "Invalid summary limit");
                return app
                    .store
                    .summary(&kind, limit, query.get("ownId").map(String::as_str));
            }
            Ok(Value::Array(app.store.ranked(kind.as_deref())?))
        })()?;
        Ok(serde_json::to_vec(&value)?)
    })
    .await;
    match result {
        Ok(Ok(bytes)) => (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            bytes,
        )
            .into_response(),
        Ok(Err(_)) => error(StatusCode::BAD_REQUEST, "Invalid leaderboard request"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Read failed"),
    }
}

async fn register(State(app): State<App>, request: Request) -> Response {
    if app.ingestion.is_none() {
        return error(StatusCode::NOT_IMPLEMENTED, "Write preview is disabled");
    }
    let body = match to_bytes(request.into_body(), 1024).await {
        Ok(b) => b,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "Registration too large"),
    };
    let input: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::BAD_REQUEST, "Invalid JSON"),
    };
    if !input.is_object()
        || input
            .as_object()
            .unwrap()
            .keys()
            .any(|k| k != "id" && k != "kind")
        || input
            .get("id")
            .is_some_and(|v| !v.as_str().is_some_and(valid_id))
        || input
            .get("kind")
            .is_some_and(|v| v != "land" && v != "water")
    {
        return error(StatusCode::BAD_REQUEST, "Invalid registration");
    }
    let id = input["id"].as_str().map(str::to_owned);
    match database(app.store, move |s| s.register(id.as_deref())).await {
        Ok(Some(credentials)) => (StatusCode::CREATED, Json(credentials)).into_response(),
        Ok(None) => error(
            StatusCode::CONFLICT,
            "Box ID is already registered; supply its token or choose another ID",
        ),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(e.body())).into_response(),
    }
}
async fn report(State(app): State<App>, headers: HeaderMap, request: Request) -> Response {
    let Some(ingestion) = app.ingestion else {
        return error(StatusCode::NOT_IMPLEMENTED, "Write preview is disabled");
    };
    // Reserve actual received bytes; tiny legacy requests cannot occupy three 8 MiB slots.
    let Ok(mut buffer) = crate::ingestion::REPORT_BUFFERS.clone().try_acquire_owned() else {
        ingestion.metrics().buffer_busy();
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Report buffers are busy; retry later",
        );
    };
    let mut stream = request.into_body().into_data_stream();
    let mut body = Vec::new();
    while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => return error(StatusCode::BAD_REQUEST, "Report body read failed"),
        };
        if body.len() + chunk.len() > MAX_BYTES {
            return error(StatusCode::PAYLOAD_TOO_LARGE, "Report too large");
        }
        let Ok(permit) = crate::ingestion::REPORT_BUFFERS
            .clone()
            .try_acquire_many_owned(chunk.len() as u32)
        else {
            ingestion.metrics().buffer_busy();
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Report buffers are busy; retry later",
            );
        };
        buffer.merge(permit);
        body.extend_from_slice(&chunk);
    }
    let input: Value =
        match tokio::task::spawn_blocking(move || serde_json::from_slice(&body)).await {
            Ok(Ok(input)) => input,
            Ok(Err(_)) => return error(StatusCode::BAD_REQUEST, "Invalid JSON"),
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Report parser failed"),
        };
    let Some(id) = input["id"]
        .as_str()
        .filter(|s| valid_id(s))
        .map(str::to_owned)
    else {
        return error(StatusCode::BAD_REQUEST, "Invalid report");
    };
    let token = push::bearer(&headers);
    let auth_id = id.clone();
    match database(app.store, move |s| s.authenticated(&auth_id, &token)).await {
        Ok(true) => {}
        Ok(false) => return error(StatusCode::UNAUTHORIZED, "Unauthorized"),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(e.body())).into_response(),
    }
    let mut report = json!({"state":input["state"]});
    for key in ["phase", "clientVersion", "coopanionVersion", "clientSystem"] {
        if let Some(value) = input.get(key) {
            report[key] = value.clone();
        }
    }
    match ingestion.commit(&id, &report, "http").await {
        Ok(entry) => Json(json!({"score":entry["score"]})).into_response(),
        Err(e) => (StatusCode::from_u16(e.status).unwrap(), Json(e.body())).into_response(),
    }
}
