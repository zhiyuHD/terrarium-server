use crate::{
    legacy_validation::LegacyValidator,
    policy::{Gate, iso},
    protocol::{apply_changes, state_hash, valid_hash, valid_version},
    scoring::score,
    store::{Store, random_hex},
};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::LazyLock;
use tokio::sync::Semaphore;
pub static REPORT_BUFFERS: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(24 * 1024 * 1024)));

#[derive(Clone)]
pub struct Ingestion {
    store: Store,
    validator: LegacyValidator,
    pending: Arc<Semaphore>,
    metrics: Arc<crate::metrics::Metrics>,
}
#[derive(Debug)]
pub struct ReportError {
    pub status: u16,
    pub message: String,
    pub resync: bool,
    pub ignored: bool,
}
impl ReportError {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            resync: false,
            ignored: false,
        }
    }
    fn resync(message: &str) -> Self {
        Self {
            resync: true,
            ..Self::new(409, message)
        }
    }
    pub fn body(&self) -> Value {
        let mut body = json!({"error":self.message});
        if self.ignored {
            body["accepted"] = json!(false);
            body["ignored"] = json!(true);
            body["upgradeRequired"] = json!(true);
        }
        body
    }
}
pub async fn database<T: Send + 'static>(
    store: Store,
    work: impl FnOnce(Store) -> anyhow::Result<T> + Send + 'static,
) -> Result<T, ReportError> {
    tokio::task::spawn_blocking(move || work(store))
        .await
        .map_err(|_| ReportError::new(500, "Database task failed"))?
        .map_err(|_| ReportError::new(500, "Database operation failed"))
}
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
impl Ingestion {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            validator: LegacyValidator::default(),
            pending: Arc::new(Semaphore::new(4)),
            metrics: Arc::new(crate::metrics::Metrics::default()),
        }
    }
    pub async fn ready(&self) -> anyhow::Result<()> {
        self.validator.ready().await
    }
    pub fn metrics(&self) -> Arc<crate::metrics::Metrics> {
        self.metrics.clone()
    }
    pub async fn gate(
        &self,
        id: &str,
        version: &Value,
        transport: &str,
    ) -> Result<Gate, ReportError> {
        let id = id.to_owned();
        let version = version.clone();
        let transport = transport.to_owned();
        database(self.store.clone(), move |s| {
            s.gate(&id, &version, &transport, now_ms())
        })
        .await
    }
    pub async fn enforce(
        &self,
        id: &str,
        version: &Value,
        transport: &str,
    ) -> Result<(), ReportError> {
        let gate = self.gate(id, version, transport).await?;
        if let Some(status) = gate.rejection {
            return Err(ReportError {
                ignored: true,
                ..ReportError::new(
                    status,
                    format!(
                        "本次提交未成功，提交内容已被忽略。{}",
                        gate.notice
                            .as_ref()
                            .and_then(|n| n["text"].as_str())
                            .unwrap_or("请升级客户端。")
                    ),
                )
            });
        }
        Ok(())
    }
    pub async fn commit(
        &self,
        id: &str,
        input: &Value,
        transport: &str,
    ) -> Result<Value, ReportError> {
        let _permit = self
            .pending
            .try_acquire()
            .map_err(|_| ReportError::new(503, "Too many pending reports"))?;
        self.enforce(id, &input["clientVersion"], transport).await?;
        for key in ["clientVersion", "coopanionVersion"] {
            if input.get(key).is_some_and(|v| !valid_version(v)) {
                return Err(ReportError::new(400, "Invalid report metadata"));
            }
        }
        if input
            .get("phase")
            .is_some_and(|v| v != "day" && v != "night")
        {
            return Err(ReportError::new(400, "Invalid report metadata"));
        }
        let owned_id = id.to_owned();
        let previous = database(self.store.clone(), move |s| s.get(&owned_id)).await?;
        let raw = if input["kind"] == "delta" {
            let base = previous
                .as_ref()
                .ok_or_else(|| ReportError::resync("Full report required"))?;
            if input["baseRevision"] != base["reportRevision"] {
                return Err(ReportError::resync("Full report required"));
            }
            if input
                .get("baseHash")
                .is_some_and(|h| h != &state_hash(&base["state"]))
            {
                return Err(ReportError::resync("Report baseline checksum mismatch"));
            }
            apply_changes(&base["state"], &input["changes"])
                .map_err(|_| ReportError::new(400, "Invalid report changes"))?
        } else {
            input["state"].clone()
        };
        let state = self.validator.validate(&raw).await.map_err(|e| {
            ReportError::new(
                if e.to_string() == "Invalid report" {
                    400
                } else {
                    500
                },
                if e.to_string() == "Invalid report" {
                    "Invalid report"
                } else {
                    "State validator unavailable"
                },
            )
        })?;
        if input
            .get("stateHash")
            .is_some_and(|h| h != &state_hash(&state))
        {
            return Err(ReportError::resync("Report state checksum mismatch"));
        }
        let now = now_ms();
        let updated = state["updatedAtMs"]
            .as_f64()
            .ok_or_else(|| ReportError::new(400, "Invalid report time"))?;
        if updated > (now + 60_000) as f64 || updated < (now - 31 * 86_400_000) as f64 {
            return Err(ReportError::new(400, "Invalid report time"));
        }
        let kind = if state.get("fish").is_some() {
            "water"
        } else {
            "land"
        };
        if let Some(old) = &previous {
            if old["kind"].as_str().unwrap_or("land") != kind {
                return Err(ReportError::new(409, "Box habitat cannot change"));
            }
            if old["state"]["updatedAtMs"]
                .as_f64()
                .is_some_and(|t| updated < t)
            {
                return Err(ReportError::new(409, "Stale report"));
            }
        }
        let points = score(&state).map_err(|_| ReportError::new(400, "Invalid report"))?;
        let mut entry = json!({"id":id,"kind":kind,"state":state,"reportTransport":transport,"score":points,"reportedAt":iso(now),"reportRevision":random_hex(16)});
        for key in ["phase", "clientVersion", "coopanionVersion"] {
            if let Some(value) = input.get(key) {
                entry[key] = value.clone();
            }
        }
        let write = entry.clone();
        let accepted = database(self.store.clone(), move |s| {
            s.commit_entry(&write, previous.as_ref())
        })
        .await?;
        if !accepted {
            return Err(ReportError::resync(
                "Concurrent report requires resynchronization",
            ));
        }
        self.metrics.accepted(transport);
        Ok(entry)
    }
}
pub struct ReportSession {
    last_seq: u64,
    requires_full: bool,
    requires_hashes: bool,
}
impl Default for ReportSession {
    fn default() -> Self {
        Self {
            last_seq: 0,
            requires_full: true,
            requires_hashes: false,
        }
    }
}
impl ReportSession {
    pub fn new() -> Self {
        Self::default()
    }
    pub async fn receive(&mut self, ingestion: &Ingestion, id: &str, text: Option<&str>) -> Value {
        let mut seq = 0;
        let result=async {
            let text=text.ok_or_else(||ReportError::new(400,"Text reports required"))?;
            let input:Value=serde_json::from_str(text).map_err(|_|ReportError::new(400,"Invalid JSON"))?;
            let next=input["seq"].as_f64().filter(|n|n.fract()==0.0&&*n>self.last_seq as f64&&*n<=9_007_199_254_740_991.0).ok_or_else(||ReportError::new(400,"Invalid report sequence"))? as u64;
            seq=next; self.last_seq=seq;
            ingestion.enforce(id,&input["clientVersion"],"websocket").await?;
            if input["protocol"].as_f64()!=Some(1.0)||!valid_version(&input["clientVersion"])||!(input["phase"]=="day"||input["phase"]=="night")||!(input["kind"]=="full"||input["kind"]=="delta") { return Err(ReportError::new(400,"Client version and report protocol are required")); }
            self.requires_hashes|=input.get("stateHash").is_some()||(input["kind"]=="delta"&&input.get("baseHash").is_some());
            if self.requires_hashes&&(!valid_hash(&input["stateHash"])||(input["kind"]=="delta"&&!valid_hash(&input["baseHash"]))) { return Err(ReportError::resync("Report checksums required")); }
            if input["kind"]=="delta"&&(self.requires_full||!input["baseRevision"].is_string()) { return Err(ReportError::resync("Full report required")); }
            let entry=ingestion.commit(id,&input,"websocket").await?;
            self.requires_full=false;
            Ok::<Value,ReportError>(json!({"kind":"ack","seq":seq,"revision":entry["reportRevision"],"score":entry["score"],"stateHash":state_hash(&entry["state"])}))
        }.await;
        match result {
            Ok(reply) => reply,
            Err(error) if error.resync => {
                self.requires_full = true;
                json!({"kind":"resync","seq":seq})
            }
            Err(error) => {
                let mut body = error.body();
                body["kind"] = json!("error");
                body["seq"] = json!(seq);
                body["status"] = json!(error.status);
                body
            }
        }
    }
}
