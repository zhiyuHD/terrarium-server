//! Release-qualified URLs keep open pages on the assets shipped with their server release.
use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn valid_release_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 80
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(b))
}
pub fn pin_public(page: &str, id: Option<&str>) -> String {
    match id.filter(|id| valid_release_id(id)) {
        Some(id) => page.replace(
            "./scene.js?v=dark-mode",
            &format!("/releases/{id}/scene.js"),
        ),
        None => page.to_owned(),
    }
}
pub fn pin_admin(page: &[u8], id: Option<&str>) -> Vec<u8> {
    let Some(id) = id.filter(|id| valid_release_id(id)) else {
        return page.to_vec();
    };
    let mut html = String::from_utf8(page.to_vec()).unwrap();
    for path in [
        "client.js",
        "client.css",
        "assets/icons.svg",
        "assets/MiSans-Regular.woff2",
        "assets/MiSans-Semibold.woff2",
        "assets/MiSans-LICENSE.pdf",
        "assets/Lucide-LICENSE.txt",
    ] {
        html = html.replace(
            &format!("=\"/{path}\""),
            &format!("=\"/releases/{id}/{path}\""),
        );
    }
    html.into_bytes()
}
pub struct Asset {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}
fn location(root: &Path, path: &str, private: bool) -> Option<(PathBuf, &'static str)> {
    let (id, path) = path.strip_prefix("/releases/")?.split_once('/')?;
    if !valid_release_id(id) {
        return None;
    }
    let mime = match (private, path) {
        (_, "scene.js") if !private => "text/javascript; charset=utf-8",
        (true, "client.js") => "text/javascript; charset=utf-8",
        (true, "client.css") => "text/css; charset=utf-8",
        (true, "assets/icons.svg") => "image/svg+xml",
        (true, "assets/MiSans-Regular.woff2" | "assets/MiSans-Semibold.woff2") => "font/woff2",
        (true, "assets/MiSans-LICENSE.pdf") => "application/pdf",
        (true, "assets/Lucide-LICENSE.txt") => "text/plain; charset=utf-8",
        _ => return None,
    };
    let directory = root.join("versions").join(id);
    Some((
        if private {
            directory.join("console")
        } else {
            directory
        }
        .join(path),
        mime,
    ))
}
pub async fn historical(
    root: Option<PathBuf>,
    path: String,
    private: bool,
) -> Result<Option<Asset>> {
    let Some((file, mime)) = root.and_then(|root| location(&root, &path, private)) else {
        return Ok(None);
    };
    tokio::task::spawn_blocking(move || {
        let meta = match std::fs::symlink_metadata(&file) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        anyhow::ensure!(
            meta.is_file() && meta.len() <= 32 * 1024 * 1024,
            "Invalid release asset"
        );
        Ok(Some(Asset {
            bytes: std::fs::read(file)?,
            mime,
        }))
    })
    .await?
}
