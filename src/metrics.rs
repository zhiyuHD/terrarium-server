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
    attempts: AtomicU64,
    waiting: AtomicU64,
    active: AtomicU64,
    peak: AtomicU64,
    busy: AtomicU64,
    buffer_busy: AtomicU64,
    connection_busy: AtomicU64,
    finished: AtomicU64,
    wait_us: AtomicU64,
    max_wait_us: AtomicU64,
    work_us: AtomicU64,
    max_work_us: AtomicU64,
    stages: [StageTiming; 5],
    write_batches: AtomicU64,
    batch_reports: AtomicU64,
    max_batch: AtomicU64,
}
impl Default for Metrics {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            connections: Mutex::new(HashMap::new()),
            http: AtomicU64::new(0),
            websocket: AtomicU64::new(0),
            attempts: AtomicU64::new(0),
            waiting: AtomicU64::new(0),
            active: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            busy: AtomicU64::new(0),
            buffer_busy: AtomicU64::new(0),
            connection_busy: AtomicU64::new(0),
            finished: AtomicU64::new(0),
            wait_us: AtomicU64::new(0),
            max_wait_us: AtomicU64::new(0),
            work_us: AtomicU64::new(0),
            max_work_us: AtomicU64::new(0),
            stages: std::array::from_fn(|_| StageTiming::default()),
            write_batches: AtomicU64::new(0),
            batch_reports: AtomicU64::new(0),
            max_batch: AtomicU64::new(0),
        }
    }
}
#[derive(Default)]
struct StageTiming {
    count: AtomicU64,
    total: AtomicU64,
    max: AtomicU64,
}
pub struct ReportTiming {
    metrics: Arc<Metrics>,
    started: Instant,
    processing: Option<Instant>,
}
impl ReportTiming {
    pub fn processing(&mut self) {
        self.metrics.waiting.fetch_sub(1, Ordering::Relaxed);
        let active = self.metrics.active.fetch_add(1, Ordering::Relaxed) + 1;
        self.metrics.peak.fetch_max(active, Ordering::Relaxed);
        let wait = self.started.elapsed().as_micros() as u64;
        self.metrics.wait_us.fetch_add(wait, Ordering::Relaxed);
        self.metrics.max_wait_us.fetch_max(wait, Ordering::Relaxed);
        self.processing = Some(Instant::now());
    }
}
impl Drop for ReportTiming {
    fn drop(&mut self) {
        if let Some(start) = self.processing {
            let work = start.elapsed().as_micros() as u64;
            self.metrics.work_us.fetch_add(work, Ordering::Relaxed);
            self.metrics.max_work_us.fetch_max(work, Ordering::Relaxed);
            self.metrics.finished.fetch_add(1, Ordering::Relaxed);
            self.metrics.active.fetch_sub(1, Ordering::Relaxed);
        } else {
            self.metrics.waiting.fetch_sub(1, Ordering::Relaxed);
        }
    }
}
pub struct ConnectionGuard {
    metrics: Arc<Metrics>,
    id: String,
}
impl Metrics {
    pub fn report_started(self: &Arc<Self>) -> ReportTiming {
        self.attempts.fetch_add(1, Ordering::Relaxed);
        self.waiting.fetch_add(1, Ordering::Relaxed);
        ReportTiming {
            metrics: self.clone(),
            started: Instant::now(),
            processing: None,
        }
    }
    pub fn buffer_busy(&self) {
        self.buffer_busy.fetch_add(1, Ordering::Relaxed);
    }
    pub fn connection_busy(&self) {
        self.connection_busy.fetch_add(1, Ordering::Relaxed);
    }
    pub fn admission_busy(&self) {
        self.busy.fetch_add(1, Ordering::Relaxed);
    }
    pub fn committed_batch(&self, reports: usize) {
        self.write_batches.fetch_add(1, Ordering::Relaxed);
        self.batch_reports
            .fetch_add(reports as u64, Ordering::Relaxed);
        self.max_batch.fetch_max(reports as u64, Ordering::Relaxed);
    }
    pub fn stage(&self, index: usize, elapsed: std::time::Duration) {
        let stage = &self.stages[index];
        let us = elapsed.as_micros() as u64;
        stage.count.fetch_add(1, Ordering::Relaxed);
        stage.total.fetch_add(us, Ordering::Relaxed);
        stage.max.fetch_max(us, Ordering::Relaxed);
    }
    pub fn processing_status(&self) -> Value {
        let n = self.finished.load(Ordering::Relaxed).max(1);
        let stages = ["policy","prepare","validate","checksum","commit"].iter().enumerate().map(|(i,name)| {let s=&self.stages[i];let count=s.count.load(Ordering::Relaxed);json!({"stage":name,"count":count,"meanMs":s.total.load(Ordering::Relaxed) as f64/count.max(1) as f64/1000.0,"maxMs":s.max.load(Ordering::Relaxed) as f64/1000.0})}).collect::<Vec<_>>();
        json!({"writeBatches":self.write_batches.load(Ordering::Relaxed),"meanReportsPerBatch":self.batch_reports.load(Ordering::Relaxed) as f64/self.write_batches.load(Ordering::Relaxed).max(1) as f64,"maxReportsPerBatch":self.max_batch.load(Ordering::Relaxed),"stages":stages,"attempted":self.attempts.load(Ordering::Relaxed),"accepted":self.accepted_reports(),
            "waiting":self.waiting.load(Ordering::Relaxed),"active":self.active.load(Ordering::Relaxed),"peakActive":self.peak.load(Ordering::Relaxed),
            "admissionBusy":self.busy.load(Ordering::Relaxed),"bufferBusy":self.buffer_busy.load(Ordering::Relaxed),"connectionBusy":self.connection_busy.load(Ordering::Relaxed),
            "finished":self.finished.load(Ordering::Relaxed),"averageWaitMs":self.wait_us.load(Ordering::Relaxed) as f64/n as f64/1000.0,
            "maxWaitMs":self.max_wait_us.load(Ordering::Relaxed) as f64/1000.0,
            "averageProcessingMs":self.work_us.load(Ordering::Relaxed) as f64/n as f64/1000.0,"maxProcessingMs":self.max_work_us.load(Ordering::Relaxed) as f64/1000.0})
    }
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
