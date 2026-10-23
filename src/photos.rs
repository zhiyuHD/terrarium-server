//! Authenticated photographs use the acknowledged state hash and return immutable PNG bytes.
use crate::{
    http::App,
    protocol::{MAX_BYTES, state_hash, valid_hash, valid_id},
};
use axum::{
    Extension, Json,
    body::{Body, to_bytes},
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::Semaphore,
};

enum Snapshot {
    Denied,
    Changed,
    Ready(Value),
}

static RENDERER: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(1));
fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error":message}))).into_response()
}
pub async fn take(
    State(app): State<App>,
    Extension(cache): Extension<Arc<crate::photo_cache::PhotoCache>>,
    request: Request,
) -> Response {
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("")
        .to_owned();
    let body = match to_bytes(request.into_body(), 4096).await {
        Ok(bytes) => bytes,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "Photo request too large"),
    };
    let input: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "Invalid photo request"),
    };
    let id = input["id"].as_str().unwrap_or("").to_owned();
    let caption = input.get("caption").and_then(Value::as_str).unwrap_or("");
    if !valid_id(&id)
        || !valid_hash(&input["stateHash"])
        || !matches!(input["phase"].as_str(), Some("day" | "night"))
        || !crate::protocol::valid_version(&input["clientVersion"])
        || caption.chars().count() > 120
        || input.get("caption").is_some_and(|v| !v.is_string())
        || caption.chars().any(|c| c.is_control() && c != '\n')
    {
        return error(
            StatusCode::BAD_REQUEST,
            "Invalid photo request; caption allows at most 120 characters",
        );
    }
    let _permit = match RENDERER.try_acquire() {
        Ok(permit) => permit,
        Err(_) => {
            return error(
                StatusCode::TOO_MANY_REQUESTS,
                "Another photo is rendering; retry shortly",
            );
        }
    };
    let expected_hash = input["stateHash"].as_str().unwrap().to_owned();
    let expected_phase = input["phase"].clone();
    let store = app.store.clone();
    let entry = match tokio::task::spawn_blocking(move || -> anyhow::Result<Snapshot> {
        if !store.authenticated(&id, &token)? {
            return Ok(Snapshot::Denied);
        }
        let Some(entry) = store.get(&id)? else {
            return Ok(Snapshot::Denied);
        };
        if state_hash(&entry["state"]) != expected_hash || entry["phase"] != expected_phase {
            return Ok(Snapshot::Changed);
        }
        Ok(Snapshot::Ready(crate::store::scene_entry(&entry)))
    })
    .await
    {
        Ok(Ok(Snapshot::Ready(entry))) => entry,
        Ok(Ok(Snapshot::Denied)) => {
            return error(
                StatusCode::UNAUTHORIZED,
                "Claim and submit this box before taking a photo",
            );
        }
        Ok(Ok(Snapshot::Changed)) => {
            return error(
                StatusCode::CONFLICT,
                "The submitted snapshot changed; sync again before taking a photo",
            );
        }
        _ => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Photo snapshot read failed",
            );
        }
    };
    let captured_at = crate::policy::iso(crate::ingestion::now_ms());
    let metadata = json!({"boxId":entry["id"],"habitat":entry["kind"],"capturedAt":captured_at,"stateHash":input["stateHash"],"caption":caption});
    let payload = json!({"entry":entry,"caption":caption,"capturedAt":captured_at,"stateHash":input["stateHash"]});
    let worker = match (app.release_root, app.release_id) {
        (Some(root), Some(id)) if crate::assets::valid_release_id(&id) => root
            .join("versions")
            .join(id)
            .join("console/assets/photo-worker.mjs"),
        (None, None) => {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("photo/dist/photo-worker.mjs")
        }
        _ => {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Invalid photo release configuration",
            );
        }
    };
    match render(worker, payload).await {
        Ok(bytes) => match publish(cache, app.store, bytes, metadata).await {
            Ok((bytes, url, expires_at)) => (
                [
                    ("content-type", "image/png"),
                    (
                        "content-disposition",
                        "inline; filename=terrarium-photo.png",
                    ),
                    (
                        "x-terrarium-state-hash",
                        input["stateHash"].as_str().unwrap(),
                    ),
                    ("x-terrarium-captured-at", captured_at.as_str()),
                    ("x-terrarium-photo-url", url.as_str()),
                    ("x-terrarium-photo-expires-at", expires_at.as_str()),
                ],
                bytes,
            )
                .into_response(),
            Err(error) => {
                eprintln!("Photo link publication failed: {error:#}");
                self::error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Temporary photo storage unavailable; retry later",
                )
            }
        },
        Err(error) => {
            eprintln!("Photo render failed: {error:#}");
            self::error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Photo rendering unavailable; retry later or contact the administrator",
            )
        }
    }
}
async fn render(worker: PathBuf, input: Value) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(worker.is_file(), "Photo worker is not installed");
    let mut child = Command::new(
        std::env::var("CORTICO_TERRARIUM_LEGACY_NODE").unwrap_or_else(|_| "node".into()),
    )
    .arg("--max-old-space-size=96")
    .arg(worker)
    .env("DISABLE_SYSTEM_FONTS_LOAD", "1")
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .kill_on_drop(true)
    .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let bytes = serde_json::to_vec(&input)?;
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        let write = async move {
            stdin.write_all(&bytes).await?;
            drop(stdin);
            Ok::<_, std::io::Error>(())
        };
        let read = async {
            let mut bytes = Vec::new();
            stdout
                .take((MAX_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .await?;
            Ok::<_, std::io::Error>(bytes)
        };
        let (_, bytes) = tokio::try_join!(write, read)?;
        anyhow::ensure!(
            bytes.len() <= MAX_BYTES && bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "Renderer returned an invalid or oversized image"
        );
        anyhow::ensure!(
            child.wait().await?.success(),
            "Renderer exited unsuccessfully"
        );
        Ok::<_, anyhow::Error>(bytes)
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => {
            let _ = child.kill().await;
            anyhow::bail!("Photo rendering timed out")
        }
    }
}

fn public_origin(value: &str) -> anyhow::Result<String> {
    let value = value.trim_end_matches('/');
    let uri: axum::http::Uri = value.parse()?;
    anyhow::ensure!(
        uri.scheme_str() == Some("https")
            && uri.authority().is_some_and(|a| !a.as_str().contains('@'))
            && uri.path_and_query().is_none_or(|p| p.as_str() == "/")
            && value.len() <= 256,
        "Public photo origin must be an HTTPS origin"
    );
    Ok(value.to_owned())
}
async fn publish(
    cache: Arc<crate::photo_cache::PhotoCache>,
    store: crate::store::Store,
    bytes: Vec<u8>,
    mut data: Value,
) -> anyhow::Result<(Vec<u8>, String, String)> {
    let origin = public_origin(
        &std::env::var("CORTICO_TERRARIUM_PUBLIC_URL")
            .unwrap_or_else(|_| "https://cortico-box.zhiyuhub.top".into()),
    )?;
    tokio::task::spawn_blocking(move || {
        let now = crate::ingestion::now_ms();
        let (bytes, link) = cache.save(bytes, now)?;
        let url = format!("{origin}/api/photos/{}", link.key);
        let expiry = crate::policy::iso(link.expires_at_ms);
        data["url"] = json!(url);
        data["expiresAt"] = json!(expiry);
        if let Err(error) = store.photo_receipt(data["boxId"].as_str().unwrap(), &data, now) {
            cache.remove(&link.key)?;
            return Err(error);
        }
        Ok((bytes, url, expiry))
    })
    .await?
}

pub fn start_cleanup(cache: &Arc<crate::photo_cache::PhotoCache>, store: crate::store::Store) {
    let weak = Arc::downgrade(cache);
    tokio::spawn(async move {
        while let Some(cache) = weak.upgrade() {
            let store = store.clone();
            let result = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                let now = crate::ingestion::now_ms();
                cache.cleanup(now)?;
                store.cleanup_photo_receipts(now)?;
                Ok(())
            })
            .await;
            if !matches!(result, Ok(Ok(()))) {
                eprintln!("Temporary photo cleanup failed");
            }
            tokio::time::sleep(Duration::from_secs(crate::photo_cache::CLEANUP_SECONDS)).await;
        }
    });
}
static DOWNLOADS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(16)));
pub async fn download(
    Extension(cache): Extension<Arc<crate::photo_cache::PhotoCache>>,
    Path(key): Path<String>,
) -> Response {
    let permit = match DOWNLOADS.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return error(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many photo downloads; retry shortly",
            );
        }
    };
    let result =
        tokio::task::spawn_blocking(move || cache.open(&key, crate::ingestion::now_ms())).await;
    let (file, size, expiry) = match result {
        Ok(Ok(Some(value))) => value,
        Ok(Ok(None)) => return error(StatusCode::NOT_FOUND, "Photo not found or expired"),
        _ => return error(StatusCode::SERVICE_UNAVAILABLE, "Photo read unavailable"),
    };
    let stream = futures_util::stream::try_unfold(
        (tokio::fs::File::from_std(file), permit),
        |(mut file, permit)| async move {
            let mut bytes = vec![0; 64 * 1024];
            let read = file.read(&mut bytes).await?;
            if read == 0 {
                return Ok::<_, std::io::Error>(None);
            }
            bytes.truncate(read);
            Ok(Some((bytes, (file, permit))))
        },
    );
    (
        [
            ("content-type", "image/png".to_owned()),
            ("content-length", size.to_string()),
            ("cache-control", "no-store".to_owned()),
            (
                "content-disposition",
                "inline; filename=terrarium-photo.png".to_owned(),
            ),
            ("x-robots-tag", "noindex, nofollow".to_owned()),
            ("x-terrarium-photo-expires-at", crate::policy::iso(expiry)),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_an_https_origin_without_credentials_paths_or_query() {
        assert_eq!(
            public_origin("https://photos.example.test/").unwrap(),
            "https://photos.example.test"
        );
        for input in [
            "http://photos.example.test",
            "https://user:secret@example.test",
            "https://example.test/path",
            "https://example.test?key=value",
            "/relative",
        ] {
            assert!(public_origin(input).is_err());
        }
    }
}
