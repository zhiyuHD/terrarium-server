//! Authenticated photographs use the acknowledged state hash and return immutable PNG bytes.
use crate::{
    http::App,
    protocol::{MAX_BYTES, state_hash, valid_hash, valid_id},
};
use axum::{
    Json,
    body::to_bytes,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::{path::PathBuf, process::Stdio, sync::LazyLock, time::Duration};
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
pub async fn take(State(app): State<App>, request: Request) -> Response {
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
    let store = app.store;
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
        Ok(bytes) => (
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
            ],
            bytes,
        )
            .into_response(),
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
