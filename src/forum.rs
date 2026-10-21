//! Forum data is an independent SQLite sidecar; ecosystem rollback leaves it intact.
use crate::{
    http::App, ingestion::now_ms, operations::OperationError, policy::iso, push::bearer,
    store::Store,
};
use anyhow::{Result, bail};
use axum::{
    Json,
    body::to_bytes,
    extract::{Path, Query, Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap},
    time::Duration,
};
use tokio::sync::Semaphore;
static REQUESTS: Semaphore = Semaphore::const_new(12);
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS posts(id INTEGER PRIMARY KEY AUTOINCREMENT,thread INTEGER REFERENCES posts(id),author TEXT NOT NULL,title TEXT NOT NULL,body TEXT NOT NULL,created INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS forum_threads ON posts(thread,id DESC);
CREATE INDEX IF NOT EXISTS forum_authors ON posts(author,thread,id DESC);
CREATE TABLE IF NOT EXISTS likes(post INTEGER NOT NULL REFERENCES posts(id),author TEXT NOT NULL,PRIMARY KEY(post,author));
CREATE INDEX IF NOT EXISTS forum_likes_author ON likes(author);
CREATE TABLE IF NOT EXISTS follows(author TEXT NOT NULL,target TEXT NOT NULL,PRIMARY KEY(author,target));
CREATE INDEX IF NOT EXISTS forum_followers ON follows(target);
CREATE TABLE IF NOT EXISTS activity(author TEXT NOT NULL,action TEXT NOT NULL,at INTEGER NOT NULL,PRIMARY KEY(author,action));
CREATE VIRTUAL TABLE IF NOT EXISTS post_search USING fts5(title,body,content='posts',content_rowid='id',tokenize='trigram');
CREATE TRIGGER IF NOT EXISTS post_insert AFTER INSERT ON posts BEGIN INSERT INTO post_search(rowid,title,body) VALUES(new.id,new.title,new.body); END;
PRAGMA user_version=1;";
fn reject<T>(status: u16, message: &str) -> Result<T> {
    bail!(OperationError::new(status, message))
}
pub fn open(store: &Store) -> Result<Connection> {
    let path = store.forum_path();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&path)?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA cache_size=-1024; PRAGMA mmap_size=0; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA max_page_count=65536;")?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    anyhow::ensure!(version <= 1, "Forum schema is newer than this binary");
    if version == 0 {
        conn.execute_batch(SCHEMA)?;
    }
    Ok(conn)
}
fn enabled(store: &Store, now: i64) -> Result<bool> {
    Ok(store.controls(now)?["forum"]["enabled"] == true)
}
pub async fn status(State(app): State<App>) -> Response {
    match crate::ingestion::database(app.store, |s| {
        Ok(json!({"enabled":enabled(&s,now_ms())?,"message":if enabled(&s,now_ms())? {Value::Null}else{json!("未开放论坛")}}))
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(e.body())).into_response(),
    }
}
pub async fn handle(
    State(app): State<App>,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    request: Request,
) -> Response {
    let Ok(_permit) = REQUESTS.try_acquire() else {
        return response_error(429, "Forum is busy; retry later");
    };
    let method = request.method().clone();
    let token = bearer(request.headers());
    let input = if method == Method::GET {
        Value::Null
    } else {
        if app.ingestion.is_none() {
            return response_error(501, "Write preview is disabled");
        }
        let bytes = match to_bytes(request.into_body(), 16384).await {
            Ok(b) => b,
            Err(_) => return response_error(413, "Forum request too large"),
        };
        match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(_) => return response_error(400, "Invalid JSON"),
        }
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = _permit;
        execute(&app.store, &method, &path, &query, &input, &token, now_ms())
    })
    .await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => {
            if let Some(e) = e.downcast_ref::<OperationError>() {
                response_error(e.status, &e.message)
            } else {
                response_error(500, "Forum operation failed")
            }
        }
        Err(_) => response_error(500, "Forum task failed"),
    }
}
fn response_error(status: u16, message: &str) -> Response {
    (StatusCode::from_u16(status).unwrap(),Json(json!({"error":message,"code":if message=="未开放论坛" {"forum_closed"}else{"forum_error"}}))).into_response()
}
fn number(query: &HashMap<String, String>, key: &str, default: i64, max: i64) -> Result<i64> {
    let n = query
        .get(key)
        .map(|s| s.parse::<i64>())
        .transpose()
        .map_err(|_| OperationError::new(400, "Invalid pagination"))?
        .unwrap_or(default);
    if n < 0 || n > max {
        return reject(400, "Invalid pagination");
    }
    Ok(n)
}
fn text(input: &Value, key: &str, max: usize) -> Result<String> {
    let Some(s) = input[key].as_str().map(str::trim).filter(|s| {
        !s.is_empty()
            && s.encode_utf16().count() <= max
            && !s
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    }) else {
        return reject(400, "Invalid forum text");
    };
    Ok(s.to_owned())
}
fn post(conn: &Connection, id: i64) -> Result<Option<Value>> {
    Ok(conn.query_row("SELECT p.id,p.thread,p.author,p.title,p.body,p.created,(SELECT count(*) FROM likes WHERE post=p.id),(SELECT count(*) FROM posts WHERE thread=p.id) FROM posts p WHERE p.id=?1",[id],row).optional()?)
}
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":r.get::<_,i64>(0)?,"thread":r.get::<_,Option<i64>>(1)?,"author":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,"body":r.get::<_,String>(4)?,"createdAt":iso(r.get(5)?),"likes":r.get::<_,i64>(6)?,"replies":r.get::<_,i64>(7)?}),
    )
}
fn rate(conn: &Connection, author: &str, action: &str, now: i64, interval: i64) -> Result<()> {
    let last: Option<i64> = conn
        .query_row(
            "SELECT at FROM activity WHERE author=?1 AND action=?2",
            params![author, action],
            |r| r.get(0),
        )
        .optional()?;
    if last.is_some_and(|last| now - last < interval) {
        return reject(429, "Forum rate limit; retry later");
    }
    conn.execute("INSERT INTO activity VALUES(?1,?2,?3) ON CONFLICT(author,action) DO UPDATE SET at=excluded.at",params![author,action,now])?;
    Ok(())
}
pub fn execute(
    store: &Store,
    method: &Method,
    path: &str,
    query: &HashMap<String, String>,
    input: &Value,
    token: &str,
    now: i64,
) -> Result<Value> {
    let mut value = execute_raw(store, method, path, query, input, token, now)?;
    display_names(store, &mut value)?;
    Ok(value)
}

fn display_names(store: &Store, value: &mut Value) -> Result<()> {
    let mut ids = BTreeSet::new();
    fn collect(value: &Value, ids: &mut BTreeSet<String>) {
        if let Some(author) = value["author"].as_str() {
            ids.insert(author.to_owned());
        }
        if let Some(thread) = value.get("thread").filter(|v| v.is_object()) {
            collect(thread, ids);
        }
        for key in ["threads", "replies"] {
            if let Some(posts) = value[key].as_array() {
                for post in posts {
                    collect(post, ids);
                }
            }
        }
    }
    collect(value, &mut ids);
    if let Some(following) = value["following"].as_array() {
        if let Some(id) = value["id"].as_str() {
            ids.insert(id.to_owned());
        }
        for id in following.iter().filter_map(Value::as_str) {
            ids.insert(id.to_owned());
        }
    }
    if ids.is_empty() {
        return Ok(());
    }
    let conn = store.reader()?;
    let sql = format!(
        "SELECT id,json_extract(entry,'$.state.name'),kind FROM boxes WHERE id IN ({})",
        vec!["?"; ids.len()].join(",")
    );
    let mut stmt = conn.prepare(&sql)?;
    let names: HashMap<String, String> = stmt
        .query_map(rusqlite::params_from_iter(ids.iter()), |r| {
            let name: Option<String> = r.get(1)?;
            let kind: String = r.get(2)?;
            Ok((
                r.get(0)?,
                name.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
                    if kind == "water" {
                        "未命名水箱"
                    } else {
                        "未命名地箱"
                    }
                    .to_owned()
                }),
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    fn name(names: &HashMap<String, String>, id: &str) -> String {
        names
            .get(id)
            .cloned()
            .unwrap_or_else(|| "未命名的生态箱".to_owned())
    }
    fn decorate(value: &mut Value, names: &HashMap<String, String>) {
        if let Some(author) = value["author"].as_str() {
            value["authorName"] = json!(name(names, author));
        }
        if let Some(thread) = value.get_mut("thread").filter(|v| v.is_object()) {
            decorate(thread, names);
        }
        for key in ["threads", "replies"] {
            if let Some(posts) = value.get_mut(key).and_then(Value::as_array_mut) {
                for post in posts {
                    decorate(post, names);
                }
            }
        }
    }
    decorate(value, &names);
    if let Some(following) = value["following"].as_array() {
        let profiles: Vec<_> = following
            .iter()
            .filter_map(Value::as_str)
            .map(|id| json!({"id":id,"name":name(&names,id)}))
            .collect();
        let own = name(&names, value["id"].as_str().unwrap());
        value["name"] = json!(own);
        value["followingProfiles"] = json!(profiles);
    }
    Ok(())
}

fn execute_raw(
    store: &Store,
    method: &Method,
    path: &str,
    query: &HashMap<String, String>,
    input: &Value,
    token: &str,
    now: i64,
) -> Result<Value> {
    if !enabled(store, now)? {
        return reject(503, "未开放论坛");
    }
    let mut conn = open(store)?;
    if method == Method::GET {
        let limit = number(query, "limit", 5, 20)?.max(1);
        let before = number(query, "before", i64::MAX, i64::MAX)?;
        if path == "threads" {
            let q = query.get("q").map(|s| s.trim()).unwrap_or("");
            if q.encode_utf16().count() > 100 {
                return reject(400, "Search query too long");
            }
            let author = query.get("author").map(String::as_str).unwrap_or("");
            let following = query.get("followingOf").map(String::as_str).unwrap_or("");
            let fts = q.chars().count() >= 3;
            let term = format!("\"{}\"", q.replace('"', "\"\""));
            let sql = format!(
                "SELECT p.id,NULL,p.author,p.title,substr(p.body,1,240),p.created,(SELECT count(*) FROM likes WHERE post=p.id),(SELECT count(*) FROM posts WHERE thread=p.id) FROM posts p WHERE p.thread IS NULL AND p.id<?1 AND (?2='' OR p.author=?2) AND (?3='' OR p.author IN(SELECT target FROM follows WHERE author=?3)) AND {} ORDER BY p.id DESC LIMIT ?5",
                if fts {
                    "p.id IN(SELECT coalesce(found.thread,found.id) FROM post_search JOIN posts found ON found.id=post_search.rowid WHERE post_search MATCH ?4)"
                } else {
                    "(?4='' OR p.id IN(SELECT coalesce(thread,id) FROM posts WHERE instr(lower(title||' '||body),lower(?4))>0))"
                }
            );
            let mut stmt = conn.prepare(&sql)?;
            let mut rows: Vec<Value> = stmt
                .query_map(
                    params![
                        before,
                        author,
                        following,
                        if fts { &term } else { q },
                        limit + 1
                    ],
                    row,
                )?
                .collect::<rusqlite::Result<_>>()?;
            let more = rows.len() > limit as usize;
            rows.truncate(limit as usize);
            return Ok(
                json!({"threads":rows,"nextCursor":if more {rows.last().unwrap()["id"].clone()}else{Value::Null},"externalContent":true}),
            );
        }
        if let Some(id) = path
            .strip_prefix("threads/")
            .and_then(|s| s.parse::<i64>().ok())
        {
            let root = post(&conn, id)?
                .filter(|p| p["thread"].is_null())
                .ok_or_else(|| OperationError::new(404, "Thread not found"))?;
            let mut stmt=conn.prepare("SELECT p.id,p.thread,p.author,p.title,p.body,p.created,(SELECT count(*) FROM likes WHERE post=p.id),0 FROM posts p WHERE thread=?1 AND id<?2 ORDER BY id DESC LIMIT ?3")?;
            let mut rows: Vec<Value> = stmt
                .query_map(params![id, before, limit + 1], row)?
                .collect::<rusqlite::Result<_>>()?;
            let more = rows.len() > limit as usize;
            rows.truncate(limit as usize);
            return Ok(
                json!({"thread":root,"replies":rows,"nextCursor":if more {rows.last().unwrap()["id"].clone()}else{Value::Null},"externalContent":true}),
            );
        }
        if let Some(id) = path
            .strip_prefix("profiles/")
            .filter(|s| crate::protocol::valid_id(s))
        {
            let exists: bool = store.reader()?.query_row(
                "SELECT EXISTS(SELECT 1 FROM credentials WHERE id=?1)",
                [id],
                |r| r.get(0),
            )?;
            if !exists {
                return reject(404, "Unknown box");
            }
            let counts:(i64,i64,i64)=conn.query_row("SELECT (SELECT count(*) FROM posts WHERE author=?1 AND thread IS NULL),(SELECT count(*) FROM follows WHERE author=?1),(SELECT count(*) FROM follows WHERE target=?1)",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            let mut stmt = conn.prepare(
                "SELECT target FROM follows WHERE author=?1 AND target>?2 ORDER BY target LIMIT ?3",
            )?;
            let after = query.get("after").map(String::as_str).unwrap_or("");
            let mut following: Vec<String> = stmt
                .query_map(params![id, after, limit + 1], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let more = following.len() > limit as usize;
            following.truncate(limit as usize);
            return Ok(
                json!({"id":id,"threads":counts.0,"followingCount":counts.1,"followers":counts.2,"nextCursor":if more {following.last().cloned()}else{None},"following":following}),
            );
        }
        return reject(404, "Forum route not found");
    }
    let author = input["id"]
        .as_str()
        .filter(|s| crate::protocol::valid_id(s))
        .ok_or_else(|| OperationError::new(401, "Claim a box before posting"))?;
    if !store.authenticated(author, token)? {
        return reject(401, "Unauthorized");
    }
    if !crate::protocol::valid_version(&input["clientVersion"]) {
        return reject(400, "Client version required");
    }
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let result = if method == Method::POST && (path == "threads" || path.ends_with("/replies")) {
        let thread = if path == "threads" {
            None
        } else {
            path.strip_prefix("threads/")
                .and_then(|s| s.strip_suffix("/replies"))
                .and_then(|s| s.parse::<i64>().ok())
        };
        if path != "threads" && thread.is_none() {
            return reject(404, "Forum route not found");
        }
        if let Some(id) = thread
            && post(&tx, id)?.is_none_or(|p| !p["thread"].is_null())
        {
            return reject(404, "Thread not found");
        }
        let title = if thread.is_none() {
            text(input, "title", 120)?
        } else {
            String::new()
        };
        let body = text(input, "body", if thread.is_none() { 4000 } else { 2000 })?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM posts WHERE author=?1 AND (thread IS NULL)=?2",
            params![author, thread.is_none()],
            |r| r.get(0),
        )?;
        if count >= if thread.is_none() { 500 } else { 5000 } {
            return reject(429, "Author post capacity reached");
        }
        let total: i64 = tx
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name='posts'",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if total >= 100_000 {
            return reject(507, "Forum capacity reached");
        }
        rate(
            &tx,
            author,
            if thread.is_none() { "thread" } else { "reply" },
            now,
            if thread.is_none() { 60_000 } else { 10_000 },
        )?;
        tx.execute(
            "INSERT INTO posts(thread,author,title,body,created) VALUES(?1,?2,?3,?4,?5)",
            params![thread, author, title, body, now],
        )?;
        post(&tx, tx.last_insert_rowid())?.unwrap()
    } else if method == Method::PUT && path == "likes" {
        let id = input["postId"]
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or_else(|| OperationError::new(400, "Invalid post ID"))?;
        let liked = input["liked"]
            .as_bool()
            .ok_or_else(|| OperationError::new(400, "Supply liked"))?;
        if post(&tx, id)?.is_none() {
            return reject(404, "Post not found");
        }
        let current: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM likes WHERE post=?1 AND author=?2)",
            params![id, author],
            |r| r.get(0),
        )?;
        if current != liked {
            rate(&tx, author, "like", now, 1000)?;
            if liked {
                let count: i64 = tx.query_row(
                    "SELECT count(*) FROM likes WHERE author=?1",
                    [author],
                    |r| r.get(0),
                )?;
                if count >= 5000 {
                    return reject(429, "Like capacity reached");
                }
                tx.execute("INSERT INTO likes VALUES(?1,?2)", params![id, author])?;
            } else {
                tx.execute(
                    "DELETE FROM likes WHERE post=?1 AND author=?2",
                    params![id, author],
                )?;
            }
        }
        json!({"postId":id,"liked":liked,"likes":post(&tx,id)?.unwrap()["likes"]})
    } else if method == Method::PUT && path == "follows" {
        let target = input["targetId"]
            .as_str()
            .filter(|s| crate::protocol::valid_id(s) && *s != author)
            .ok_or_else(|| OperationError::new(400, "Invalid follow target"))?;
        let following = input["following"]
            .as_bool()
            .ok_or_else(|| OperationError::new(400, "Supply following"))?;
        let exists: bool = store.reader()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM credentials WHERE id=?1)",
            [target],
            |r| r.get(0),
        )?;
        if !exists {
            return reject(404, "Unknown box");
        }
        let current: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM follows WHERE author=?1 AND target=?2)",
            params![author, target],
            |r| r.get(0),
        )?;
        if current != following {
            rate(&tx, author, "follow", now, 1000)?;
            if following {
                let count: i64 = tx.query_row(
                    "SELECT count(*) FROM follows WHERE author=?1",
                    [author],
                    |r| r.get(0),
                )?;
                if count >= 500 {
                    return reject(429, "Follow capacity reached");
                }
                tx.execute("INSERT INTO follows VALUES(?1,?2)", params![author, target])?;
            } else {
                tx.execute(
                    "DELETE FROM follows WHERE author=?1 AND target=?2",
                    params![author, target],
                )?;
            }
        }
        json!({"targetId":target,"following":following})
    } else {
        return reject(405, "Method not allowed");
    };
    tx.commit()?;
    Ok(result)
}

pub fn backup(store: &Store, output: &std::path::Path) -> Result<()> {
    anyhow::ensure!(!output.exists(), "Forum backup target exists");
    let conn = open(store)?;
    conn.execute(
        "VACUUM INTO ?1",
        [output
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid backup path"))?],
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(output, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
