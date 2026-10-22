//! Public branding is selected by the persisted cloud control.
use crate::{
    http::App,
    ingestion::{database, now_ms},
};
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use serde_json::json;
pub fn logo(version: &str) -> &'static str {
    match version {
        "0.8.8" => include_str!("../web/logo-0.8.8.svg"),
        "0.8.7" => include_str!("../web/logo-0.8.7.svg"),
        _ => include_str!("../web/logo-0.8.6.svg"),
    }
}
pub fn page(version: &str, release: Option<&str>) -> String {
    crate::assets::pin_public(
        &include_str!("../web/index.html")
            .replace("<!-- RELEASE_LOGO -->", logo(version))
            .replace("<!-- LOGO_VERSION -->", version),
        release,
    )
}
pub async fn index(State(app): State<App>) -> Response {
    match database(app.store, |s| s.controls(now_ms())).await {
        Ok(c) => Html(page(
            c["logo"]["version"].as_str().unwrap_or("0.8.6"),
            app.release_id.as_deref(),
        ))
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(e.body())).into_response(),
    }
}
pub async fn status(State(app): State<App>) -> Response {
    match database(app.store, |s| s.controls(now_ms())).await {
        Ok(c) => {
            let version = c["logo"]["version"].as_str().unwrap_or("0.8.6");
            Json(json!({"logoVersion":version,"logo":logo(version)})).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(e.body())).into_response(),
    }
}
