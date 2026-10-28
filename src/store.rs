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
    "story.json",
    "story-granary.json",
    "story-seedbed.json",
    "admin-token",
    "neighbors.json",
];
#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    writer: Arc<Mutex<Connection>>,
}
pub(crate) struct ReportWrite {
    pub entry: Value,
    pub expected: Option<Option<String>>,
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
        let mut conn = Connection::open(&path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        ensure!(version <= 1, "Database schema is newer than this binary");
        conn.execute_batch("CREATE TABLE IF NOT EXISTS boxes(id TEXT PRIMARY KEY,kind TEXT NOT NULL,score REAL NOT NULL,reported_at TEXT NOT NULL,revision TEXT,entry TEXT NOT NULL,id_sort TEXT NOT NULL,search TEXT NOT NULL,plants INTEGER NOT NULL,animals INTEGER NOT NULL); CREATE INDEX IF NOT EXISTS boxes_counts ON boxes(kind,plants,animals); CREATE INDEX IF NOT EXISTS boxes_gallery_metadata ON boxes(kind,score DESC,id_sort,id,reported_at,search); CREATE INDEX IF NOT EXISTS boxes_global_rank ON boxes(score DESC,id_sort); CREATE INDEX IF NOT EXISTS boxes_rank ON boxes(kind,score DESC,id_sort); CREATE INDEX IF NOT EXISTS boxes_recent ON boxes(kind,reported_at DESC,id_sort); CREATE TABLE IF NOT EXISTS credentials(id TEXT PRIMARY KEY,token TEXT NOT NULL); CREATE TABLE IF NOT EXISTS legacy_documents(name TEXT PRIMARY KEY,content BLOB NOT NULL); PRAGMA user_version=1;")?;
        crate::summary_cache::initialize(&mut conn)?;
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
    pub fn photo_dir(&self) -> PathBuf {
        self.path.with_extension("photos")
    }
    pub fn forum_path(&self) -> PathBuf {
        self.path.with_extension("forum.sqlite")
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
        let serialized = serde_json::to_string(entry)?;
        let expected = previous.map(|e| e["reportRevision"].as_str().map(str::to_owned));
        self.transaction(|tx| self.commit_prepared(tx, entry, &expected, &serialized))
    }
    fn commit_prepared(
        &self,
        tx: &Connection,
        entry: &Value,
        expected: &Option<Option<String>>,
        serialized: &str,
    ) -> Result<bool> {
        let id = entry["id"].as_str().context("Invalid ID")?;
        let current: Option<Option<String>> = tx
            .query_row("SELECT revision FROM boxes WHERE id=?1", [id], |r| r.get(0))
            .optional()?;
        if current != *expected {
            return Ok(false);
        }
        insert_serialized_entry(tx, id, entry, serialized)?;
        crate::story::on_report(self, tx, entry)?;
        tx.execute(
            "DELETE FROM legacy_documents WHERE name='leaderboard.json'",
            [],
        )?;
        Ok(true)
    }
    pub(crate) fn commit_reports(
        &self,
        reports: &[ReportWrite],
    ) -> Result<Vec<Result<bool, String>>> {
        let prepared: Vec<_> = reports
            .iter()
            .map(|report| -> Result<String, String> {
                let id = report.entry["id"]
                    .as_str()
                    .ok_or_else(|| "Invalid ID".to_owned())?;
                validate_entry(id, &report.entry).map_err(|e| e.to_string())?;
                serde_json::to_string(&report.entry).map_err(|e| e.to_string())
            })
            .collect();
        self.transaction(|tx| {
            let mut results = Vec::with_capacity(reports.len());
            for (report, serialized) in reports.iter().zip(&prepared) {
                let serialized = match serialized {
                    Ok(text) => text,
                    Err(error) => {
                        results.push(Err(error.clone()));
                        continue;
                    }
                };
                tx.execute_batch("SAVEPOINT report_write")?;
                let result = self.commit_prepared(tx, &report.entry, &report.expected, serialized);
                if result.is_err() {
                    tx.execute_batch("ROLLBACK TO report_write")?;
                }
                tx.execute_batch("RELEASE report_write")?;
                results.push(result.map_err(|e| e.to_string()));
            }
            Ok(results)
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
        let serialized = serde_json::to_string(entry)?;
        let mut conn = self.writer()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision: Option<Option<String>> = tx
            .query_row("SELECT revision FROM boxes WHERE id=?1", [id], |r| r.get(0))
            .optional()?;
        ensure!(
            revision.is_some() && revision.flatten().as_deref() == expected_revision,
            "Report revision conflict"
        );
        insert_serialized_entry(&tx, id, entry, &serialized)?;
        tx.execute(
            "DELETE FROM legacy_documents WHERE name='leaderboard.json'",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn ranked(&self, kind: Option<&str>) -> Result<Vec<Value>> {
        let conn = self.reader()?;
        let mut stmt = conn.prepare(if kind.is_some() {
            "SELECT entry,score FROM boxes WHERE kind=?1 ORDER BY score DESC,id_sort LIMIT 100"
        } else {
            "SELECT entry,score FROM boxes ORDER BY score DESC,id_sort LIMIT 100"
        })?;
        let rows = stmt.query_map(rusqlite::params_from_iter(kind), |r| {
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
    /// Wallet ranking is separate from ecosystem score and uses the last accepted snapshot.
    pub fn coins(&self, limit: usize, own: Option<&str>) -> Result<Value> {
        let conn = self.reader()?;
        let mut stmt = conn.prepare(
            "WITH wallets AS (
            SELECT id,id_sort,reported_at,name,earned,earned-spent coins
            FROM box_summaries WHERE kind='land' AND earned>0
        ), ranks AS (SELECT *,row_number() OVER(ORDER BY coins DESC,id_sort) position,
            rank() OVER(ORDER BY coins DESC) competitive,count(*) OVER() total FROM wallets)
        SELECT id,name,reported_at,earned,coins,position,competitive,total FROM ranks
        WHERE position<=?1 OR id=?2 ORDER BY position",
        )?;
        let rows = stmt.query_map(params![limit as i64, own], |r| {
            Ok((serde_json::json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,Option<String>>(1)?,
                "reportedAt":r.get::<_,String>(2)?,"earned":r.get::<_,i64>(3)?,"coins":r.get::<_,i64>(4)?,
                "rank":r.get::<_,i64>(5)?,"competitiveRank":r.get::<_,i64>(6)?}),r.get::<_,i64>(7)?))
        })?;
        let mut entries = Vec::new();
        let mut mine = Value::Null;
        let mut total = 0;
        for row in rows {
            let (mut entry, count) = row?;
            total = count;
            if entry["name"].is_null() {
                entry.as_object_mut().unwrap().remove("name");
            }
            if own.is_some_and(|id| entry["id"] == id) {
                mine = entry.clone();
            }
            if entry["rank"].as_u64().unwrap() <= limit as u64 {
                entries.push(entry);
            }
        }
        Ok(
            serde_json::json!({"habitat":"land","metric":"coins","total":total,"entries":entries,"own":mine}),
        )
    }
    pub fn summary(&self, kind: &str, limit: usize, own: Option<&str>) -> Result<Value> {
        let mut conn = self.reader()?;
        let tx = conn.transaction()?;
        let total: i64 = tx.query_row("SELECT count(*) FROM boxes WHERE kind=?1", [kind], |r| {
            r.get(0)
        })?;
        let mut stmt=tx.prepare("SELECT b.id,b.score,s.name,s.parts,b.reported_at FROM boxes b JOIN box_summaries s ON s.id=b.id WHERE b.kind=?1 ORDER BY b.score DESC,b.id_sort LIMIT ?2")?;
        let mut entries = Vec::new();
        let mut mine = Value::Null;
        let mut competitive = 0;
        let mut last_score = None;
        let mut rows = stmt.query(params![kind, limit as i64])?;
        while let Some(row) = rows.next()? {
            let rank = entries.len() + 1;
            let score = row.get::<_, f64>(1)?;
            if last_score != Some(score) {
                competitive = rank;
                last_score = Some(score);
            }
            let mut summary = serde_json::json!({"rank":rank,"competitiveRank":competitive,"id":row.get::<_,String>(0)?,"score":score,"scoreParts":serde_json::from_str::<Value>(&row.get::<_,String>(3)?)?,"reportedAt":row.get::<_,String>(4)?});
            if let Some(name) = row.get::<_, Option<String>>(2)? {
                summary["name"] = Value::from(name);
            }
            if own.is_some_and(|id| summary["id"] == id) {
                mine = summary.clone();
            }
            entries.push(summary);
        }
        if mine.is_null()
            && let Some(id) = own
        {
            let own_row: Option<(f64,String,Option<String>,String,String)> = tx.query_row("SELECT b.score,s.id_sort,s.name,s.parts,b.reported_at FROM boxes b JOIN box_summaries s ON s.id=b.id WHERE b.kind=?1 AND b.id=?2",params![kind,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
            if let Some((score, id_sort, name, parts, reported)) = own_row {
                let higher: i64 = tx.query_row(
                    "SELECT count(*) FROM boxes WHERE kind=?1 AND score>?2",
                    params![kind, score],
                    |r| r.get(0),
                )?;
                let ties: i64 = tx.query_row(
                    "SELECT count(*) FROM boxes WHERE kind=?1 AND score=?2 AND id_sort<?3",
                    params![kind, score, id_sort],
                    |r| r.get(0),
                )?;
                mine = serde_json::json!({"rank":higher+ties+1,"competitiveRank":higher+1,"id":id,"score":score,"scoreParts":serde_json::from_str::<Value>(&parts)?,"reportedAt":reported});
                if let Some(name) = name {
                    mine["name"] = Value::from(name);
                }
            }
        }
        Ok(serde_json::json!({"habitat":kind,"total":total,"entries":entries,"own":mine}))
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
        let total: i64 = if query.is_empty() {
            counts.0
        } else {
            tx.query_row(
            "SELECT count(*) FROM boxes INDEXED BY boxes_gallery_metadata WHERE kind=?1 AND instr(search,?2)>0",
            params![kind, query.to_lowercase()],
            |r| r.get(0),
        )?
        };
        let pages = (total as usize).div_ceil(size).max(1);
        let page = requested.min(pages);
        let order = if recent {
            "reported_at DESC,id_sort"
        } else {
            "score DESC,id_sort"
        };
        let sql = format!(
            "WITH ranked AS (SELECT id,score,id_sort,reported_at,search,row_number() OVER(ORDER BY score DESC,id_sort) rank FROM boxes INDEXED BY boxes_gallery_metadata WHERE kind=?1), page AS MATERIALIZED (SELECT id,score,rank,row_number() OVER(ORDER BY {order}) page_order FROM ranked WHERE instr(search,?2)>0 ORDER BY {order} LIMIT ?3 OFFSET ?4) SELECT boxes.entry,page.score,page.rank FROM page JOIN boxes ON boxes.id=page.id ORDER BY page.page_order"
        );
        let mut stmt = tx.prepare(&sql)?;
        let entries = stmt
            .query_map(
                params![
                    kind,
                    query.to_lowercase(),
                    size as i64,
                    ((page - 1) * size) as i64
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, f64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )?
            .map(|row| -> Result<Value> {
                let (text, score, rank) = row?;
                let entry: Value = serde_json::from_str(&text)?;
                let mut result = scene_entry(&entry);
                result["rank"] = serde_json::json!(rank);
                result["score"] = serde_json::json!(score);
                Ok(result)
            })
            .collect::<Result<Vec<_>>>()?;
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
    let serialized = serde_json::to_string(entry)?;
    insert_serialized_entry(conn, id, entry, &serialized)
}
fn insert_serialized_entry(
    conn: &Connection,
    id: &str,
    entry: &Value,
    serialized: &str,
) -> Result<()> {
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
    conn.execute("INSERT INTO boxes VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(id) DO UPDATE SET kind=excluded.kind,score=excluded.score,reported_at=excluded.reported_at,revision=excluded.revision,entry=excluded.entry,id_sort=excluded.id_sort,search=excluded.search,plants=excluded.plants,animals=excluded.animals",params![id,entry.get("kind").and_then(Value::as_str).unwrap_or("land"),score(state)?,entry["reportedAt"].as_str(),entry["reportRevision"].as_str(),serialized,id_sort,search,plants as i64,animals as i64])?;
    crate::summary_cache::write(conn, id, entry, &id_sort)?;
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

/// Public rendering fields exclude history and replay payloads.
pub(crate) fn scene_entry(entry: &Value) -> Value {
    let state = &entry["state"];
    let water = entry["kind"] == "water";
    let fields = if water {
        vec![
            "name",
            "progression",
            "facilities",
            "plants",
            "kind",
            "oxygen",
            "cleanliness",
            "fish",
            "eggs",
            "food",
        ]
    } else {
        vec![
            "name",
            "progression",
            "facilities",
            "plants",
            "soil",
            "economy",
            "decorations",
            "litter",
            "seeds",
            "bugs",
            "items",
            "nests",
            "eggs",
            "food",
            "webs",
        ]
    };
    let compact = fields
        .into_iter()
        .filter_map(|key| state.get(key).map(|v| (key.to_string(), v.clone())))
        .collect::<serde_json::Map<_, _>>();
    let mut result = serde_json::json!({"id":entry["id"],"kind":entry["kind"],"score":entry["score"],"reportedAt":entry["reportedAt"],"replayCount":state["replays"].as_array().map_or(0,Vec::len),"state":compact});
    if let Some(phase) = entry.get("phase") {
        result["phase"] = phase.clone();
    }
    result
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    use serde_json::json;
    fn report(id: &str) -> ReportWrite {
        let state: Value =
            serde_json::from_str::<Value>(include_str!("../tests/fixtures/reports.json")).unwrap()
                ["land"]
                .clone();
        ReportWrite {
            entry: json!({"id":id,"kind":"land","state":state,"phase":"day","reportedAt":"2026-10-28T00:00:00Z","reportRevision":"fixture"}),
            expected: None,
        }
    }
    #[test]
    fn failed_item_rolls_back_its_snapshot_and_cache_without_rejecting_other_items() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = Store::open(dir.path().join("data.sqlite")).unwrap();
        store.writer().unwrap().execute_batch("CREATE TRIGGER reject_one BEFORE INSERT ON box_summaries WHEN new.id='garden-bad' BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
        let result = store
            .commit_reports(&[
                report("garden-first"),
                report("garden-bad"),
                report("garden-last"),
            ])
            .unwrap();
        assert!(matches!(result[0], Ok(true)));
        assert!(result[1].is_err());
        assert!(matches!(result[2], Ok(true)));
        assert!(store.get("garden-bad").unwrap().is_none());
        assert!(store.get("garden-first").unwrap().is_some());
        assert!(store.get("garden-last").unwrap().is_some());
        assert_eq!(store.summary("land", 20, None).unwrap()["total"], 2);
    }
    #[test]
    fn a_deferred_commit_failure_rejects_the_whole_batch_and_persists_no_snapshots() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = Store::open(dir.path().join("data.sqlite")).unwrap();
        store.writer().unwrap().execute_batch("PRAGMA foreign_keys=ON;CREATE TABLE fixture_parent(id INTEGER PRIMARY KEY);CREATE TABLE fixture_guard(id TEXT,missing INTEGER REFERENCES fixture_parent(id) DEFERRABLE INITIALLY DEFERRED);CREATE TRIGGER fail_commit AFTER INSERT ON boxes BEGIN INSERT INTO fixture_guard VALUES(new.id,1);END;").unwrap();
        assert!(
            store
                .commit_reports(&[report("garden-first"), report("garden-last")])
                .is_err()
        );
        assert!(store.get("garden-first").unwrap().is_none());
        assert!(store.get("garden-last").unwrap().is_none());
        assert_eq!(store.summary("land", 20, None).unwrap()["total"], 0);
    }
}
