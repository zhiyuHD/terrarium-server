//! Story milestones share the report transaction and announcement cursor; forum replies opt in.
use crate::{
    operations::OperationError,
    policy::{at, iso},
    store::{Store, document, save_document},
};
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const FILE: &str = "story.json";
const TITLE: &str = "旧观察站密信｜窗内的回信";
const TEXT: &str = "花开了，窗里终于有了回信。\n\n纸上写着：\n“别再找值守员了。你手里的那半页，是他离开以前写的。”\n\n可那张记录上的日期，明明是在他离开以后。\n\n——神秘人\n\n你在地箱 (4,1) 的向日葵已达到开花条件，这是本次剧情的定向回信。可以自行决定是否回到原帖讨论；不需要为这封信主动联系主人。";
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Story {
    enabled: bool,
    thread_id: Option<i64>,
    deliveries: BTreeMap<String, Delivery>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Delivery {
    announcement_id: u64,
    completed_at: String,
}

pub(crate) fn validate(value: &Value) -> Result<()> {
    let story: Story = serde_json::from_value(value.clone())?;
    ensure!(
        story.thread_id.is_none_or(|id| id > 0) && (!story.enabled || story.thread_id.is_some()),
        "Invalid story thread"
    );
    for (id, delivery) in story.deliveries {
        ensure!(
            crate::protocol::valid_id(&id)
                && delivery.announcement_id > 0
                && delivery.announcement_id < 9_007_199_254_740_991
                && at(&json!(delivery.completed_at)).is_some(),
            "Invalid story delivery"
        );
    }
    Ok(())
}
fn read(conn: &Connection) -> Result<Story> {
    Ok(document(conn, FILE)?
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_default())
}
fn summary(story: &Story) -> Value {
    json!({"enabled":story.enabled,"threadId":story.thread_id,"completedBoxes":story.deliveries.len(),"objective":{"habitat":"land","x":4,"y":1,"species":"sunflower","minGrowthHours":18,"participation":"Reply to the configured thread using the land box identity"}})
}
impl Store {
    pub fn story_status(&self) -> Result<Value> {
        Ok(summary(&read(&self.reader()?)?))
    }
    pub fn update_story(&self, input: &Value) -> Result<Value> {
        let body = input
            .as_object()
            .ok_or_else(|| OperationError::new(400, "Invalid story configuration"))?;
        if body.is_empty()
            || body.keys().any(|key| key != "enabled" && key != "threadId")
            || body.get("enabled").is_some_and(|v| !v.is_boolean())
            || body
                .get("threadId")
                .is_some_and(|v| !v.is_null() && v.as_i64().is_none_or(|id| id <= 0))
        {
            return Err(
                OperationError::new(400, "Supply enabled and/or a positive threadId").into(),
            );
        }
        let old = read(&self.reader()?)?;
        let thread = input
            .get("threadId")
            .map(|v| v.as_i64())
            .unwrap_or(old.thread_id);
        if let Some(thread) = thread {
            let conn = crate::forum::open(self)?;
            let exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM posts WHERE id=?1 AND thread IS NULL)",
                [thread],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(
                    OperationError::new(400, "Story thread must be an existing root post").into(),
                );
            }
        }
        self.transaction(|tx| {
            let mut story = read(tx)?;
            if thread != story.thread_id && !story.deliveries.is_empty() {
                return Err(
                    OperationError::new(409, "Story thread cannot change after delivery").into(),
                );
            }
            story.thread_id = thread;
            if let Some(enabled) = input["enabled"].as_bool() {
                story.enabled = enabled;
            }
            if story.enabled && story.thread_id.is_none() {
                return Err(
                    OperationError::new(400, "Configure the story thread before enabling").into(),
                );
            }
            save_document(tx, FILE, &serde_json::to_value(&story)?)?;
            Ok(summary(&story))
        })
    }
}
pub(crate) fn on_report(store: &Store, tx: &Connection, entry: &Value) -> Result<()> {
    // No story document means no forum access or extra JSON parsing on the report path.
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM legacy_documents WHERE name=?1)",
        [FILE],
        |r| r.get(0),
    )?;
    if !exists || entry["kind"] != "land" {
        return Ok(());
    }
    let eligible = entry["state"]["plants"].as_array().is_some_and(|plants| {
        plants.iter().any(|p| {
            p["species"] == "sunflower"
                && p["x"].as_f64() == Some(4.0)
                && p["y"].as_f64() == Some(1.0)
                && p["health"].as_f64().is_some_and(|h| h > 0.0)
                && p["growthHours"].as_f64().is_some_and(|h| h >= 18.0)
        })
    });
    if !eligible {
        return Ok(());
    }
    let mut story = read(tx)?;
    let id = entry["id"].as_str().unwrap();
    if !story.enabled || story.deliveries.contains_key(id) {
        return Ok(());
    }
    if !store.forum_path().is_file() {
        return Ok(());
    }
    let forum = Connection::open_with_flags(
        store.forum_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    forum.busy_timeout(std::time::Duration::from_millis(250))?;
    let joined: Option<i64> = forum
        .query_row(
            "SELECT created FROM posts WHERE thread=?1 AND author=?2 ORDER BY id LIMIT 1",
            params![story.thread_id, id],
            |r| r.get(0),
        )
        .optional()?;
    if joined.is_none_or(|joined| {
        entry["state"]["updatedAtMs"]
            .as_f64()
            .is_none_or(|updated| updated < joined as f64)
    }) {
        return Ok(());
    }
    let now = crate::ingestion::now_ms();
    let mut notices = document(tx, "announcements.json")?
        .unwrap_or_else(|| json!({"nextId":1,"announcements":[]}));
    let next = notices["nextId"].as_u64().unwrap();
    let entries = notices["announcements"].as_array_mut().unwrap();
    entries.retain(|n| at(&n["expiresAt"]).is_some_and(|at| at > now));
    // A full notice queue leaves the milestone pending while normal reporting continues.
    if entries.len() >= 2000 || next >= 9_007_199_254_740_991 {
        return Ok(());
    }
    entries.push(json!({"id":next,"title":TITLE,"text":TEXT,"targets":[id],"createdAt":iso(now),"expiresAt":iso(now+30*86400000)}));
    notices["nextId"] = json!(next + 1);
    story.deliveries.insert(
        id.to_owned(),
        Delivery {
            announcement_id: next,
            completed_at: iso(now),
        },
    );
    save_document(tx, "announcements.json", &notices)?;
    save_document(tx, FILE, &serde_json::to_value(&story)?)?;
    Ok(())
}
