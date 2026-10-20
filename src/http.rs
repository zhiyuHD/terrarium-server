use crate::{
    ingestion::{Ingestion, database, now_ms},
    protocol::{MAX_BYTES, valid_id},
    push,
    scoring::score_parts,
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
use std::collections::HashMap;
#[derive(Clone)]
pub struct App {
    pub store: Store,
    pub release_id: Option<String>,
    pub ingestion: Option<Ingestion>,
}
pub fn router(app: App) -> Router {
    Router::new()
        .route("/",get(||async{Html(include_str!("../web/index.html"))}))
        .route("/scene.js",get(||async{([(axum::http::header::CONTENT_TYPE,"text/javascript; charset=utf-8")],include_str!("../web/scene.js"))}))
        .route("/health",get(health))
        .route("/api/leaderboard",get(leaderboard))
        .route("/api/boxes/{id}",get(box_detail))
        .route("/api/feedback",any(crate::admin::feedback))
        .route("/api/admin",any(||async{StatusCode::NOT_FOUND}))
        .route("/api/admin/{*path}",any(||async{StatusCode::NOT_FOUND}))
        .route("/api/registrations",post(register))
        .route("/api/reports",put(report))
        .route("/api/reports/ws",get(push::upgrade))
        .route("/api/announcements",get(push::announcements))
        .route("/api/announcements/public",get(push::public_announcements))
        .fallback(||async{(StatusCode::NOT_IMPLEMENTED,Json(json!({"error":"Rust migration preview: this endpoint is not implemented","accepted":false})))})
        .layer(axum::middleware::map_response(|mut response:Response|async move{response.headers_mut().insert("cache-control","no-store".parse().unwrap());response.headers_mut().insert("x-content-type-options","nosniff".parse().unwrap());response}))
        .layer(DefaultBodyLimit::disable())
        .with_state(app)
}
async fn health(State(app): State<App>) -> Response {
    let controls = match database(app.store, |s| s.controls(now_ms())).await {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(e.body())).into_response(),
    };
    Json(json!({"ok":true,"version":env!("CARGO_PKG_VERSION"),"releaseId":app.release_id,"implementation":"rust","readOnly":app.ingestion.is_none(),"websocketPush":app.ingestion.is_some(),"httpPushEnabled":app.ingestion.is_some()&&controls["httpPush"]["enabled"]==true,"minimumClientVersion":controls["minimumClientVersion"]["version"]})).into_response()
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
    let result=tokio::task::spawn_blocking(move||->anyhow::Result<Value>{
        if query.get("view").is_some_and(|v|v=="gallery") {
            let kind=kind.as_deref().ok_or_else(||anyhow::anyhow!("Gallery requires a box kind"))?;
            let number=|key:&str,default:usize|->anyhow::Result<usize>{Ok(query.get(key).map(|v|v.parse()).transpose()?.unwrap_or(default))};
            let page=number("page",1)?;let size=number("pageSize",4)?;let sort=query.get("sort").map(String::as_str).unwrap_or("score");let search=query.get("q").map(String::as_str).unwrap_or("").trim();
            anyhow::ensure!(page>0&&page<=9_007_199_254_740_991&&(1..=12).contains(&size)&&matches!(sort,"score"|"recent")&&search.encode_utf16().count()<=100,"Invalid gallery pagination, sort or search");
            return app.store.gallery(kind,search,sort=="recent",page,size);
        }
        let entries=app.store.ranked(kind.as_deref())?;
        if query.get("view").is_some_and(|v|v=="summary") {
            let kind=kind.ok_or_else(||anyhow::anyhow!("Summary requires kind"))?;
            let limit=query.get("limit").map(|v|v.parse::<usize>()).transpose()?.unwrap_or(20);anyhow::ensure!((1..=100).contains(&limit),"Invalid summary limit");
            let summarize=|i:usize|->anyhow::Result<Value>{let entry=&entries[i];let mut row=json!({"rank":i+1,"competitiveRank":entries.iter().position(|e|e["score"]==entry["score"]).unwrap()+1,"id":entry["id"],"score":entry["score"],"scoreParts":score_parts(&entry["state"])?,"reportedAt":entry["reportedAt"]});if let Some(name)=entry["state"].get("name"){row["name"]=name.clone();}Ok(row)};
            let own=query.get("ownId").and_then(|id|entries.iter().position(|e|e["id"]==*id)).map(summarize).transpose()?;
            return Ok(json!({"habitat":kind,"total":entries.len(),"entries":(0..entries.len().min(limit)).map(summarize).collect::<anyhow::Result<Vec<_>>>()?,"own":own}));
        }
        Ok(Value::Array(entries.into_iter().take(100).collect()))
    }).await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
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
    let body = match to_bytes(request.into_body(), MAX_BYTES).await {
        Ok(b) => b,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "Report too large"),
    };
    let input: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::BAD_REQUEST, "Invalid JSON"),
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
    for key in ["phase", "clientVersion", "coopanionVersion"] {
        if let Some(value) = input.get(key) {
            report[key] = value.clone();
        }
    }
    match ingestion.commit(&id, &report, "http").await {
        Ok(entry) => Json(json!({"score":entry["score"]})).into_response(),
        Err(e) => (StatusCode::from_u16(e.status).unwrap(), Json(e.body())).into_response(),
    }
}
