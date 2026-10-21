use crate::{
    http::App,
    ingestion::now_ms,
    metrics::{Metrics, memory_bytes},
    operations::OperationError,
    policy::iso,
    push::bearer,
    store::Store,
};
use anyhow::{Result, ensure};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};
#[derive(Clone)]
pub struct AdminNetwork {
    bind: SocketAddr,
    network: u32,
    mask: u32,
}
fn private(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_loopback()
}
impl AdminNetwork {
    pub fn new(bind: SocketAddr, cidr: &str) -> Result<Self> {
        let IpAddr::V4(host) = bind.ip() else {
            anyhow::bail!("Admin host must be a private or loopback IPv4 address");
        };
        ensure!(
            private(host),
            "Admin host must be a private or loopback IPv4 address"
        );
        let (network, bits) = cidr
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("Invalid private admin CIDR"))?;
        let ip: Ipv4Addr = network.parse()?;
        let prefix: u32 = bits.parse()?;
        ensure!(
            private(ip) && (8..=32).contains(&prefix) && bits.bytes().all(|b| b.is_ascii_digit()),
            "Invalid private admin CIDR"
        );
        let mask = u32::MAX << (32 - prefix);
        let network = u32::from(ip) & mask;
        ensure!(
            u32::from(host) & mask == network,
            "Admin host must belong to the allowed CIDR"
        );
        Ok(Self {
            bind,
            network,
            mask,
        })
    }
    pub fn trusted(&self, peer: SocketAddr, headers: &HeaderMap) -> bool {
        let IpAddr::V4(ip) = peer.ip().to_canonical() else {
            return false;
        };
        let authority = self.bind.to_string();
        private(ip)
            && u32::from(ip) & self.mask == self.network
            && headers.get("host").is_some_and(|h| h == authority.as_str())
            && headers
                .get("origin")
                .is_none_or(|h| h == format!("http://{authority}").as_str())
    }
}
#[derive(Clone)]
struct AdminState {
    app: App,
    network: AdminNetwork,
    metrics: Arc<Metrics>,
}
impl IntoResponse for OperationError {
    fn into_response(self) -> Response {
        (
            StatusCode::from_u16(self.status).unwrap(),
            Json(json!({"error":self.message})),
        )
            .into_response()
    }
}
pub async fn operation<T: Send + 'static>(
    store: Store,
    work: impl FnOnce(Store) -> Result<T> + Send + 'static,
) -> std::result::Result<T, OperationError> {
    tokio::task::spawn_blocking(move || work(store))
        .await
        .map_err(|_| OperationError::new(500, "Database task failed"))?
        .map_err(|e| {
            e.downcast::<OperationError>()
                .unwrap_or_else(|_| OperationError::new(500, "Database operation failed"))
        })
}
pub fn router(app: App, network: AdminNetwork) -> Router {
    let metrics = app
        .ingestion
        .as_ref()
        .map(|i| i.metrics())
        .unwrap_or_else(|| Arc::new(Metrics::default()));
    Router::new()
        .fallback(handle)
        .with_state(AdminState { app, network, metrics })
        .layer(axum::middleware::map_response(|mut response: Response| async move {
            if !response.headers().contains_key("cache-control") {response.headers_mut().insert("cache-control", "no-store".parse().unwrap());}
            response.headers_mut().insert("x-content-type-options", "nosniff".parse().unwrap());
            response.headers_mut().insert("content-security-policy", "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; font-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'".parse().unwrap());
            response
        }))
}
fn asset(path: &str) -> Option<(&'static str, &'static [u8])> {
    Some(match path {
        "/" => (
            "text/html; charset=utf-8",
            include_bytes!("../web/admin/index.html"),
        ),
        "/client.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../web/admin/client.js"),
        ),
        "/client.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../web/admin/client.css"),
        ),
        "/assets/icons.svg" => (
            "image/svg+xml",
            include_bytes!("../web/admin/assets/icons.svg"),
        ),
        "/assets/MiSans-Regular.woff2" => (
            "font/woff2",
            include_bytes!("../web/admin/assets/MiSans-Regular.woff2"),
        ),
        "/assets/MiSans-Semibold.woff2" => (
            "font/woff2",
            include_bytes!("../web/admin/assets/MiSans-Semibold.woff2"),
        ),
        "/assets/MiSans-LICENSE.pdf" => (
            "application/pdf",
            include_bytes!("../web/admin/assets/MiSans-LICENSE.pdf"),
        ),
        "/assets/Lucide-LICENSE.txt" => (
            "text/plain; charset=utf-8",
            include_bytes!("../web/admin/assets/Lucide-LICENSE.txt"),
        ),
        _ => return None,
    })
}
async fn body(
    request: Request,
    limit: usize,
    message: &str,
) -> std::result::Result<Value, OperationError> {
    let bytes = to_bytes(request.into_body(), limit)
        .await
        .map_err(|_| OperationError::new(413, message))?;
    serde_json::from_slice(&bytes).map_err(|_| OperationError::new(400, "Invalid JSON"))
}
pub fn cursor(query: Option<&str>) -> std::result::Result<u64, OperationError> {
    let value = query
        .unwrap_or("")
        .split('&')
        .find_map(|p| p.strip_prefix("after="))
        .unwrap_or("0");
    value
        .parse::<u64>()
        .ok()
        .filter(|n| *n <= 9_007_199_254_740_991)
        .ok_or_else(|| OperationError::new(400, "Invalid cursor"))
}
fn path_id(path: &str, prefix: &str, suffix: &str) -> Option<u64> {
    let id = path.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if id.starts_with('0') || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    id.parse::<u64>()
        .ok()
        .filter(|id| *id > 0 && *id <= 9_007_199_254_740_991)
}
async fn handle(
    State(state): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    if !state.network.trusted(peer, request.headers()) {
        return OperationError::new(403, "Forbidden").into_response();
    }
    match dispatch(state, request).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}
async fn dispatch(
    state: AdminState,
    request: Request,
) -> std::result::Result<Response, OperationError> {
    let path = request.uri().path().to_owned();
    let method = request.method().clone();
    if method == Method::GET && path.starts_with("/releases/") {
        let asset = crate::assets::historical(state.app.release_root, path, true)
            .await
            .map_err(|_| OperationError::new(500, "Asset read failed"))?
            .ok_or_else(|| OperationError::new(404, "Not found"))?;
        return Ok((
            [
                ("content-type", asset.mime),
                ("cache-control", "public, max-age=31536000, immutable"),
            ],
            asset.bytes,
        )
            .into_response());
    }
    if !path.starts_with("/api/admin/") {
        if method == Method::GET
            && let Some((mime, bytes)) = asset(&path)
        {
            let page = if path == "/" {
                crate::assets::pin_admin(bytes, state.app.release_id.as_deref())
            } else {
                bytes.to_vec()
            };
            return Ok(([("content-type", mime)], page).into_response());
        }
        return Err(OperationError::new(404, "Not found"));
    }
    let known = matches!(
        path.as_str(),
        "/api/admin/boxes"
            | "/api/admin/controls"
            | "/api/admin/controls/logo"
            | "/api/admin/controls/forum"
            | "/api/admin/controls/http-push"
            | "/api/admin/controls/minimum-client-version"
            | "/api/admin/feedback"
            | "/api/admin/announcements"
    ) || path.starts_with("/api/admin/feedback/")
        || path.starts_with("/api/admin/announcements/");
    if !known {
        return Err(OperationError::new(404, "Not found"));
    }
    let token = bearer(request.headers());
    if !operation(state.app.store.clone(), move |s| {
        s.admin_authenticated(&token)
    })
    .await?
    {
        return Err(OperationError::new(401, "Unauthorized"));
    }
    let store = state.app.store;
    let metrics = state.metrics;
    if method == Method::GET {
        let query = request.uri().query().map(str::to_owned);
        let route = path.clone();
        let data = operation(store, move |s| match route.as_str() {
            "/api/admin/boxes" => {
                let mut boxes = s.admin_boxes()?;
                metrics.annotate(&mut boxes);
                Ok(json!({
                    "boxes": boxes,
                    "serverAt": iso(now_ms()),
                    "uptimeSec": metrics.uptime(),
                    "memoryBytes": memory_bytes(),
                    "acceptedReports": metrics.accepted_reports()
                }))
            }
            "/api/admin/controls" => s.controls(now_ms()),
            "/api/admin/announcements" => s.list_announcements(),
            "/api/admin/feedback" => s.list_feedback(cursor(query.as_deref())?),
            _ => Err(OperationError::new(405, "Method not allowed").into()),
        })
        .await?;
        return Ok(Json(data).into_response());
    }
    if state.app.ingestion.is_none() {
        return Err(OperationError::new(501, "Write preview is disabled"));
    }
    match (method, path.as_str()) {
        (
            Method::PUT,
            "/api/admin/controls/http-push"
            | "/api/admin/controls/minimum-client-version"
            | "/api/admin/controls/logo"
            | "/api/admin/controls/forum",
        ) => {
            let input = body(request, 1024, "Control too large").await?;
            let key = if path.ends_with("/http-push") {
                "httpPush"
            } else if path.ends_with("/forum") {
                "forum"
            } else if path.ends_with("/logo") {
                "logo"
            } else {
                "minimumClientVersion"
            };
            let controls = operation(store, move |s| {
                s.update_named_control(key, &input, now_ms())
            })
            .await?;
            Ok(Json(controls).into_response())
        }
        (Method::POST, "/api/admin/announcements") => {
            let input = body(request, 16384, "Announcement too large").await?;
            let notice =
                operation(store, move |s| s.publish_announcement(&input, now_ms())).await?;
            Ok((StatusCode::CREATED, Json(notice)).into_response())
        }
        (Method::DELETE, _) if path_id(&path, "/api/admin/announcements/", "").is_some() => {
            let id = path_id(&path, "/api/admin/announcements/", "").unwrap();
            let removed = operation(store, move |s| s.revoke_announcement(id)).await?;
            Ok((
                if removed {
                    StatusCode::OK
                } else {
                    StatusCode::NOT_FOUND
                },
                Json(json!({"removed":removed})),
            )
                .into_response())
        }
        (Method::POST, _) if path_id(&path, "/api/admin/feedback/", "/resolve").is_some() => {
            let id = path_id(&path, "/api/admin/feedback/", "/resolve").unwrap();
            let resolved = operation(store, move |s| s.resolve_feedback(id, now_ms())).await?;
            Ok((
                if resolved {
                    StatusCode::OK
                } else {
                    StatusCode::NOT_FOUND
                },
                Json(json!({"resolved":resolved})),
            )
                .into_response())
        }
        _ => Err(OperationError::new(405, "Method not allowed")),
    }
}
pub async fn feedback(State(app): State<App>, request: Request) -> Response {
    let result = async {
        if app.ingestion.is_none() {
            return Err(OperationError::new(501, "Write preview is disabled"));
        }
        if request.method() != Method::POST {
            return Err(OperationError::new(405, "Method not allowed"));
        }
        let authorization = request.headers().contains_key("authorization");
        let token = bearer(request.headers());
        let input = body(request, 16384, "Feedback too large").await?;
        if !input.is_object() {
            return Err(OperationError::new(400, "Invalid feedback"));
        }
        let box_id = if input.get("id").is_some() || authorization {
            let id = input["id"].as_str().unwrap_or("").to_owned();
            let auth_id = id.clone();
            if !operation(app.store.clone(), move |s| {
                s.authenticated(&auth_id, &token)
            })
            .await?
            {
                return Err(OperationError::new(401, "Unauthorized"));
            }
            Some(id)
        } else {
            None
        };
        let receipt = operation(app.store, move |s| {
            s.submit_feedback(&input, box_id.as_deref(), now_ms())
        })
        .await?;
        Ok::<_, OperationError>((StatusCode::CREATED, Json(receipt)).into_response())
    }
    .await;
    result.unwrap_or_else(IntoResponse::into_response)
}
