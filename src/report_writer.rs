//! Bounded report batches acknowledge only after the shared SQLite transaction commits.
use crate::{
    metrics::Metrics,
    store::{ReportWrite, Store},
};
use std::sync::{Arc, mpsc};
use tokio::sync::oneshot;
#[derive(Clone)]
pub(crate) struct ReportWriter {
    sender: mpsc::SyncSender<Job>,
}
struct Job {
    report: ReportWrite,
    reply: oneshot::Sender<Result<(bool, serde_json::Value), String>>,
}
pub(crate) enum WriteError {
    Busy,
    Database,
}
impl ReportWriter {
    pub fn new(store: Store, metrics: Arc<Metrics>) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Job>(crate::ingestion::REPORT_CONCURRENCY);
        std::thread::Builder::new()
            .name("report-writer".into())
            .spawn(move || {
                while let Ok(first) = receiver.recv() {
                    let mut jobs = vec![first];
                    while jobs.len() < crate::ingestion::REPORT_CONCURRENCY {
                        match receiver.try_recv() {
                            Ok(job) => jobs.push(job),
                            Err(_) => break,
                        }
                    }
                    let (reports, replies): (Vec<_>, Vec<_>) =
                        jobs.into_iter().map(|job| (job.report, job.reply)).unzip();
                    match store.commit_reports(&reports) {
                        Ok(results) => {
                            metrics.committed_batch(
                                results.iter().filter(|r| matches!(r, Ok(true))).count(),
                            );
                            for ((reply, result), report) in
                                replies.into_iter().zip(results).zip(reports)
                            {
                                let _ = reply.send(result.map(|accepted| (accepted, report.entry)));
                            }
                        }
                        Err(_) => {
                            for reply in replies {
                                let _ =
                                    reply.send(Err("Database report transaction failed".into()));
                            }
                        }
                    }
                }
            })
            .expect("Report writer thread");
        Self { sender }
    }
    pub async fn write(
        &self,
        report: ReportWrite,
    ) -> Result<(bool, serde_json::Value), WriteError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .try_send(Job { report, reply })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => WriteError::Busy,
                mpsc::TrySendError::Disconnected(_) => WriteError::Database,
            })?;
        response
            .await
            .map_err(|_| WriteError::Database)?
            .map_err(|_| WriteError::Database)
    }
}
