use crate::scoring::score;
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

const DOCUMENTS: &[&str] = &[
    "leaderboard.json",
    "credentials.json",
    "controls.json",
    "announcements.json",
    "feedback.json",
    "protocol-notices.json",
    "admin-token",
];
#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    writer: Arc<Mutex<Connection>>,
}
pub struct LegacySnapshot {
    documents: BTreeMap<String, Vec<u8>>,
    boxes: BTreeMap<String, Value>,
    credentials: BTreeMap<String, String>,
}
impl LegacySnapshot {
    pub fn read(dir: &Path) -> Result<Self> {
        let mut documents = BTreeMap::new();
        for name in DOCUMENTS {
            let path = dir.join(name);
            match fs::symlink_metadata(&path) {
                Ok(meta) => {
                    ensure!(
                        meta.is_file(),
                        "Snapshot input must be a regular file: {name}"
                    );
                    let bytes = fs::read(path)?;
                    if name.ends_with(".json") {
                        let value = serde_json::from_slice::<Value>(&bytes)
                            .with_context(|| format!("Invalid JSON in {name}"))?;
                        crate::policy::validate_document(name, &value)?;
                    }
                    documents.insert((*name).to_owned(), bytes);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        let boxes: BTreeMap<String, Value> = serde_json::from_slice(
            documents
                .get("leaderboard.json")
                .context("Missing leaderboard.json")?,
        )?;
        let credentials: BTreeMap<String, String> = serde_json::from_slice(
            documents
                .get("credentials.json")
                .context("Missing credentials.json")?,
        )?;
        for (id, entry) in &boxes {
            validate_entry(id, entry)?;
        }
        for (id, token) in &credentials {
            ensure!(
                valid_id(id) && !token.is_empty(),
                "Invalid credential entry"
            );
        }
        Ok(Self {
            documents,
            boxes,
            credentials,
        })
    }
    pub fn counts(&self) -> (usize, usize, usize) {
        (
            self.boxes.len(),
            self.credentials.len(),
            self.documents.len(),
        )
    }
}
pub fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    (2..=40).contains(&b.len())
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}
fn validate_entry(id: &str, entry: &Value) -> Result<()> {
    ensure!(
        valid_id(id)
            && entry["id"] == id
            && entry["state"].is_object()
            && entry["reportedAt"].is_string(),
        "Invalid box entry"
    );
    let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("land");
    ensure!(
        matches!(kind, "land" | "water")
            && (kind == "water") == entry["state"].get("fish").is_some(),
        "Invalid box habitat"
    );
    score(&entry["state"])?;
    Ok(())
}
impl Store {
    pub fn existing(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        ensure!(path.is_file(), "Database does not exist");
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        ensure!(version == 1, "Unsupported database schema");
        drop(conn);
        Self::open(path)
    }
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_owned();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let file = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .mode(0o600)
                .open(&path)?;
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        let conn = Connection::open(&path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        ensure!(version <= 1, "Database schema is newer than this binary");
        conn.execute_batch("CREATE TABLE IF NOT EXISTS boxes(id TEXT PRIMARY KEY,kind TEXT NOT NULL,score REAL NOT NULL,reported_at TEXT NOT NULL,revision TEXT,entry TEXT NOT NULL,id_sort TEXT NOT NULL,search TEXT NOT NULL,plants INTEGER NOT NULL,animals INTEGER NOT NULL); CREATE INDEX IF NOT EXISTS boxes_rank ON boxes(kind,score DESC,id_sort); CREATE INDEX IF NOT EXISTS boxes_recent ON boxes(kind,reported_at DESC,id_sort); CREATE TABLE IF NOT EXISTS credentials(id TEXT PRIMARY KEY,token TEXT NOT NULL); CREATE TABLE IF NOT EXISTS legacy_documents(name TEXT PRIMARY KEY,content BLOB NOT NULL); PRAGMA user_version=1;")?;
        Ok(Self {
            path,
            writer: Arc::new(Mutex::new(conn)),
        })
    }
    fn writer(&self) -> Result<MutexGuard<'_, Connection>> {
        self.writer
            .lock()
            .map_err(|_| anyhow::anyhow!("Database writer lock poisoned"))
    }
    pub fn reader(&self) -> Result<Connection> {
        let conn = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(conn)
    }
    pub(crate) fn transaction<T>(
        &self,
        work: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut conn = self.writer()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let value = work(&tx)?;
        tx.commit()?;
        Ok(value)
    }
    pub fn authenticated(&self, id: &str, token: &str) -> Result<bool> {
        use sha2::{Digest, Sha256};
        use subtle::ConstantTimeEq;
        let expected: Option<String> = self
            .reader()?
            .query_row("SELECT token FROM credentials WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        let actual = Sha256::digest(token.as_bytes());
        let hash = Sha256::digest(expected.as_deref().unwrap_or("").as_bytes());
        Ok(bool::from(actual.ct_eq(&hash)) && expected.is_some())
    }
    pub fn register(&self, requested: Option<&str>) -> Result<Option<Value>> {
        self.transaction(|tx|{let id=requested.map(str::to_owned).unwrap_or_else(||format!("box-{}",random_hex(12)));let count:i64=tx.query_row("SELECT (SELECT count(*) FROM credentials WHERE id=?1)+(SELECT count(*) FROM boxes WHERE id=?1)",[&id],|r|r.get(0))?;if count>0{return Ok(None);}let token=random_hex(32);tx.execute("INSERT INTO credentials VALUES(?1,?2)",params![id,token])?;tx.execute("DELETE FROM legacy_documents WHERE name='credentials.json'",[])?;Ok(Some(serde_json::json!({"id":id,"token":token})))})
    }
    pub fn commit_entry(&self, entry: &Value, previous: Option<&Value>) -> Result<bool> {
        let id = entry["id"].as_str().context("Invalid ID")?;
        validate_entry(id, entry)?;
        self.transaction(|tx| {
            let current: Option<Option<String>> = tx
                .query_row("SELECT revision FROM boxes WHERE id=?1", [id], |r| r.get(0))
                .optional()?;
            let expected = previous.map(|e| e["reportRevision"].as_str().map(str::to_owned));
            if current != expected {
                return Ok(false);
            }
            insert_entry(tx, id, entry)?;
            tx.execute(
                "DELETE FROM legacy_documents WHERE name='leaderboard.json'",
                [],
            )?;
            Ok(true)
        })
    }
    pub fn import(&self, snapshot: LegacySnapshot) -> Result<()> {
        let mut conn = self.writer()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count:i64=tx.query_row("SELECT (SELECT count(*) FROM boxes)+(SELECT count(*) FROM credentials)+(SELECT count(*) FROM legacy_documents)",[],|r|r.get(0))?;
        ensure!(count == 0, "Import requires an empty database");
        for (id, entry) in snapshot.boxes {
            insert_entry(&tx, &id, &entry)?;
        }
        for (id, token) in snapshot.credentials {
            tx.execute("INSERT INTO credentials VALUES(?1,?2)", params![id, token])?;
        }
        for (name, bytes) in snapshot.documents {
            tx.execute(
                "INSERT INTO legacy_documents VALUES(?1,?2)",
                params![name, bytes],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn get(&self, id: &str) -> Result<Option<Value>> {
        let conn = self.reader()?;
        let row: Option<(String, f64)> = conn
            .query_row("SELECT entry,score FROM boxes WHERE id=?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        row.map(|(text, score)| {
            let mut value: Value = serde_json::from_str(&text)?;
            value["score"] = Value::from(score);
            Ok(value)
        })
        .transpose()
    }
    /// Compare and replace one report in a single write transaction. Failed comparisons write nothing.
    pub fn replace_report(
        &self,
        id: &str,
        expected_revision: Option<&str>,
        entry: &Value,
    ) -> Result<()> {
        validate_entry(id, entry)?;
        let mut conn = self.writer()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision: Option<Option<String>> = tx
            .query_row("SELECT revision FROM boxes WHERE id=?1", [id], |r| r.get(0))
            .optional()?;
        ensure!(
            revision.is_some() && revision.flatten().as_deref() == expected_revision,
            "Report revision conflict"
        );
        insert_entry(&tx, id, entry)?;
        tx.execute(
            "DELETE FROM legacy_documents WHERE name='leaderboard.json'",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn ranked(&self, kind: Option<&str>) -> Result<Vec<Value>> {
        let conn = self.reader()?;
        let mut stmt = conn.prepare(
            "SELECT entry,score FROM boxes WHERE (?1 IS NULL OR kind=?1) ORDER BY score DESC,id_sort",
        )?;
        let rows = stmt.query_map([kind], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
        })?;
        rows.map(|r| {
            let (text, score) = r?;
            let mut entry: Value = serde_json::from_str(&text)?;
            entry["score"] = Value::from(score);
            Ok(entry)
        })
        .collect()
    }
    pub fn gallery(
        &self,
        kind: &str,
        query: &str,
        recent: bool,
        requested: usize,
        size: usize,
    ) -> Result<Value> {
        let mut conn = self.reader()?;
        let tx = conn.transaction()?;
        let counts:(i64,i64,i64)=tx.query_row("SELECT count(*),coalesce(sum(plants),0),coalesce(sum(animals),0) FROM boxes WHERE kind=?1",[kind],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let total: i64 = tx.query_row(
            "SELECT count(*) FROM boxes WHERE kind=?1 AND instr(search,?2)>0",
            params![kind, query.to_lowercase()],
            |r| r.get(0),
        )?;
        let pages = (total as usize).div_ceil(size).max(1);
        let page = requested.min(pages);
        let order = if recent {
            "reported_at DESC,id_sort"
        } else {
            "score DESC,id_sort"
        };
        let sql = format!(
            "WITH ranked AS (SELECT *,row_number() OVER(ORDER BY score DESC,id_sort) rank FROM boxes WHERE kind=?1) SELECT entry,score,rank FROM ranked WHERE instr(search,?2)>0 ORDER BY {order} LIMIT ?3 OFFSET ?4"
        );
        let mut stmt = tx.prepare(&sql)?;
        let entries=stmt.query_map(params![kind,query.to_lowercase(),size as i64,((page-1)*size) as i64],|r|Ok((r.get::<_,String>(0)?,r.get::<_,f64>(1)?,r.get::<_,i64>(2)?)))?.map(|row|->Result<Value>{
            let(text,score,rank)=row?;let entry:Value=serde_json::from_str(&text)?;let state=&entry["state"];
            let fields=if kind=="water" {vec!["name","progression","facilities","plants","kind","oxygen","cleanliness","fish","food"]} else {vec!["name","progression","facilities","plants","soil","seeds","bugs","items","nests","eggs","food","webs"]};
            let compact=fields.into_iter().filter_map(|key|state.get(key).map(|v|(key.to_string(),v.clone()))).collect::<serde_json::Map<_,_>>();
            let mut result=serde_json::json!({"id":entry["id"],"kind":kind,"rank":rank,"score":score,"reportedAt":entry["reportedAt"],"replayCount":state["replays"].as_array().map_or(0,Vec::len),"state":compact});
            if let Some(phase)=entry.get("phase"){result["phase"]=phase.clone();}Ok(result)
        }).collect::<Result<Vec<_>>>()?;
        Ok(
            serde_json::json!({"page":page,"pageSize":size,"pages":pages,"total":total,"counts":{"boxes":counts.0,"plants":counts.1,"animals":counts.2},"entries":entries}),
        )
    }
    pub fn export(&self, output: &Path) -> Result<()> {
        ensure!(!output.exists(), "Export destination already exists");
        let parent = output.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let stage = parent.join(format!(".export-{}", std::process::id()));
        fs::create_dir(&stage)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&stage, fs::Permissions::from_mode(0o700))?;
        }
        let result = (|| -> Result<()> {
            let mut conn = self.reader()?;
            let tx = conn.transaction()?;
            let mut docs: BTreeMap<String, Vec<u8>> = tx
                .prepare("SELECT name,content FROM legacy_documents")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            if !docs.contains_key("leaderboard.json") {
                let boxes: BTreeMap<String, Value> = tx
                    .prepare("SELECT id,entry FROM boxes")?
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                    .map(|r| {
                        let (id, text) = r?;
                        Ok((id, serde_json::from_str::<Value>(&text)?))
                    })
                    .collect::<Result<_>>()?;
                docs.insert("leaderboard.json".into(), serde_json::to_vec(&boxes)?);
            }
            if !docs.contains_key("credentials.json") {
                let credentials: BTreeMap<String, String> = tx
                    .prepare("SELECT id,token FROM credentials")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                docs.insert("credentials.json".into(), serde_json::to_vec(&credentials)?);
            }
            for (name, bytes) in docs {
                fs::write(stage.join(name), bytes)?;
            }
            tx.commit()?;
            fs::rename(&stage, output)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(stage);
        }
        result
    }
    pub fn backup(&self, output: &Path) -> Result<()> {
        ensure!(!output.exists(), "Backup destination already exists");
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(output)?;
        }
        let conn = self.reader()?;
        conn.execute(
            "VACUUM INTO ?1",
            [output.to_str().context("Invalid backup path")?],
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(output, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
    pub fn check(&self) -> Result<()> {
        let result: String = self
            .reader()?
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if result != "ok" {
            bail!("SQLite integrity check failed");
        }
        Ok(())
    }
}
fn insert_entry(conn: &Connection, id: &str, entry: &Value) -> Result<()> {
    let state = &entry["state"];
    let id_sort: String = id
        .chars()
        .map(|c| match c {
            '_' => '!',
            '-' => '"',
            other => other,
        })
        .collect();
    let search = format!("{} {}", state["name"].as_str().unwrap_or(""), id).to_lowercase();
    let plants = state["plants"].as_array().context("Missing plants")?.len();
    let animals = state
        .get("fish")
        .unwrap_or(&state["bugs"])
        .as_array()
        .context("Missing animals")?
        .len();
    conn.execute("INSERT INTO boxes VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(id) DO UPDATE SET kind=excluded.kind,score=excluded.score,reported_at=excluded.reported_at,revision=excluded.revision,entry=excluded.entry,id_sort=excluded.id_sort,search=excluded.search,plants=excluded.plants,animals=excluded.animals",params![id,entry.get("kind").and_then(Value::as_str).unwrap_or("land"),score(state)?,entry["reportedAt"].as_str(),entry["reportRevision"].as_str(),serde_json::to_string(entry)?,id_sort,search,plants as i64,animals as i64])?;
    Ok(())
}

pub fn random_hex(bytes: usize) -> String {
    let mut data = vec![0; bytes];
    getrandom::fill(&mut data).expect("OS random source failed");
    data.iter().map(|b| format!("{b:02x}")).collect()
}
pub(crate) fn document(conn: &Connection, name: &str) -> Result<Option<Value>> {
    let bytes: Option<Vec<u8>> = conn
        .query_row(
            "SELECT content FROM legacy_documents WHERE name=?1",
            [name],
            |r| r.get(0),
        )
        .optional()?;
    bytes
        .map(|b| {
            let value = serde_json::from_slice(&b)?;
            crate::policy::validate_document(name, &value)?;
            Ok(value)
        })
        .transpose()
}
pub(crate) fn save_document(conn: &Connection, name: &str, value: &Value) -> Result<()> {
    conn.execute("INSERT INTO legacy_documents VALUES(?1,?2) ON CONFLICT(name) DO UPDATE SET content=excluded.content",params![name,serde_json::to_vec(value)?])?;
    Ok(())
}
