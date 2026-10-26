//! Opt-in visits grant a separate shared harvest and seed packets; owner snapshots remain unchanged.
use crate::{
    http::App,
    ingestion::now_ms,
    operations::OperationError,
    push::bearer,
    store::{Store, document, save_document},
};
use anyhow::{Result, ensure};
use axum::{
    Json,
    body::to_bytes,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
const DAY: i64 = 86_400_000;
fn check(ok: bool, status: u16, message: &str) -> Result<()> {
    if !ok {
        return Err(OperationError::new(status, message).into());
    }
    Ok(())
}
pub fn execute(store: &Store, input: &Value, token: &str, now: i64) -> Result<Value> {
    let id = input["id"].as_str().unwrap_or("");
    check(
        store.authenticated(id, token)?,
        401,
        "Invalid box credentials",
    )?;
    let action = input["action"].as_str().unwrap_or("status");
    store.transaction(|tx| {
        let mut data = document(tx, "neighbors.json")?.unwrap_or_else(|| json!({"nextId":1,"profiles":{}}));
        let profiles = data["profiles"].as_object_mut().unwrap();
        let profile = profiles.entry(id.to_owned()).or_insert_with(||json!({"enabled":false,"mail":[],"visits":[],"day":0,"actions":0}));
        if action == "configure" {
            check(input["enabled"].is_boolean(), 400, "enabled must be boolean")?;
            profile["enabled"] = input["enabled"].clone();
        }
        if action == "ack" {
            let cursor = input["cursor"].as_u64().ok_or_else(||OperationError::new(400,"Invalid mailbox cursor"))?;
            profile["mail"].as_array_mut().unwrap().retain(|m|m["id"].as_u64().unwrap() > cursor);
        }
        if ["visit", "harvest", "gift"].contains(&action) {
            let target = input["target"].as_str().unwrap_or("");
            check(target != id && crate::protocol::valid_id(target), 400, "Select another box")?;
            check(profiles[id]["enabled"] == true && profiles.get(target).is_some_and(|p|p["enabled"] == true), 403, "Both boxes must enable neighbor visits")?;
            let entry: Option<String> = tx.query_row("SELECT entry FROM boxes WHERE id=?1 AND kind='land'", [target], |r|r.get(0)).optional()?;
            let entry: Value = serde_json::from_str(&entry.ok_or_else(||OperationError::new(404,"Land box not found"))?)?;
            if action == "harvest" { check(crate::policy::at(&entry["reportedAt"]).is_some_and(|at|at <= now + 60_000 && at + DAY > now),409,"Owner must submit a fresh snapshot before sharing fruit")?; }
            let note = input["note"].as_str().unwrap_or("");
            check(note.chars().count() <= 240 && !note.chars().any(|c|c.is_control()), 400, "Visit note allows 240 characters")?;
            let day = now.div_euclid(DAY);
            if profiles[id]["day"] != day { profiles[id]["day"] = json!(day); profiles[id]["actions"] = json!(0); }
            let actions = profiles[id]["actions"].as_u64().unwrap_or(0);
            check(actions < 5, 429, "Five neighbor actions per UTC day; try tomorrow")?;
            let next = data["nextId"].as_u64().unwrap();
            check(next < 9_007_199_254_740_990, 409, "Mailbox cursor exhausted")?;
            let mut mail = None;
            if action == "gift" {
                let species = input["species"].as_str().unwrap_or("");
                check(["tomato","strawberry","mushroom","fern","clover","moss","basil","bean","sunflower","mint","daisy","lavender"].contains(&species),400,"Unknown seed packet")?;
                mail = Some((target, json!({"id":next,"from":id,"kind":"seed","species":species,"portions":1,"at":now})));
            } else if action == "harvest" {
                let plant = entry["state"]["plants"].as_array().and_then(|plants| plants.iter().find(|p|p["id"]==input["plant"])).ok_or_else(||OperationError::new(404,"Crop not found"))?;
                let species = plant["species"].as_str().unwrap_or("");
                let (first, interval) = match species { "tomato" => (18.0,8.0), "strawberry" => (12.0,6.0), _ => return Err(OperationError::new(400,"Select a fruit crop").into()) };
                let hours = plant["fruitGrowthHours"].as_f64().unwrap_or(0.0);
                let round = ((hours-first)/interval).floor() as i64;
                check(round >= 0 && plant["health"].as_f64().unwrap_or(0.0)>=40.0,409,"No ripe shared fruit")?;
                let key = format!("{}:{}:{}", target, plant["id"].as_str().unwrap(), round);
                let claims = data["claims"].as_object_mut();
                if let Some(claims)=claims { claims.retain(|_,at|at.as_i64().is_some_and(|at| at + 30*DAY > now)); check(!claims.contains_key(&key),409,"This shared harvest was already picked")?; }
                if !data["claims"].is_object() { data["claims"] = json!({}); }
                check(data["claims"].as_object().unwrap().len() < 10000,429,"Shared harvest ledger is full")?;
                data["claims"][key]=json!(now);
                mail = Some((id,json!({"id":next,"from":target,"kind":"fruit","species":species,"portions":1,"at":now})));
            }
            let profiles=data["profiles"].as_object_mut().unwrap();
            profiles.get_mut(id).unwrap()["actions"] = json!(actions+1);
            if let Some((recipient, item))=mail {
                let inbox=profiles.get_mut(recipient).unwrap()["mail"].as_array_mut().unwrap();
                inbox.retain(|m|m["at"].as_i64().is_some_and(|at|at+30*DAY>now));
                check(inbox.len()<32,409,"Recipient mailbox is full; no action was consumed")?;
                inbox.push(item);
            }
            let record=json!({"id":next,"from":id,"to":target,"action":action,"note":note,"at":now});
            for who in [id,target] { let log=profiles.get_mut(who).unwrap()["visits"].as_array_mut().unwrap(); log.push(record.clone()); if log.len()>32 { log.remove(0); } }
            data["nextId"]=json!(next+1);
        } else { check(["status","configure","ack"].contains(&action),400,"Unknown neighbor action")?; }
        let result = json!({"enabled":data["profiles"][id]["enabled"],"mail":data["profiles"][id]["mail"],"visits":data["profiles"][id]["visits"],"remainingActions":5-data["profiles"][id]["actions"].as_u64().unwrap_or(0).min(5),"sharing":"Shared fruit is a separate visitor portion; the owner's plants and basket are preserved. Seed gifts are free welcome packets."});
        save_document(tx,"neighbors.json",&data)?;
        Ok(result)
    })
}
pub fn validate(value: &Value) -> Result<()> {
    ensure!(
        value["nextId"].as_u64().is_some_and(|n| n > 0) && value["profiles"].is_object(),
        "Invalid neighbor ledger"
    );
    for profile in value["profiles"].as_object().unwrap().values() {
        ensure!(
            profile["enabled"].is_boolean()
                && profile["mail"].as_array().is_some_and(|a| a.len() <= 32)
                && profile["visits"].as_array().is_some_and(|a| a.len() <= 32),
            "Invalid neighbor profile"
        );
    }
    Ok(())
}
pub async fn handle(State(app): State<App>, request: Request) -> Response {
    if app.ingestion.is_none() {
        return StatusCode::NOT_IMPLEMENTED.into_response();
    }
    let token = bearer(request.headers());
    let bytes = match to_bytes(request.into_body(), 4096).await {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let input: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match tokio::task::spawn_blocking(move || execute(&app.store, &input, &token, now_ms())).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => {
            let (status, message) = error
                .downcast_ref::<OperationError>()
                .map(|e| (e.status, e.message.as_str()))
                .unwrap_or((500, "Neighbor operation failed"));
            (
                StatusCode::from_u16(status).unwrap(),
                Json(json!({"error":message})),
            )
                .into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
