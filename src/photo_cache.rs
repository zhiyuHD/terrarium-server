//! Expiring PNG files live outside releases and SQLite; downloads use bounded buffers.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub const PHOTO_TTL_MS: i64 = 24 * 60 * 60 * 1000;
pub const MAX_CACHE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_PHOTOS: usize = 4096;
pub const CLEANUP_SECONDS: u64 = 60;

pub struct PhotoCache {
    dir: PathBuf,
    writer: Mutex<()>,
}
pub struct PhotoLink {
    pub key: String,
    pub expires_at_ms: i64,
}
impl PhotoCache {
    pub fn new(dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            dir,
            writer: Mutex::new(()),
        })
    }
    pub fn save(&self, bytes: Vec<u8>, now: i64) -> Result<(Vec<u8>, PhotoLink)> {
        ensure!(
            bytes.len() <= crate::protocol::MAX_BYTES && bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "Invalid cached photo"
        );
        let _guard = self
            .writer
            .lock()
            .map_err(|_| anyhow::anyhow!("Photo cache lock poisoned"))?;
        self.cleanup(now)?;
        fs::create_dir_all(&self.dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700))?;
        }
        let mut count = 0;
        let mut size = 0u64;
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            if parse_key(&entry.file_name().to_string_lossy()).is_some() {
                let metadata = match entry.metadata() {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.into()),
                };
                count += 1;
                size += metadata.len();
            }
        }
        ensure!(
            count < MAX_PHOTOS && size + bytes.len() as u64 <= MAX_CACHE_BYTES,
            "Temporary photo storage is full"
        );
        let expires_at_ms = now
            .checked_add(PHOTO_TTL_MS)
            .context("Invalid photo expiry")?;
        let key = format!("{expires_at_ms}-{}.png", crate::store::random_hex(32));
        let temporary = self.dir.join(format!("{key}.tmp"));
        let destination = self.dir.join(&key);
        let write = || -> Result<()> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &destination)?;
            Ok(())
        };
        if let Err(error) = write() {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        Ok((bytes, PhotoLink { key, expires_at_ms }))
    }
    pub fn open(&self, key: &str, now: i64) -> Result<Option<(File, u64, i64)>> {
        let Some(expiry) = parse_key(key) else {
            return Ok(None);
        };
        if expiry <= now {
            self.remove(key)?;
            return Ok(None);
        }
        let path = self.dir.join(key);
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.len() <= crate::protocol::MAX_BYTES as u64,
            "Invalid photo file"
        );
        Ok(Some((file, metadata.len(), expiry)))
    }
    pub fn remove(&self, key: &str) -> Result<()> {
        ensure!(parse_key(key).is_some(), "Invalid photo key");
        remove(&self.dir.join(key))
    }
    pub fn cleanup(&self, now: i64) -> Result<usize> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        };
        let mut removed = 0;
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if parse_key(&name).is_some_and(|expiry| expiry <= now)
                || name
                    .strip_suffix(".tmp")
                    .and_then(parse_key)
                    .is_some_and(|expiry| expiry <= now)
            {
                remove(&entry.path())?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}
fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
pub fn parse_key(key: &str) -> Option<i64> {
    let (time, token) = key.strip_suffix(".png")?.split_once('-')?;
    if time.is_empty()
        || time.len() > 16
        || !time.bytes().all(|b| b.is_ascii_digit())
        || token.len() != 64
        || !token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    time.parse::<i64>().ok().filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\ntemporary-image";
    #[test]
    fn photos_expire_after_24_hours_survive_reopening_and_cleanup_without_reads() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("photos");
        let cache = PhotoCache::new(root.clone());
        let now = 1_800_000_000_000;
        let (_, old) = cache.save(PNG.to_vec(), now).unwrap();
        let (_, next) = cache.save(PNG.to_vec(), now + 1000).unwrap();
        assert_ne!(old.key, next.key);
        assert_eq!(old.expires_at_ms, now + PHOTO_TTL_MS);
        drop(cache);
        let cache = PhotoCache::new(root.clone());
        assert!(
            cache
                .open(&old.key, now + PHOTO_TTL_MS - 1)
                .unwrap()
                .is_some()
        );
        assert_eq!(cache.cleanup(now + PHOTO_TTL_MS).unwrap(), 1);
        assert!(!root.join(&old.key).exists());
        assert!(cache.open(&old.key, now + PHOTO_TTL_MS).unwrap().is_none());
        assert!(cache.open(&next.key, now + PHOTO_TTL_MS).unwrap().is_some());
        assert!(
            cache
                .open(&next.key, now + PHOTO_TTL_MS + 1000)
                .unwrap()
                .is_none()
        );
        assert_eq!(fs::read_dir(root).unwrap().count(), 0);
    }
    #[test]
    fn bounds_storage_without_evicting_live_links_and_rejects_paths() {
        let dir = TempDir::new().unwrap();
        let cache = PhotoCache::new(dir.path().join("photos"));
        let now = 1_800_000_000_000;
        let (_, link) = cache.save(PNG.to_vec(), now).unwrap();
        for key in [
            "../outside.png",
            "/etc/passwd",
            "123-abc.png",
            "0-abc.png",
            &format!("{}-{}.png", now, "A".repeat(64)),
        ] {
            assert!(cache.open(key, now).unwrap().is_none());
        }
        for i in 0..(MAX_CACHE_BYTES / crate::protocol::MAX_BYTES as u64) {
            let key = format!("{}-{i:064x}.png", now + PHOTO_TTL_MS);
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(cache.dir.join(key))
                .unwrap();
            file.write_all(PNG).unwrap();
            file.set_len(crate::protocol::MAX_BYTES as u64).unwrap();
        }
        assert!(cache.save(PNG.to_vec(), now).is_err());
        assert!(cache.dir.join(&link.key).exists());
        cache.cleanup(now + PHOTO_TTL_MS).unwrap();
        assert!(cache.save(PNG.to_vec(), now + PHOTO_TTL_MS).is_ok());
    }
}
