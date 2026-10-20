//! Temporary state-validation fallback until Rust covers legacy migrations and optional fields.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};
#[derive(Clone, Default)]
pub struct LegacyValidator {
    worker: Arc<Mutex<Option<Worker>>>,
}
struct Worker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}
impl Worker {
    fn start() -> Result<Self> {
        let mut child = Command::new(
            std::env::var("CORTICO_TERRARIUM_LEGACY_NODE").unwrap_or_else(|_| "node".into()),
        )
        .args([
            "--input-type=module",
            "-e",
            include_str!("../bridge/validator.mjs"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("Legacy validator could not start")?;
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Ok(Self {
            child,
            input,
            output,
        })
    }
}
impl LegacyValidator {
    pub async fn ready(&self) -> Result<()> {
        let reply = self.exchange(&Value::Null).await?;
        ensure!(
            reply["invalid"] == true,
            "Legacy validator readiness reply is incompatible"
        );
        Ok(())
    }
    pub async fn validate(&self, state: &Value) -> Result<Value> {
        let reply = self.exchange(state).await?;
        ensure!(reply.get("state").is_some(), "Invalid report");
        Ok(reply["state"].clone())
    }
    async fn exchange(&self, state: &Value) -> Result<Value> {
        let mut guard = self.worker.lock().await;
        // A cancelled request owns and drops the worker; its unread reply cannot reach another request.
        let mut worker = match guard.take() {
            Some(worker) => worker,
            None => Worker::start()?,
        };
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            let mut bytes = serde_json::to_vec(state)?;
            bytes.push(b'\n');
            worker.input.write_all(&bytes).await?;
            worker.input.flush().await?;
            let mut output = Vec::new();
            let mut limited =
                (&mut worker.output).take((crate::protocol::MAX_BYTES + 65536) as u64);
            limited.read_until(b'\n', &mut output).await?;
            ensure!(
                output.last() == Some(&b'\n'),
                "Legacy validator stopped or exceeded response limit"
            );
            Ok::<Value, anyhow::Error>(serde_json::from_slice(&output)?)
        })
        .await;
        match result {
            Ok(Ok(result)) => {
                *guard = Some(worker);
                Ok(result)
            }
            other => {
                let _ = worker.child.kill().await;
                match other {
                    Err(_) => anyhow::bail!("Legacy validator timed out"),
                    Ok(Err(e)) => Err(e),
                    _ => unreachable!(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scripted(script: &str) -> Worker {
        let mut child = Command::new("node")
            .args(["-e", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        Worker {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
        }
    }
    fn land() -> Value {
        serde_json::from_str::<Value>(include_str!("../tests/fixtures/reports.json")).unwrap()["land"].clone()
    }
    #[tokio::test]
    async fn new_land_species_preserve_harvest_rounds_and_reject_invalid_counters() {
        let validator = LegacyValidator::default();
        let mut state = land();
        state["plants"][0]["species"] = serde_json::json!("sunflower");
        state["plants"][0]["growthHours"] = serde_json::json!(30);
        state["plants"][0]["stage"] = serde_json::json!(3);
        state["plants"][0]["seedRounds"] = serde_json::json!(1);
        state["plants"][1]["species"] = serde_json::json!("mint");
        state["bugs"] = serde_json::json!([{
            "id": "bug-new-neighbor", "species": "ladybug", "x": 1, "y": 1,
            "ageHours": 1, "energy": 80, "hunger": 10
        }]);
        state["food"] = serde_json::json!([{
            "id": "food-new-neighbor", "kind": "sunflower_seed", "x": 0, "y": 0,
            "portions": 3, "droppedAtMs": state["updatedAtMs"]
        }]);
        assert_eq!(validator.validate(&state).await.unwrap(), state);
        state["plants"][0]["seedRounds"] = serde_json::json!(2);
        assert!(validator.validate(&state).await.is_err());
        assert_eq!(validator.validate(&land()).await.unwrap(), land());
    }
    #[tokio::test]
    async fn cancelling_validation_cannot_deliver_its_reply_to_the_next_request() {
        let validator = LegacyValidator {
            worker: Arc::new(Mutex::new(Some(scripted(
                "process.stdin.once('data',()=>setTimeout(()=>process.stdout.write(JSON.stringify({state:{contaminated:true}})+'\\n'),100))",
            )))),
        };
        let cloned = validator.clone();
        let task = tokio::spawn(async move { cloned.validate(&land()).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if validator.worker.try_lock().is_err() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        task.abort();
        let _ = task.await;
        assert_eq!(validator.validate(&land()).await.unwrap(), land());
    }
    #[tokio::test]
    async fn worker_process_failure_rejects_report_and_restarts_on_next_validation() {
        let validator = LegacyValidator {
            worker: Arc::new(Mutex::new(Some(scripted("process.exit(1)")))),
        };
        assert!(validator.validate(&land()).await.is_err());
        assert_eq!(validator.validate(&land()).await.unwrap(), land());
    }
}
