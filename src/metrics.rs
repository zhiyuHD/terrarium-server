use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
pub struct Metrics {
    started: Instant,
    connections: Mutex<HashMap<String, usize>>,
    http: AtomicU64,
    websocket: AtomicU64,
}
impl Default for Metrics {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            connections: Mutex::new(HashMap::new()),
            http: AtomicU64::new(0),
            websocket: AtomicU64::new(0),
        }
    }
}
pub struct ConnectionGuard {
    metrics: Arc<Metrics>,
    id: String,
}
impl Metrics {
    pub fn connected(self: &Arc<Self>, id: &str) -> ConnectionGuard {
        *self
            .connections
            .lock()
            .unwrap()
            .entry(id.to_owned())
            .or_default() += 1;
        ConnectionGuard {
            metrics: self.clone(),
            id: id.to_owned(),
        }
    }
    pub fn accepted(&self, transport: &str) {
        if transport == "http" {
            &self.http
        } else {
            &self.websocket
        }
        .fetch_add(1, Ordering::Relaxed);
    }
    pub fn uptime(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
    pub fn accepted_reports(&self) -> Value {
        json!({"http":self.http.load(Ordering::Relaxed),"websocket":self.websocket.load(Ordering::Relaxed)})
    }
    pub fn annotate(&self, boxes: &mut [Value]) {
        let counts = self.connections.lock().unwrap();
        for entry in boxes {
            entry["wsConnections"] = json!(
                counts
                    .get(entry["id"].as_str().unwrap())
                    .copied()
                    .unwrap_or(0)
            );
        }
    }
}
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        let mut counts = self.metrics.connections.lock().unwrap();
        let count = counts.get_mut(&self.id).unwrap();
        *count -= 1;
        if *count == 0 {
            counts.remove(&self.id);
        }
    }
}
pub fn memory_bytes() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
                .map(|kb| kb * 1024)
        })
}
