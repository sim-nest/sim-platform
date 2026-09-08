use super::{
    BTreeMap, CommandRoute, CommandSpec, Datum, ExecutionRecord, LocalCheckError, MountAccess,
    OperationOutcome, PathBuf, ProcessAttempt, ProcessRefusal, ResourceAccess, SandboxAttempt,
    Symbol,
};

pub(super) fn record_process(attempt: ProcessAttempt) -> ExecutionRecord {
    match attempt {
        ProcessAttempt::Completed { receipt } => ExecutionRecord {
            exit_code: Some(receipt.result.exit_code),
            raw: process_result_datum(
                receipt.result.exit_code,
                &receipt.result.stdout,
                &receipt.result.stderr,
                receipt.result.truncated,
            ),
            cleanup_proven: false,
            bounded: !receipt.result.truncated,
        },
        ProcessAttempt::StoppedAfterTimeout { receipt }
        | ProcessAttempt::StoppedAfterCancel { receipt } => ExecutionRecord {
            exit_code: None,
            raw: node(
                "process-stopped-v1",
                vec![
                    ("provider", Datum::String(receipt.provider)),
                    ("cleanup", Datum::String(receipt.cleanup)),
                ],
            ),
            cleanup_proven: true,
            bounded: true,
        },
        ProcessAttempt::NotDispatched { refusal } => ExecutionRecord {
            exit_code: None,
            raw: process_refusal_datum(&refusal),
            cleanup_proven: true,
            bounded: true,
        },
        ProcessAttempt::UnknownAfterDispatch { evidence } => ExecutionRecord {
            exit_code: None,
            raw: refusal_datum(
                "unknown after dispatch",
                &format!("{}: {}", evidence.stage, evidence.detail),
            ),
            cleanup_proven: false,
            bounded: false,
        },
    }
}

pub(super) fn record_sandbox(
    attempt: SandboxAttempt,
    policy: &sim_lib_exec::SandboxPolicy,
) -> ExecutionRecord {
    match attempt {
        SandboxAttempt::Completed(result) => {
            let bounded =
                result.report.proves_required(policy) && result.report.limit_hits.is_empty();
            let cleanup = !result.report.cleanup.is_empty();
            ExecutionRecord {
                exit_code: Some(result.exit_code),
                raw: process_result_datum(
                    result.exit_code,
                    &String::from_utf8_lossy(&result.stdout),
                    &String::from_utf8_lossy(&result.stderr),
                    !result.report.limit_hits.is_empty(),
                ),
                cleanup_proven: cleanup,
                bounded,
            }
        }
        SandboxAttempt::Stopped(report) => ExecutionRecord {
            exit_code: None,
            raw: node(
                "sandbox-stopped-v1",
                vec![
                    ("launcher", Datum::String(report.launcher)),
                    ("cleanup", Datum::String(report.cleanup)),
                ],
            ),
            cleanup_proven: true,
            bounded: true,
        },
        SandboxAttempt::Refused(refusal) => ExecutionRecord {
            exit_code: None,
            raw: refusal_datum("sandbox refused", &refusal.reason),
            cleanup_proven: true,
            bounded: true,
        },
        SandboxAttempt::Unknown(refusal) => ExecutionRecord {
            exit_code: None,
            raw: refusal_datum("sandbox unknown", &refusal.reason),
            cleanup_proven: false,
            bounded: false,
        },
    }
}

pub(super) fn validate_command_resources(
    command: &CommandSpec,
    resources: &BTreeMap<String, PathBuf>,
) -> Result<(), LocalCheckError> {
    for resource in command.resources() {
        let path = resources
            .get(&resource.source)
            .ok_or_else(|| LocalCheckError::ResourceUnavailable(resource.source.clone()))?;
        if !path.is_dir() {
            return Err(LocalCheckError::ResourceUnavailable(
                resource.source.clone(),
            ));
        }
    }
    if let CommandRoute::Sandbox { policy, .. } = command.route() {
        if command
            .resources()
            .iter()
            .all(|resource| resource.guest_path != "/work")
        {
            return Err(LocalCheckError::InvalidContract(
                "sandbox command has no explicit /work checkout resource".into(),
            ));
        }
        if policy
            .mounts()
            .iter()
            .any(|mount| mount.access == MountAccess::Writable)
            && command
                .resources()
                .iter()
                .all(|resource| resource.access != ResourceAccess::Writable)
        {
            return Err(LocalCheckError::InvalidContract(
                "writable sandbox mount lacks writable command resource".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn node(tag: &str, fields: Vec<(&str, Datum)>) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("local-check", tag),
        fields: fields
            .into_iter()
            .map(|(name, value)| (Symbol::new(name), value))
            .collect(),
    }
}
pub(super) fn refusal_datum(stage: &str, detail: &str) -> Datum {
    node(
        "refusal-v1",
        vec![
            ("stage", Datum::String(stage.into())),
            ("detail", Datum::String(detail.into())),
        ],
    )
}
fn process_result_datum(exit: i32, stdout: &str, stderr: &str, truncated: bool) -> Datum {
    node(
        "process-result-v1",
        vec![
            ("exit", Datum::String(exit.to_string())),
            ("stdout", Datum::String(stdout.into())),
            ("stderr", Datum::String(stderr.into())),
            ("truncated", Datum::Bool(truncated)),
        ],
    )
}
fn process_refusal_datum(refusal: &ProcessRefusal) -> Datum {
    let (kind, detail) = match refusal {
        ProcessRefusal::Invalid(detail) => ("invalid", detail),
        ProcessRefusal::Refused(detail) => ("refused", detail),
        ProcessRefusal::SpawnFailed(detail) => ("spawn-failed", detail),
    };
    node(
        "process-refusal-v1",
        vec![
            (
                "kind",
                Datum::Symbol(Symbol::qualified("process-refusal", kind)),
            ),
            ("detail", Datum::String(detail.clone())),
        ],
    )
}

pub(super) fn outcome_evidence(outcome: OperationOutcome) -> Datum {
    match outcome {
        OperationOutcome::AlreadyTrue { evidence } => node(
            "already-true-v1",
            vec![("evidence", Datum::String(evidence.to_string()))],
        ),
        OperationOutcome::Verified { evidence } => node(
            "verified-v1",
            vec![("evidence", Datum::String(evidence.to_string()))],
        ),
        OperationOutcome::Diverged { observed, expected } => node(
            "diverged-v1",
            vec![("observed", observed), ("expected", expected)],
        ),
        OperationOutcome::Uncertain { last_durable_step } => node(
            "uncertain-v1",
            vec![(
                "last-durable-step",
                Datum::Symbol(Symbol::qualified(
                    "operation-step",
                    match last_durable_step {
                        sim_lib_operation_gate::OperationStep::IntentPersisted => {
                            "intent-persisted"
                        }
                        sim_lib_operation_gate::OperationStep::LeaseAcquired => "lease-acquired",
                        sim_lib_operation_gate::OperationStep::DispatchPersisted => {
                            "dispatch-persisted"
                        }
                        sim_lib_operation_gate::OperationStep::ReceiptPersisted => {
                            "receipt-persisted"
                        }
                        sim_lib_operation_gate::OperationStep::ObservationPersisted => {
                            "observation-persisted"
                        }
                        sim_lib_operation_gate::OperationStep::OutcomePersisted => {
                            "outcome-persisted"
                        }
                    },
                )),
            )],
        ),
    }
}
