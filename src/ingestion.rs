use crate::{
    legacy_validation::LegacyValidator,
    policy::{Gate, iso},
    protocol::{apply_changes, state_hash, valid_client_system, valid_hash, valid_version},
    scoring::score,
    store::{Store, random_hex},
};
use serde_json::{Value, json};
use std::sync::LazyLock;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::Semaphore;
pub static REPORT_BUFFERS: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(24 * 1024 * 1024)));

#[derive(Clone)]
pub struct Ingestion {
    store: Store,
    validators: [LegacyValidator; 2],
    next_validator: Arc<AtomicUsize>,
    pending: Arc<Semaphore>,
    metrics: Arc<crate::metrics::Metrics>,
    gates: Arc<Mutex<HashMap<String, CachedGate>>>,
}
struct CachedGate {
    version: Value,
    transport: String,
    expires_at: i64,
    gate: Gate,
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
            validators: std::array::from_fn(|_| LegacyValidator::default()),
            next_validator: Arc::new(AtomicUsize::new(0)),
            pending: Arc::new(Semaphore::new(4)),
            metrics: Arc::new(crate::metrics::Metrics::default()),
            gates: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    pub async fn ready(&self) -> anyhow::Result<()> {
        tokio::try_join!(self.validators[0].ready(), self.validators[1].ready())?;
        Ok(())
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
        let gates = self.gates.clone();
        database(self.store.clone(), move |s| {
            let now = now_ms();
            let controls = s.controls(now)?;
            if let Some(cached) = gates.lock().unwrap().get(&id)
                && cached.expires_at > now
                && cached.version == version
                && cached.transport == transport
                && cached.gate.controls == controls
            {
                return Ok(cached.gate.clone());
            }
            let gate = s.gate(&id, &version, &transport, now)?;
            if valid_version(&version) {
                let mut cache = gates.lock().unwrap();
                if cache.len() >= 16384 {
                    cache.retain(|_, cached| cached.expires_at > now);
                }
                if cache.len() < 16384 {
                    cache.insert(
                        id,
                        CachedGate {
                            version,
                            transport,
                            expires_at: now + 60_000,
                            gate: gate.clone(),
                        },
                    );
                }
            }
            Ok(gate)
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
        let mut timing = self.metrics.report_started();
        let _permit =
            tokio::time::timeout(std::time::Duration::from_secs(2), self.pending.acquire())
                .await
                .map_err(|_| {
                    self.metrics.admission_busy();
                    ReportError::new(503, "Report processing is busy; retry after a short delay")
                })?
                .map_err(|_| ReportError::new(503, "Report processing stopped"))?;
        timing.processing();
        self.enforce(id, &input["clientVersion"], transport).await?;
        for key in ["clientVersion", "coopanionVersion"] {
            if input.get(key).is_some_and(|v| !valid_version(v)) {
                return Err(ReportError::new(400, "Invalid report metadata"));
            }
        }
        if input
            .get("clientSystem")
            .is_some_and(|value| !valid_client_system(value))
        {
            return Err(ReportError::new(400, "Invalid report metadata"));
        }
        if input
            .get("phase")
            .is_some_and(|v| v != "day" && v != "night")
        {
            return Err(ReportError::new(400, "Invalid report metadata"));
        }
        let owned_id = id.to_owned();
        let full = input["kind"] != "delta";
        let payload = if full {
            input["state"].clone()
        } else {
            input["changes"].clone()
        };
        let revision = input["baseRevision"].clone();
        let baseline_hash = input.get("baseHash").cloned();
        let (previous, raw) = database(self.store.clone(), move |s| {
            let previous = s.get(&owned_id)?;
            let prepare = || -> Result<Value, ReportError> {
                if full {
                    return Ok(payload);
                }
                let base = previous
                    .as_ref()
                    .ok_or_else(|| ReportError::resync("Full report required"))?;
                if revision != base["reportRevision"] {
                    return Err(ReportError::resync("Full report required"));
                }
                if baseline_hash.is_some_and(|hash| hash != state_hash(&base["state"])) {
                    return Err(ReportError::resync("Report baseline checksum mismatch"));
                }
                apply_changes(&base["state"], &payload)
                    .map_err(|_| ReportError::new(400, "Invalid report changes"))
            };
            Ok(prepare().map(|raw| (previous, raw)))
        })
        .await??;
        let validator = &self.validators
            [self.next_validator.fetch_add(1, Ordering::Relaxed) % self.validators.len()];
        let state = validator.validate(&raw).await.map_err(|e| {
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
        let expected = input.get("stateHash").cloned();
        let state = tokio::task::spawn_blocking(move || {
            if expected.is_some_and(|hash| hash != state_hash(&state)) {
                Err(ReportError::resync("Report state checksum mismatch"))
            } else {
                Ok(state)
            }
        })
        .await
        .map_err(|_| ReportError::new(500, "Report checksum failed"))??;
        let now = now_ms();
        let updated = state["updatedAtMs"]
            .as_f64()
            .ok_or_else(|| ReportError::new(400, "Invalid report time"))?;
        if updated > (now + 60_000) as f64 || updated < (now - 31 * 86_400_000) as f64 {
            return Err(ReportError::new(
                400,
                format!(
                    "Invalid report time: stateUpdatedAtMs={updated}, serverAtMs={now}; allowed age <= 31 days and future offset <= 60000 ms"
                ),
            ));
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
        for key in ["phase", "clientVersion", "coopanionVersion", "clientSystem"] {
            if let Some(value) = input.get(key) {
                entry[key] = value.clone();
            }
        }
        let (accepted, entry) = database(self.store.clone(), move |s| {
            let accepted = s.commit_entry(&entry, previous.as_ref())?;
            Ok((accepted, entry))
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
            let text=text.to_owned();
            let input:Value=tokio::task::spawn_blocking(move || serde_json::from_str(&text)).await.map_err(|_|ReportError::new(500,"Report parser failed"))?.map_err(|_|ReportError::new(400,"Invalid JSON"))?;
            let next=input["seq"].as_f64().filter(|n|n.fract()==0.0&&*n>self.last_seq as f64&&*n<=9_007_199_254_740_991.0).ok_or_else(||ReportError::new(400,"Invalid report sequence"))? as u64;
            seq=next; self.last_seq=seq;
            ingestion.enforce(id,&input["clientVersion"],"websocket").await?;
            if input["protocol"].as_f64()!=Some(1.0)||!valid_version(&input["clientVersion"])||!(input["phase"]=="day"||input["phase"]=="night")||!(input["kind"]=="full"||input["kind"]=="delta") { return Err(ReportError::new(400,"Client version and report protocol are required")); }
            self.requires_hashes|=input.get("stateHash").is_some()||(input["kind"]=="delta"&&input.get("baseHash").is_some());
            if self.requires_hashes&&(!valid_hash(&input["stateHash"])||(input["kind"]=="delta"&&!valid_hash(&input["baseHash"]))) { return Err(ReportError::resync("Report checksums required")); }
            if input["kind"]=="delta"&&(self.requires_full||!input["baseRevision"].is_string()) { return Err(ReportError::resync("Full report required")); }
            let entry=ingestion.commit(id,&input,"websocket").await?;
            self.requires_full=false;
            Ok::<Value,ReportError>(json!({"kind":"ack","seq":seq,"revision":entry["reportRevision"],"score":entry["score"],"stateHash":if let Some(hash)=input.get("stateHash") { hash.clone() } else { let state=entry["state"].clone(); tokio::task::spawn_blocking(move || state_hash(&state)).await.map_err(|_|ReportError::new(500,"Report checksum failed"))?.into() }}))
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
