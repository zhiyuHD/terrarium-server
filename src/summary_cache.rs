//! Derived ranking rows exclude histories and are rebuilt after older binaries write snapshots.
use crate::scoring::{SCORE_VERSION, curve_parts, score_parts};
use anyhow::Result;
use rusqlite::{Connection, params};
use serde_json::Value;

pub(crate) fn initialize(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS box_summaries(id TEXT PRIMARY KEY,kind TEXT NOT NULL,revision TEXT,reported_at TEXT NOT NULL,name TEXT,parts TEXT NOT NULL,earned INTEGER,spent INTEGER,id_sort TEXT NOT NULL,metadata TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS box_wallets ON box_summaries(kind,(earned-spent) DESC,id_sort) WHERE earned>0;
        CREATE TRIGGER IF NOT EXISTS box_summary_update AFTER UPDATE ON boxes BEGIN DELETE FROM box_summaries WHERE id=old.id; END;
        CREATE TRIGGER IF NOT EXISTS box_summary_delete AFTER DELETE ON boxes BEGIN DELETE FROM box_summaries WHERE id=old.id; END;")?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS box_score_rules(version INTEGER NOT NULL);")?;
    let version = tx.query_row("SELECT MAX(version) FROM box_score_rules", [], |row| {
        row.get::<_, Option<i64>>(0)
    })?;
    if version.is_none() || version == Some(2) {
        let cached = {
            let mut query = tx.prepare("SELECT json_array(s.id,s.kind,s.revision,s.reported_at,s.name,s.parts,s.earned,s.spent,s.id_sort,s.metadata) FROM box_summaries s JOIN boxes b ON b.id=s.id WHERE s.revision IS b.revision AND s.reported_at=b.reported_at")?;
            query
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for row in cached {
            let mut fields: Vec<Value> = serde_json::from_str(&row)?;
            let parts = curve_parts(
                serde_json::from_str(fields[5].as_str().unwrap())?,
                if fields[1] == "water" {
                    &["plants", "fish", "water"]
                } else {
                    &["plants", "bugs", "seeds"]
                },
            );
            let score = parts
                .as_object()
                .unwrap()
                .values()
                .map(|v| v.as_f64().unwrap())
                .sum::<f64>() as i64;
            tx.execute(
                "UPDATE boxes SET score=?2 WHERE id=?1 AND score!=?2",
                params![fields[0].as_str().unwrap(), score],
            )?;
            fields[5] = Value::String(parts.to_string());
            tx.execute("INSERT OR REPLACE INTO box_summaries SELECT json_extract(?1,'$[0]'),json_extract(?1,'$[1]'),json_extract(?1,'$[2]'),json_extract(?1,'$[3]'),json_extract(?1,'$[4]'),json_extract(?1,'$[5]'),json_extract(?1,'$[6]'),json_extract(?1,'$[7]'),json_extract(?1,'$[8]'),json_extract(?1,'$[9]')", [serde_json::to_string(&fields)?])?;
        }
    } else if version != Some(SCORE_VERSION) {
        tx.execute("DELETE FROM box_summaries", [])?;
    }
    if version != Some(SCORE_VERSION) {
        tx.execute(
            "DELETE FROM legacy_documents WHERE name='leaderboard.json'",
            [],
        )?;
    }
    {
        let mut query = tx.prepare("SELECT b.id,b.kind,b.revision,b.reported_at,b.id_sort,json_extract(b.entry,'$.state.name'),json_extract(b.entry,'$.state.economy.earned'),json_extract(b.entry,'$.state.economy.spent'),
            CASE WHEN b.kind='water' THEN json_object('plants',json_extract(b.entry,'$.state.plants'),'fish',json_extract(b.entry,'$.state.fish'),'oxygen',json_extract(b.entry,'$.state.oxygen'),'cleanliness',json_extract(b.entry,'$.state.cleanliness'))
            ELSE json_object('plants',json_extract(b.entry,'$.state.plants'),'bugs',json_extract(b.entry,'$.state.bugs')) END,json_object('clientVersion',json_extract(b.entry,'$.clientVersion'),'coopanionVersion',json_extract(b.entry,'$.coopanionVersion'),'reportTransport',json_extract(b.entry,'$.reportTransport'),'clientSystem',json_extract(b.entry,'$.clientSystem'))
            FROM boxes b LEFT JOIN box_summaries s ON s.id=b.id WHERE s.id IS NULL OR s.revision IS NOT b.revision OR s.reported_at!=b.reported_at")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let state: Value = serde_json::from_str(&row.get::<_, String>(8)?)?;
            let parts = score_parts(&state)?;
            let score = parts
                .as_object()
                .unwrap()
                .values()
                .map(|v| v.as_f64().unwrap())
                .sum::<f64>() as i64;
            let id: String = row.get(0)?;
            tx.execute(
                "UPDATE boxes SET score=?2 WHERE id=?1 AND score!=?2",
                params![id, score],
            )?;
            let earned = row.get::<_, Option<i64>>(6).ok().flatten();
            let spent = row.get::<_, Option<i64>>(7).ok().flatten();
            let (earned, spent) = wallet(earned, spent);
            tx.execute(
                "INSERT OR REPLACE INTO box_summaries VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(5)?,
                    parts.to_string(),
                    earned,
                    spent,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(9)?
                ],
            )?;
        }
    }
    tx.execute("DELETE FROM box_score_rules", [])?;
    tx.execute(
        "INSERT INTO box_score_rules(version) VALUES(?1)",
        [SCORE_VERSION],
    )?;
    tx.commit()?;
    Ok(())
}
fn wallet(earned: Option<i64>, spent: Option<i64>) -> (Option<i64>, Option<i64>) {
    match (earned, spent) {
        (Some(e), Some(s)) if (1..=1_000_000_000).contains(&e) && (0..=e).contains(&s) => {
            (Some(e), Some(s))
        }
        _ => (None, None),
    }
}
pub(crate) fn write(conn: &Connection, id: &str, entry: &Value, id_sort: &str) -> Result<()> {
    let state = &entry["state"];
    let (earned, spent) = wallet(
        state["economy"]["earned"].as_i64(),
        state["economy"]["spent"].as_i64(),
    );
    conn.execute(
        "INSERT OR REPLACE INTO box_summaries VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            id,
            entry.get("kind").and_then(Value::as_str).unwrap_or("land"),
            entry["reportRevision"].as_str(),
            entry["reportedAt"].as_str(),
            state["name"].as_str(),
            score_parts(state)?.to_string(),
            earned,
            spent,
            id_sort,
            serde_json::json!({"clientVersion":entry["clientVersion"],"coopanionVersion":entry["coopanionVersion"],"reportTransport":entry["reportTransport"],"clientSystem":entry["clientSystem"]}).to_string()
        ],
    )?;
    Ok(())
}
