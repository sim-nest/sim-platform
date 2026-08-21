use sim_lib_exec::{
    ProcResult, ProcessCancellation, ProcessError, ProcessPort, ProcessReceipt, ProcessRequest,
};
use std::{collections::VecDeque, sync::Mutex};

/// One deterministic modeled process outcome, including cleanup races.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelProcessOutcome {
    Exit {
        at_mono_ns: u64,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        exit_code: i32,
    },
    SpawnFailure(String),
    Timeout {
        at_mono_ns: u64,
        kill_failure: Option<String>,
        leaked_descendants: bool,
    },
    Cancelled {
        at_mono_ns: u64,
        kill_failure: Option<String>,
        leaked_descendants: bool,
    },
}

/// FIFO scripted process adapter. It never reads host time, files, environment, or processes.
pub struct ModelProcess {
    outcomes: Mutex<VecDeque<ModelProcessOutcome>>,
    requests: Mutex<Vec<ProcessRequest>>,
}
impl ModelProcess {
    #[must_use]
    pub fn new(outcomes: impl IntoIterator<Item = ModelProcessOutcome>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into_iter().collect()),
            requests: Mutex::default(),
        }
    }
    #[must_use]
    pub fn requests(&self) -> Vec<ProcessRequest> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}
impl ProcessPort for ModelProcess {
    fn run(
        &self,
        request: &ProcessRequest,
        cancellation: &ProcessCancellation,
    ) -> Result<ProcessReceipt, ProcessError> {
        self.requests
            .lock()
            .expect("model process mutex poisoned")
            .push(request.clone());
        let outcome = self
            .outcomes
            .lock()
            .expect("model process mutex poisoned")
            .pop_front()
            .ok_or_else(|| ProcessError::Spawn("model script exhausted".into()))?;
        if cancellation.is_cancelled() {
            return Err(ProcessError::Cancelled {
                kill_failure: None,
                leaked_descendants: false,
            });
        }
        match outcome {
            ModelProcessOutcome::SpawnFailure(v) => Err(ProcessError::Spawn(v)),
            ModelProcessOutcome::Timeout {
                kill_failure,
                leaked_descendants,
                ..
            } => Err(ProcessError::Timeout {
                kill_failure,
                leaked_descendants,
            }),
            ModelProcessOutcome::Cancelled {
                kill_failure,
                leaked_descendants,
                ..
            } => Err(ProcessError::Cancelled {
                kill_failure,
                leaked_descendants,
            }),
            ModelProcessOutcome::Exit {
                at_mono_ns,
                stdout,
                stderr,
                exit_code,
            } => {
                if at_mono_ns > request.timeout_ms.saturating_mul(1_000_000) {
                    return Err(ProcessError::Timeout {
                        kill_failure: None,
                        leaked_descendants: false,
                    });
                }
                let (stdout, stderr, truncated) = cap(stdout, stderr, request.max_output_bytes);
                Ok(ProcessReceipt {
                    provider: "platform/site/model".into(),
                    elapsed_mono_ns: at_mono_ns,
                    result: ProcResult {
                        stdout: String::from_utf8_lossy(&stdout).into_owned(),
                        stderr: String::from_utf8_lossy(&stderr).into_owned(),
                        exit_code,
                        truncated,
                    },
                })
            }
        }
    }
}
fn cap(mut stdout: Vec<u8>, mut stderr: Vec<u8>, budget: usize) -> (Vec<u8>, Vec<u8>, bool) {
    let total = stdout.len().saturating_add(stderr.len());
    stdout.truncate(budget);
    stderr.truncate(budget.saturating_sub(stdout.len()));
    (stdout, stderr, total > budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, path::PathBuf};
    fn request() -> ProcessRequest {
        ProcessRequest {
            argv: vec!["tool".into(), "arg with spaces".into()],
            cwd: PathBuf::from("/root/work"),
            root: PathBuf::from("/root"),
            timeout_ms: 5,
            max_output_bytes: 4,
            stdin: None,
            environment: BTreeMap::new(),
        }
    }
    #[test]
    fn models_output_timeout_cancellation_failed_kill_and_leaks() {
        let model = ModelProcess::new([
            ModelProcessOutcome::Exit {
                at_mono_ns: 1,
                stdout: b"12345".to_vec(),
                stderr: b"err".to_vec(),
                exit_code: 7,
            },
            ModelProcessOutcome::Timeout {
                at_mono_ns: 6_000_000,
                kill_failure: Some("denied".into()),
                leaked_descendants: true,
            },
            ModelProcessOutcome::Cancelled {
                at_mono_ns: 2,
                kill_failure: None,
                leaked_descendants: false,
            },
        ]);
        let result = model
            .run(&request(), &ProcessCancellation::default())
            .unwrap();
        assert!(result.result.truncated);
        assert_eq!(result.result.stdout, "1234");
        assert!(matches!(
            model.run(&request(), &ProcessCancellation::default()),
            Err(ProcessError::Timeout {
                kill_failure: Some(_),
                leaked_descendants: true
            })
        ));
        assert!(matches!(
            model.run(&request(), &ProcessCancellation::default()),
            Err(ProcessError::Cancelled { .. })
        ));
        assert_eq!(model.requests().len(), 3);
    }
    #[test]
    fn pre_cancel_wins_deterministically() {
        let model = ModelProcess::new([ModelProcessOutcome::SpawnFailure("must not win".into())]);
        let token = ProcessCancellation::default();
        token.cancel();
        assert!(matches!(
            model.run(&request(), &token),
            Err(ProcessError::Cancelled { .. })
        ));
    }
}
