use sim_lib_exec::{
    DispatchEvidence, ProcResult, ProcessAttempt, ProcessCancellation, ProcessPort, ProcessReceipt,
    ProcessRefusal, ProcessRequest, StopReceipt,
};
use std::{collections::VecDeque, sync::Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelProcessOutcome {
    Refused(String),
    SpawnFailure(String),
    Exit {
        at_mono_ns: u64,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        exit_code: i32,
    },
    TimeoutStopped {
        at_mono_ns: u64,
    },
    CancelStopped {
        at_mono_ns: u64,
    },
    UnknownAfterDispatch {
        stage: String,
        detail: String,
    },
}
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
    fn run(&self, request: &ProcessRequest, cancellation: &ProcessCancellation) -> ProcessAttempt {
        self.requests
            .lock()
            .expect("model process mutex poisoned")
            .push(request.clone());
        if cancellation.is_cancelled() {
            return ProcessAttempt::NotDispatched {
                refusal: ProcessRefusal::Refused("cancelled before spawn".into()),
            };
        }
        let Some(outcome) = self
            .outcomes
            .lock()
            .expect("model process mutex poisoned")
            .pop_front()
        else {
            return ProcessAttempt::NotDispatched {
                refusal: ProcessRefusal::Refused("model script exhausted".into()),
            };
        };
        match outcome {
            ModelProcessOutcome::Refused(v) => ProcessAttempt::NotDispatched {
                refusal: ProcessRefusal::Refused(v),
            },
            ModelProcessOutcome::SpawnFailure(v) => ProcessAttempt::NotDispatched {
                refusal: ProcessRefusal::SpawnFailed(v),
            },
            ModelProcessOutcome::TimeoutStopped { at_mono_ns } => {
                ProcessAttempt::StoppedAfterTimeout {
                    receipt: stop(at_mono_ns),
                }
            }
            ModelProcessOutcome::CancelStopped { at_mono_ns } => {
                ProcessAttempt::StoppedAfterCancel {
                    receipt: stop(at_mono_ns),
                }
            }
            ModelProcessOutcome::UnknownAfterDispatch { stage, detail } => {
                ProcessAttempt::UnknownAfterDispatch {
                    evidence: DispatchEvidence {
                        provider: "platform/site/model".into(),
                        stage,
                        detail,
                    },
                }
            }
            ModelProcessOutcome::Exit {
                at_mono_ns,
                stdout,
                stderr,
                exit_code,
            } => {
                if at_mono_ns > request.budget.timeout_ms.saturating_mul(1_000_000) {
                    return ProcessAttempt::StoppedAfterTimeout {
                        receipt: stop(at_mono_ns),
                    };
                }
                let (stdout, stderr, truncated) =
                    cap(stdout, stderr, request.budget.max_output_bytes);
                ProcessAttempt::Completed {
                    receipt: ProcessReceipt {
                        provider: "platform/site/model".into(),
                        elapsed_mono_ns: at_mono_ns,
                        result: ProcResult {
                            stdout: String::from_utf8_lossy(&stdout).into_owned(),
                            stderr: String::from_utf8_lossy(&stderr).into_owned(),
                            exit_code,
                            truncated,
                        },
                    },
                }
            }
        }
    }
}
fn stop(elapsed_mono_ns: u64) -> StopReceipt {
    StopReceipt {
        provider: "platform/site/model".into(),
        elapsed_mono_ns,
        cleanup: "process group killed and reaped".into(),
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
    use sim_lib_exec::{ProcessBudget, ProgramRef, ProjectRootRef, SealedBindings};
    fn request() -> ProcessRequest {
        ProcessRequest {
            program: ProgramRef::new("tool").unwrap(),
            argv: vec![],
            root: ProjectRootRef::new("project").unwrap(),
            environment: SealedBindings::empty(),
            private_artifacts: vec![],
            budget: ProcessBudget {
                timeout_ms: 5,
                max_output_bytes: 4,
                stdin: None,
            },
        }
    }
    #[test]
    fn distinguishes_both_sides_of_spawn_and_preserves_nonzero_completion() {
        let model = ModelProcess::new([
            ModelProcessOutcome::SpawnFailure("missing".into()),
            ModelProcessOutcome::UnknownAfterDispatch {
                stage: "reap".into(),
                detail: "lost status".into(),
            },
            ModelProcessOutcome::Exit {
                at_mono_ns: 1,
                stdout: b"12345".to_vec(),
                stderr: vec![],
                exit_code: 7,
            },
        ]);
        assert!(matches!(
            model.run(&request(), &ProcessCancellation::default()),
            ProcessAttempt::NotDispatched { .. }
        ));
        assert!(matches!(
            model.run(&request(), &ProcessCancellation::default()),
            ProcessAttempt::UnknownAfterDispatch { .. }
        ));
        let ProcessAttempt::Completed { receipt } =
            model.run(&request(), &ProcessCancellation::default())
        else {
            panic!()
        };
        assert_eq!(receipt.result.exit_code, 7);
        assert!(receipt.result.truncated)
    }
    #[test]
    fn stopped_outcomes_assert_cleanup() {
        let model = ModelProcess::new([
            ModelProcessOutcome::TimeoutStopped { at_mono_ns: 6 },
            ModelProcessOutcome::CancelStopped { at_mono_ns: 2 },
        ]);
        assert!(matches!(
            model.run(&request(), &ProcessCancellation::default()),
            ProcessAttempt::StoppedAfterTimeout { .. }
        ));
        assert!(matches!(
            model.run(&request(), &ProcessCancellation::default()),
            ProcessAttempt::StoppedAfterCancel { .. }
        ))
    }
}
