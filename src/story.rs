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

#[derive(Clone, Copy)]
enum Chapter {
    NorthWindow,
    Granary,
}
impl Chapter {
    fn file(self) -> &'static str {
        match self {
            Self::NorthWindow => "story.json",
            Self::Granary => "story-granary.json",
        }
    }
    fn objective(self) -> Value {
        match self {
            Self::NorthWindow => {
                json!({"habitat":"land","x":4,"y":1,"species":"sunflower","minGrowthHours":18,"participation":"Reply to the configured thread using the land box identity"})
            }
            Self::Granary => {
                json!({"habitat":"land","prerequisite":"northWindow","steps":[{"step":1,"minStoredFood":3,"minAntsInsideSameNest":1},{"step":2,"queenAndOffspringInSameNest":true,"offspring":"ant egg or worker younger than 6 hours"},{"step":3,"minEntrances":2,"minAntsInsideSameNest":1},{"step":4,"reply":"[续记]北窗仍在观察","afterStep":3}],"participation":"Reply to the configured thread using the same land box identity"})
            }
        }
    }
    fn message(self, step: u8) -> (&'static str, &'static str) {
        match self {
            Self::NorthWindow => (TITLE, TEXT),
            Self::Granary => crate::story_text::PAGES[usize::from(step - 1)],
        }
    }
    fn eligible(self, entry: &Value, step: u8) -> bool {
        let state = &entry["state"];
        match self {
            Self::NorthWindow => state["plants"].as_array().is_some_and(|plants| {
                plants.iter().any(|p| {
                    p["species"] == "sunflower"
                        && p["x"].as_f64() == Some(4.0)
                        && p["y"].as_f64() == Some(1.0)
                        && p["growthHours"].as_f64().is_some_and(|h| h >= 18.0)
                        && p["health"].as_f64().is_some_and(|h| h > 0.0)
                })
            }),
            Self::Granary => {
                step == 4
                    || state["nests"].as_array().is_some_and(|nests| {
                        nests.iter().any(|nest| {
                            let ants: Vec<_> = state["bugs"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter(|bug| {
                                    bug["species"] == "ant"
                                        && bug["inside"]["nestId"] == nest["id"]
                                        && bug["energy"].as_f64().is_some_and(|v| v > 0.0)
                                        && bug["hunger"].as_f64().unwrap_or(0.0) < 100.0
                                })
                                .collect();
                            match step {
                                1 => {
                                    nest["food"].as_f64().is_some_and(|food| food >= 3.0)
                                        && !ants.is_empty()
                                }
                                2 => {
                                    ants.iter().any(|ant| ant["role"] == "queen")
                                        && (state["eggs"].as_array().is_some_and(|eggs| {
                                            eggs.iter().any(|egg| {
                                                egg["species"] == "ant"
                                                    && egg["inside"]["nestId"] == nest["id"]
                                            })
                                        }) || ants.iter().any(|ant| {
                                            ant["role"] != "queen"
                                                && ant["ageHours"]
                                                    .as_f64()
                                                    .is_some_and(|age| age < 6.0)
                                        }))
                                }
                                3 => {
                                    !ants.is_empty()
                                        && nest["entrances"]
                                            .as_array()
                                            .is_some_and(|entrances| !entrances.is_empty())
                                }
                                _ => unreachable!(),
                            }
                        })
                    })
            }
        }
    }
}
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
    #[serde(default, skip_serializing_if = "is_zero")]
    step: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reply_after_id: Option<i64>,
}
fn is_zero(value: &u8) -> bool {
    *value == 0
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
                && delivery.step <= 4
                && delivery.reply_after_id.is_none_or(|id| id >= 0)
                && (delivery.step != 3 || delivery.reply_after_id.is_some())
                && delivery.announcement_id > 0
                && delivery.announcement_id < 9_007_199_254_740_991
                && at(&json!(delivery.completed_at)).is_some(),
            "Invalid story delivery"
        );
    }
    Ok(())
}
fn read(conn: &Connection, chapter: Chapter) -> Result<Story> {
    Ok(document(conn, chapter.file())?
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_default())
}
fn summary(story: &Story, chapter: Chapter) -> Value {
    let mut result = json!({"enabled":story.enabled,"threadId":story.thread_id,"completedBoxes":story.deliveries.len(),"objective":chapter.objective()});
    if matches!(chapter, Chapter::Granary) {
        result["completedBoxes"] = json!(story.deliveries.values().filter(|d| d.step == 4).count());
        result["progressCounts"] = json!(
            (1..=4)
                .map(|step| story.deliveries.values().filter(|d| d.step >= step).count())
                .collect::<Vec<_>>()
        );
    }
    result
}
impl Store {
    pub fn story_status(&self) -> Result<Value> {
        Ok(summary(
            &read(&self.reader()?, Chapter::NorthWindow)?,
            Chapter::NorthWindow,
        ))
    }
    pub fn granary_story_status(&self) -> Result<Value> {
        Ok(summary(
            &read(&self.reader()?, Chapter::Granary)?,
            Chapter::Granary,
        ))
    }
    pub fn update_story(&self, input: &Value) -> Result<Value> {
        self.update_chapter(input, Chapter::NorthWindow)
    }
    pub fn update_granary_story(&self, input: &Value) -> Result<Value> {
        self.update_chapter(input, Chapter::Granary)
    }
    fn update_chapter(&self, input: &Value, chapter: Chapter) -> Result<Value> {
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
        let old = read(&self.reader()?, chapter)?;
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
            let mut story = read(tx, chapter)?;
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
            save_document(tx, chapter.file(), &serde_json::to_value(&story)?)?;
            Ok(summary(&story, chapter))
        })
    }
}
pub(crate) fn on_report(store: &Store, tx: &Connection, entry: &Value) -> Result<()> {
    if entry["kind"] != "land" {
        return Ok(());
    }
    let first = read(tx, Chapter::NorthWindow)?;
    let previously_completed = first.deliveries.contains_key(entry["id"].as_str().unwrap());
    deliver(store, tx, entry, Chapter::NorthWindow, first)?;
    if previously_completed {
        let next = read(tx, Chapter::Granary)?;
        deliver(store, tx, entry, Chapter::Granary, next)?;
    }
    Ok(())
}
fn deliver(
    store: &Store,
    tx: &Connection,
    entry: &Value,
    chapter: Chapter,
    mut story: Story,
) -> Result<()> {
    let id = entry["id"].as_str().unwrap();
    let previous = story.deliveries.get(id);
    let step = match chapter {
        Chapter::NorthWindow if previous.is_some() => return Ok(()),
        Chapter::NorthWindow => 0,
        Chapter::Granary => previous.map_or(1, |d| d.step + 1),
    };
    if !story.enabled || step > 4 || !chapter.eligible(entry, step) {
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
    if matches!(chapter, Chapter::Granary) && step == 4 {
        let after = at(&json!(previous.unwrap().completed_at)).unwrap();
        let answered: Option<i64> = forum.query_row(
            "SELECT created FROM posts WHERE thread=?1 AND author=?2 AND created>=?3 AND instr(body,?4)>0 AND id>?5 ORDER BY created LIMIT 1",
            params![story.thread_id,id,after,crate::story_text::ANSWER,previous.unwrap().reply_after_id.unwrap()], |r| r.get(0)
        ).optional()?;
        if answered.is_none_or(|answered| {
            entry["state"]["updatedAtMs"]
                .as_f64()
                .is_none_or(|updated| updated < answered as f64)
        }) {
            return Ok(());
        }
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
    let reply_after_id = if matches!(chapter, Chapter::Granary) && step == 3 {
        Some(forum.query_row("SELECT coalesce(max(id),0) FROM posts", [], |r| r.get(0))?)
    } else {
        None
    };
    let (title, text) = chapter.message(step);
    entries.push(json!({"id":next,"title":title,"text":text,"targets":[id],"createdAt":iso(now),"expiresAt":iso(now+30*86400000)}));
    notices["nextId"] = json!(next + 1);
    story.deliveries.insert(
        id.to_owned(),
        Delivery {
            announcement_id: next,
            completed_at: iso(now),
            step,
            reply_after_id,
        },
    );
    save_document(tx, "announcements.json", &notices)?;
    save_document(tx, chapter.file(), &serde_json::to_value(&story)?)?;
    Ok(())
}
