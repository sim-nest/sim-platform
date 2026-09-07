//! Durable exact-command adapter for local implementation checks.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use sim_kernel::{CapabilityName, Datum, Symbol};
use sim_lib_exec::{
    CommandId, CommandReplayPolicy, CommandRoute, CommandSpec, LauncherRegistry, LocalCheckLease,
    LocalCheckPort, LocalCheckRequest, LocalCheckResult, LocalCheckStatus, MountAccess,
    OutputState, ProcessAttempt, ProcessCancellation, ProcessPort, ProcessRefusal, ProcessRequest,
    ResourceAccess, SandboxAttempt, SandboxRequest,
};
use sim_lib_journal::JournalBackend;
use sim_lib_operation_gate::{
    FencedDispatch, FencedDispatchId, LeaseWindow, LifecyclePerformer, LifecyclePerformerResponse,
    OperationError, OperationGrant, OperationIntent, OperationLifecycle, OperationOutcome,
    PostconditionObserver, PostconditionRequest, PostconditionResponse, ReplayPolicy,
};
use thiserror::Error;

/// Typed refusal from command admission, execution composition, or durable operation handling.
#[derive(Debug, Error)]
pub enum LocalCheckError {
    /// The request names no exact installed command.
    #[error("local check command is not installed")]
    CommandNotInstalled,
    /// A command resource has no boot-authorized native root.
    #[error("local check resource is unavailable: {0}")]
    ResourceUnavailable(String),
    /// A command contract is inconsistent with its selected execution route.
    #[error("invalid local check contract: {0}")]
    InvalidContract(String),
    /// The durable operation lifecycle refused or could not persist the request.
    #[error(transparent)]
    Operation(#[from] OperationError),
}

#[derive(Clone, Debug)]
struct ExecutionRecord {
    exit_code: Option<i32>,
    raw: Datum,
    cleanup_proven: bool,
    bounded: bool,
}

/// Derives stable M5 intent from one request and its exact installed command.
///
/// # Errors
/// Returns [`LocalCheckError::CommandNotInstalled`] for an identity mismatch or
/// propagates canonical operation-intent construction failure.
pub fn request_check(
    request: &LocalCheckRequest,
    command: &CommandSpec,
) -> Result<OperationIntent, LocalCheckError> {
    if request.command() != command.id() {
        return Err(LocalCheckError::CommandNotInstalled);
    }
    OperationIntent::new(
        "local-check/run",
        request.canonical_datum(),
        command.outputs().canonical_datum(),
        match command.replay() {
            CommandReplayPolicy::Idempotent => ReplayPolicy::Idempotent,
            CommandReplayPolicy::ExactlyOnce => ReplayPolicy::ExactlyOnce,
        },
    )
    .map_err(Into::into)
}

/// Registered Ubuntu composition of M5, `ProcessPort`, and `SandboxLauncher`.
pub struct LocalCheckAdapter<B: JournalBackend> {
    lifecycle: OperationLifecycle<B>,
    process: Arc<dyn ProcessPort>,
    launchers: Arc<LauncherRegistry>,
    commands: BTreeMap<CommandId, CommandSpec>,
    resources: Arc<BTreeMap<String, PathBuf>>,
    executions: Arc<Mutex<BTreeMap<FencedDispatchId, ExecutionRecord>>>,
}

impl<B: JournalBackend> LocalCheckAdapter<B> {
    /// Builds a closed adapter from boot-authorized ports, specs, and native resources.
    ///
    /// # Errors
    /// Returns a typed refusal for an empty or duplicate allowlist, or when a
    /// command names a missing or inconsistent resource.
    pub fn new(
        backend: B,
        process: Arc<dyn ProcessPort>,
        launchers: LauncherRegistry,
        commands: impl IntoIterator<Item = CommandSpec>,
        resources: BTreeMap<String, PathBuf>,
    ) -> Result<Self, LocalCheckError> {
        let mut installed = BTreeMap::new();
        for command in commands {
            validate_command_resources(&command, &resources)?;
            if installed.insert(command.id().clone(), command).is_some() {
                return Err(LocalCheckError::InvalidContract(
                    "duplicate command identity".into(),
                ));
            }
        }
        if installed.is_empty() {
            return Err(LocalCheckError::InvalidContract(
                "empty command allowlist".into(),
            ));
        }
        Ok(Self {
            lifecycle: OperationLifecycle::new(backend),
            process,
            launchers: Arc::new(launchers),
            commands: installed,
            resources: Arc::new(resources),
            executions: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }

    /// Returns the exact installed command when present.
    #[must_use]
    pub fn command(&self, id: &CommandId) -> Option<&CommandSpec> {
        self.commands.get(id)
    }

    /// Executes or reconciles one local check under a bounded fenced lease.
    ///
    /// # Errors
    /// Returns a typed refusal when the request is not installed, its lease is
    /// invalid, or the durable lifecycle cannot advance.
    pub fn run(
        &mut self,
        request: &LocalCheckRequest,
        holder: Datum,
        now: u64,
        expires_at: u64,
        cancellation: &ProcessCancellation,
    ) -> Result<OperationOutcome, LocalCheckError> {
        let command = self
            .commands
            .get(request.command())
            .ok_or(LocalCheckError::CommandNotInstalled)?
            .clone();
        match (command.network(), request.network_grant()) {
            (sim_lib_exec::NetworkAccess::Absent, None) => {}
            (sim_lib_exec::NetworkAccess::Scoped(expected), Some((observed, _)))
                if expected == observed => {}
            _ => {
                return Err(LocalCheckError::InvalidContract(
                    "separate network capability grant does not match command".into(),
                ));
            }
        }
        let intent = request_check(request, &command)?;
        let grant = OperationGrant::new(
            intent.id().clone(),
            CapabilityName::new("exec/local-check"),
            Datum::String(request.grant().as_str().into()),
        )?;
        let mut performer = LocalPerformer {
            process: Arc::clone(&self.process),
            launchers: Arc::clone(&self.launchers),
            command: command.clone(),
            cancellation: cancellation.clone(),
            resources: Arc::clone(&self.resources),
            executions: Arc::clone(&self.executions),
        };
        let mut observer = LocalObserver {
            command,
            resources: Arc::clone(&self.resources),
            executions: Arc::clone(&self.executions),
        };
        self.lifecycle
            .run(
                &intent,
                &grant,
                LeaseWindow::new(holder, now, expires_at)?,
                &mut performer,
                &mut observer,
            )
            .map_err(Into::into)
    }

    /// Reconstructs the durable lifecycle for a request without performing it.
    ///
    /// # Errors
    /// Returns a typed refusal when the request is not installed or the journal
    /// cannot be verified.
    pub fn record(
        &self,
        request: &LocalCheckRequest,
    ) -> Result<Option<sim_lib_operation_gate::OperationLifecycleRecord>, LocalCheckError> {
        let command = self
            .commands
            .get(request.command())
            .ok_or(LocalCheckError::CommandNotInstalled)?;
        let intent = request_check(request, command)?;
        self.lifecycle.record(intent.id()).map_err(Into::into)
    }
}

impl<B: JournalBackend> LocalCheckPort for LocalCheckAdapter<B> {
    fn check(
        &mut self,
        request: &LocalCheckRequest,
        lease: &LocalCheckLease,
        cancellation: &ProcessCancellation,
    ) -> LocalCheckResult {
        let operation = self
            .commands
            .get(request.command())
            .and_then(|command| request_check(request, command).ok())
            .map(|intent| intent.id().to_string());
        match self.run(
            request,
            lease.holder.clone(),
            lease.acquired_at,
            lease.expires_at,
            cancellation,
        ) {
            Ok(outcome) => LocalCheckResult {
                operation,
                status: match outcome {
                    OperationOutcome::AlreadyTrue { .. } => LocalCheckStatus::AlreadyTrue,
                    OperationOutcome::Verified { .. } => LocalCheckStatus::Verified,
                    OperationOutcome::Diverged { .. } => LocalCheckStatus::Diverged,
                    OperationOutcome::Uncertain { .. } => LocalCheckStatus::Uncertain,
                },
                evidence: outcome_evidence(outcome),
            },
            Err(error) => LocalCheckResult {
                operation,
                status: LocalCheckStatus::Refused,
                evidence: node(
                    "local-check-refusal-v1",
                    vec![("detail", Datum::String(error.to_string()))],
                ),
            },
        }
    }
}

struct LocalPerformer {
    process: Arc<dyn ProcessPort>,
    launchers: Arc<LauncherRegistry>,
    command: CommandSpec,
    cancellation: ProcessCancellation,
    resources: Arc<BTreeMap<String, PathBuf>>,
    executions: Arc<Mutex<BTreeMap<FencedDispatchId, ExecutionRecord>>>,
}

impl LifecyclePerformer for LocalPerformer {
    fn identity(&self) -> Datum {
        Datum::String("platform/site/ubuntu-pc/local-check-performer".into())
    }

    fn perform(&mut self, dispatch: &FencedDispatch) -> LifecyclePerformerResponse {
        let argv = match self.command.invocation().argv() {
            Ok(argv) => argv,
            Err(error) => {
                return LifecyclePerformerResponse::Receipt(refusal_datum(
                    "invalid invocation",
                    &error.to_string(),
                ));
            }
        };
        let record = match self.command.route() {
            CommandRoute::Process => {
                let request = ProcessRequest {
                    program: self.command.program().clone(),
                    argv,
                    root: self.command.root().clone(),
                    environment: self.command.environment().clone(),
                    private_artifacts: vec![],
                    budget: self.command.budget().clone(),
                };
                record_process(self.process.run(&request, &self.cancellation))
            }
            CommandRoute::Sandbox { launcher, policy } => {
                let request = match SandboxRequest::new(
                    self.command.program().clone(),
                    argv,
                    self.command.environment().clone(),
                    self.command.budget().stdin.clone().unwrap_or_default(),
                    policy.clone(),
                ) {
                    Ok(request) => request,
                    Err(error) => {
                        return LifecyclePerformerResponse::Receipt(refusal_datum(
                            "invalid sandbox request",
                            &error.to_string(),
                        ));
                    }
                };
                record_sandbox(
                    self.launchers
                        .launch(launcher, &request, &self.cancellation),
                    policy,
                )
            }
        };
        let scratch_clean =
            clean_scratch(self.command.cleanup().scratch_resources(), &self.resources);
        let mut record = record;
        record.cleanup_proven &= scratch_clean;
        let raw = record.raw.clone();
        self.executions
            .lock()
            .expect("local execution record lock")
            .insert(dispatch.id().clone(), record);
        LifecyclePerformerResponse::Receipt(raw)
    }
}

struct LocalObserver {
    command: CommandSpec,
    resources: Arc<BTreeMap<String, PathBuf>>,
    executions: Arc<Mutex<BTreeMap<FencedDispatchId, ExecutionRecord>>>,
}

impl PostconditionObserver for LocalObserver {
    fn identity(&self) -> Datum {
        Datum::String("platform/site/ubuntu-pc/local-check-observer".into())
    }

    fn observe(&mut self, request: &PostconditionRequest) -> PostconditionResponse {
        let outputs = match observe_outputs(&self.command, &self.resources) {
            Ok(value) => value,
            Err(reason) => {
                return PostconditionResponse::Unavailable {
                    reason: Datum::String(reason),
                };
            }
        };
        let record = request.dispatch().and_then(|dispatch| {
            self.executions
                .lock()
                .expect("local execution record lock")
                .get(dispatch)
                .cloned()
        });
        let Some(record) = record else {
            if outputs.complete && !self.command.outputs().outputs().is_empty() {
                return PostconditionResponse::Satisfied {
                    observed: self.command.outputs().canonical_datum(),
                    evidence: outputs.evidence,
                };
            }
            return PostconditionResponse::NotSatisfied {
                observed: Datum::String("no independently observed completed invocation".into()),
                evidence: outputs.evidence,
            };
        };
        if !record.bounded || !record.cleanup_proven {
            return PostconditionResponse::Unavailable {
                reason: node(
                    "local-check-observer-unavailable-v1",
                    vec![
                        ("raw", record.raw),
                        ("bounded", Datum::Bool(record.bounded)),
                        ("cleanup-proven", Datum::Bool(record.cleanup_proven)),
                    ],
                ),
            };
        }
        let exit_matches = record
            .exit_code
            .is_some_and(|code| self.command.outputs().exit_codes().contains(&code));
        let evidence = node(
            "local-check-observation-v1",
            vec![
                ("command", Datum::String(self.command.id().to_string())),
                (
                    "exit",
                    record
                        .exit_code
                        .map_or(Datum::Nil, |code| Datum::String(code.to_string())),
                ),
                ("outputs", outputs.evidence),
                ("cleanup", Datum::Bool(record.cleanup_proven)),
            ],
        );
        if exit_matches && outputs.complete {
            PostconditionResponse::Satisfied {
                observed: self.command.outputs().canonical_datum(),
                evidence,
            }
        } else {
            PostconditionResponse::NotSatisfied {
                observed: node(
                    "local-check-diverged-v1",
                    vec![
                        ("exit-matches", Datum::Bool(exit_matches)),
                        ("outputs-match", Datum::Bool(outputs.complete)),
                    ],
                ),
                evidence,
            }
        }
    }
}

struct OutputObservation {
    complete: bool,
    evidence: Datum,
}

fn observe_outputs(
    command: &CommandSpec,
    resources: &BTreeMap<String, PathBuf>,
) -> Result<OutputObservation, String> {
    let mut complete = true;
    let mut evidence = Vec::new();
    for expected in command.outputs().outputs() {
        let root = resources
            .get(&expected.resource)
            .ok_or_else(|| format!("unregistered output resource {}", expected.resource))?;
        let root = root
            .canonicalize()
            .map_err(|error| format!("output root unavailable: {error}"))?;
        let path = root.join(&expected.relative_path);
        let state_matches = match &expected.state {
            OutputState::Exists => confined_existing(&root, &path)?.is_some(),
            OutputState::Absent => confined_existing(&root, &path)?.is_none(),
            OutputState::FileContent(expected_id) => {
                let Some(path) = confined_existing(&root, &path)? else {
                    complete = false;
                    continue;
                };
                let bytes =
                    fs::read(path).map_err(|error| format!("output read failed: {error}"))?;
                Datum::Bytes(bytes)
                    .content_id()
                    .map_err(|_| "output content is not canonical".to_owned())?
                    == *expected_id
            }
        };
        complete &= state_matches;
        evidence.push(node(
            "output-path-v1",
            vec![
                ("resource", Datum::String(expected.resource.clone())),
                (
                    "relative-path",
                    Datum::String(expected.relative_path.clone()),
                ),
                ("matches", Datum::Bool(state_matches)),
            ],
        ));
    }
    Ok(OutputObservation {
        complete,
        evidence: Datum::Vector(evidence),
    })
}

fn confined_existing(root: &Path, path: &Path) -> Result<Option<PathBuf>, String> {
    match path.canonicalize() {
        Ok(path) if path.starts_with(root) => Ok(Some(path)),
        Ok(_) => Err("output path escapes its registered root".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut parent = path.parent();
            while let Some(candidate) = parent {
                if candidate.exists() {
                    let candidate = candidate
                        .canonicalize()
                        .map_err(|error| format!("output parent unavailable: {error}"))?;
                    return if candidate.starts_with(root) {
                        Ok(None)
                    } else {
                        Err("output parent escapes its registered root".into())
                    };
                }
                parent = candidate.parent();
            }
            Err("output path has no registered ancestor".into())
        }
        Err(error) => Err(format!("output observation failed: {error}")),
    }
}

fn clean_scratch(
    names: &std::collections::BTreeSet<String>,
    resources: &BTreeMap<String, PathBuf>,
) -> bool {
    names.iter().all(|name| {
        resources
            .get(name)
            .is_some_and(|root| clear_owned_root(root).is_ok())
    })
}

fn clear_owned_root(root: &Path) -> Result<(), String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("scratch root unavailable: {error}"))?;
    for entry in fs::read_dir(&root).map_err(|error| format!("scratch root unreadable: {error}"))? {
        let path = entry
            .map_err(|error| format!("scratch entry unreadable: {error}"))?
            .path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("scratch metadata unavailable: {error}"))?;
        if metadata.file_type().is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        }
        .map_err(|error| format!("scratch cleanup failed: {error}"))?;
    }
    Ok(())
}

fn record_process(attempt: ProcessAttempt) -> ExecutionRecord {
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

fn record_sandbox(
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

fn validate_command_resources(
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

fn node(tag: &str, fields: Vec<(&str, Datum)>) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("local-check", tag),
        fields: fields
            .into_iter()
            .map(|(name, value)| (Symbol::new(name), value))
            .collect(),
    }
}
fn refusal_datum(stage: &str, detail: &str) -> Datum {
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

fn outcome_evidence(outcome: OperationOutcome) -> Datum {
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

#[cfg(test)]
mod tests {
    use super::*;
    use sim_lib_exec::{
        ArgAtom, BuildSourceRef, CapabilityGrantRef, CleanupContract, CommandInvocation,
        CommandReplayPolicy, CommandResource, MountAccess, NetworkAccess, OutputContract,
        OutputExpectation, PacketRef, ProcResult, ProcessBudget, ProcessReceipt, ProgramRef,
        ProjectRootRef, SandboxControl, SandboxEvidence, SandboxLimits, SandboxMount,
        SandboxPolicy, SandboxReport, SandboxRequirement, SandboxResult, SealedBindings,
    };
    use sim_lib_journal::MemoryBackend;
    use std::{
        collections::BTreeSet,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[derive(Default)]
    struct RefusingProcess;
    impl ProcessPort for RefusingProcess {
        fn run(&self, _: &ProcessRequest, _: &ProcessCancellation) -> ProcessAttempt {
            ProcessAttempt::NotDispatched {
                refusal: ProcessRefusal::Refused("sandbox required".into()),
            }
        }
    }

    struct FixtureLauncher {
        work: PathBuf,
        scratch: PathBuf,
    }
    impl sim_lib_exec::SandboxLauncher for FixtureLauncher {
        fn id(&self) -> &'static str {
            "fixture/sandbox"
        }
        fn launch(&self, request: &SandboxRequest, _: &ProcessCancellation) -> SandboxAttempt {
            let script = request.argv.last().map(ArgAtom::as_str).unwrap_or_default();
            fs::write(self.scratch.join("temporary"), b"leak").unwrap();
            let (exit_code, stderr) = if script == "format" {
                fs::write(self.work.join("formatted.rs"), b"fn main() {}\n").unwrap();
                (0, Vec::new())
            } else {
                (1, b"deliberate test failure".to_vec())
            };
            SandboxAttempt::Completed(SandboxResult {
                stdout: vec![],
                stderr,
                exit_code,
                report: SandboxReport {
                    launcher: self.id().into(),
                    controls: all_controls()
                        .map(|control| SandboxEvidence {
                            control,
                            achieved: true,
                            detail: "fixture control proof".into(),
                        })
                        .collect(),
                    limit_hits: vec![],
                    cleanup: "fixture process group empty".into(),
                },
            })
        }
    }

    fn all_controls() -> impl Iterator<Item = SandboxControl> {
        [
            SandboxControl::Network,
            SandboxControl::Mounts,
            SandboxControl::Root,
            SandboxControl::Environment,
            SandboxControl::Identity,
            SandboxControl::Cpu,
            SandboxControl::Memory,
            SandboxControl::WallTime,
            SandboxControl::ProcessCount,
            SandboxControl::FileCount,
            SandboxControl::FileBytes,
            SandboxControl::Output,
            SandboxControl::Stdin,
            SandboxControl::ProcessTree,
        ]
        .into_iter()
    }
    fn policy(resources: &[CommandResource]) -> SandboxPolicy {
        SandboxPolicy::new(
            all_controls().map(|control| (control, SandboxRequirement::Required)),
            resources
                .iter()
                .map(|resource| SandboxMount {
                    source: resource.source.clone(),
                    guest_path: resource.guest_path.clone(),
                    access: match resource.access {
                        ResourceAccess::ReadOnly => MountAccess::ReadOnly,
                        ResourceAccess::Writable => MountAccess::Writable,
                    },
                })
                .collect(),
            SandboxLimits {
                cpu_seconds: 10,
                memory_bytes: 256 * 1024 * 1024,
                wall_time_ms: 5_000,
                process_count: 16,
                file_count: 1_000,
                file_bytes: 16 * 1024 * 1024,
                output_bytes: 16 * 1024,
                stdin_bytes: 1,
            },
        )
        .unwrap()
    }
    fn root(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "sim-local-check-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn spec(
        script: &str,
        resources: &[CommandResource],
        outputs: OutputContract,
        launcher: &str,
    ) -> CommandSpec {
        CommandSpec::new(
            ProgramRef::new("shell").unwrap(),
            ProjectRootRef::new("work").unwrap(),
            CommandInvocation::Interpreter {
                flags: vec![ArgAtom::new("-c").unwrap()],
                script: script.as_bytes().to_vec(),
            },
            SealedBindings::literals([("PATH".into(), "/usr/bin".into())]).unwrap(),
            resources.to_vec(),
            ProcessBudget {
                timeout_ms: 5_000,
                max_output_bytes: 16 * 1024,
                stdin: None,
            },
            outputs,
            CleanupContract::process_group(["scratch".into()]).unwrap(),
            NetworkAccess::Absent,
            CommandRoute::Sandbox {
                launcher: launcher.into(),
                policy: policy(resources),
            },
            CommandReplayPolicy::ExactlyOnce,
        )
        .unwrap()
    }
    fn request(command: &CommandSpec, label: &str) -> LocalCheckRequest {
        LocalCheckRequest::new(
            PacketRef::new(format!("packet/{label}")).unwrap(),
            command.id().clone(),
            BuildSourceRef::new(format!("source/{label}")).unwrap(),
            CapabilityGrantRef::new(format!("grant/{label}")).unwrap(),
        )
    }

    #[test]
    fn exact_adapter_observes_format_change_failure_and_scratch_cleanup() {
        let work = root("work");
        let scratch = root("scratch");
        fs::write(work.join("formatted.rs"), b"fn  main( ){}\n").unwrap();
        let expected = Datum::Bytes(b"fn main() {}\n".to_vec())
            .content_id()
            .unwrap();
        let resources = vec![
            CommandResource {
                source: "work".into(),
                guest_path: "/work".into(),
                access: ResourceAccess::Writable,
            },
            CommandResource {
                source: "scratch".into(),
                guest_path: "/scratch".into(),
                access: ResourceAccess::Writable,
            },
        ];
        let format = spec(
            "format",
            &resources,
            OutputContract::new(
                [0],
                vec![OutputExpectation {
                    resource: "work".into(),
                    relative_path: "formatted.rs".into(),
                    state: OutputState::FileContent(expected),
                }],
            )
            .unwrap(),
            "fixture/sandbox",
        );
        let fail = spec(
            "fail",
            &resources,
            OutputContract::new([0], vec![]).unwrap(),
            "fixture/sandbox",
        );
        let mut registry = LauncherRegistry::default();
        registry
            .register(Arc::new(FixtureLauncher {
                work: work.clone(),
                scratch: scratch.clone(),
            }))
            .unwrap();
        let roots = BTreeMap::from([
            ("work".into(), work.clone()),
            ("scratch".into(), scratch.clone()),
        ]);
        let mut adapter = LocalCheckAdapter::new(
            MemoryBackend::default(),
            Arc::new(RefusingProcess),
            registry,
            [format.clone(), fail.clone()],
            roots,
        )
        .unwrap();
        assert!(matches!(
            adapter
                .run(
                    &request(&format, "format"),
                    Datum::String("operator".into()),
                    1,
                    10,
                    &ProcessCancellation::default()
                )
                .unwrap(),
            OperationOutcome::Verified { .. }
        ));
        assert_eq!(
            fs::read(work.join("formatted.rs")).unwrap(),
            b"fn main() {}\n"
        );
        assert_eq!(
            fs::read_dir(&scratch).unwrap().count(),
            0,
            "owned scratch must be empty"
        );
        assert!(matches!(
            adapter
                .run(
                    &request(&fail, "fail"),
                    Datum::String("operator".into()),
                    20,
                    30,
                    &ProcessCancellation::default()
                )
                .unwrap(),
            OperationOutcome::Diverged { .. }
        ));
        assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
        fs::remove_dir_all(work).unwrap();
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn request_cannot_substitute_an_uninstalled_command_identity() {
        let work = root("wrong-command-work");
        let scratch = root("wrong-command-scratch");
        let resources = vec![
            CommandResource {
                source: "work".into(),
                guest_path: "/work".into(),
                access: ResourceAccess::Writable,
            },
            CommandResource {
                source: "scratch".into(),
                guest_path: "/scratch".into(),
                access: ResourceAccess::Writable,
            },
        ];
        let installed = spec(
            "format",
            &resources,
            OutputContract::new([0], vec![]).unwrap(),
            "fixture/sandbox",
        );
        let other = spec(
            "different bytes",
            &resources,
            OutputContract::new([0], vec![]).unwrap(),
            "fixture/sandbox",
        );
        let mut registry = LauncherRegistry::default();
        registry
            .register(Arc::new(FixtureLauncher {
                work: work.clone(),
                scratch: scratch.clone(),
            }))
            .unwrap();
        let mut adapter = LocalCheckAdapter::new(
            MemoryBackend::default(),
            Arc::new(RefusingProcess),
            registry,
            [installed],
            BTreeMap::from([
                ("work".into(), work.clone()),
                ("scratch".into(), scratch.clone()),
            ]),
        )
        .unwrap();
        assert!(matches!(
            adapter.run(
                &request(&other, "other"),
                Datum::String("operator".into()),
                1,
                2,
                &ProcessCancellation::default()
            ),
            Err(LocalCheckError::CommandNotInstalled)
        ));
        fs::remove_dir_all(work).unwrap();
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn process_receipt_is_not_mistaken_for_independent_cleanup_evidence() {
        struct Completing;
        impl ProcessPort for Completing {
            fn run(&self, _: &ProcessRequest, _: &ProcessCancellation) -> ProcessAttempt {
                ProcessAttempt::Completed {
                    receipt: ProcessReceipt {
                        provider: "fixture/process".into(),
                        elapsed_mono_ns: 1,
                        result: ProcResult {
                            stdout: String::new(),
                            stderr: String::new(),
                            exit_code: 0,
                            truncated: false,
                        },
                    },
                }
            }
        }
        let work = root("process-work");
        let resources = vec![CommandResource {
            source: "work".into(),
            guest_path: "/work".into(),
            access: ResourceAccess::Writable,
        }];
        let command = CommandSpec::new(
            ProgramRef::new("tool").unwrap(),
            ProjectRootRef::new("work").unwrap(),
            CommandInvocation::Argv(vec![]),
            SealedBindings::empty(),
            resources,
            ProcessBudget {
                timeout_ms: 1_000,
                max_output_bytes: 100,
                stdin: None,
            },
            OutputContract::new([0], vec![]).unwrap(),
            CleanupContract::process_group(BTreeSet::new()).unwrap(),
            NetworkAccess::Scoped(CapabilityName::new("network/test")),
            CommandRoute::Process,
            CommandReplayPolicy::ExactlyOnce,
        )
        .unwrap();
        let mut adapter = LocalCheckAdapter::new(
            MemoryBackend::default(),
            Arc::new(Completing),
            LauncherRegistry::default(),
            [command.clone()],
            BTreeMap::from([("work".into(), work.clone())]),
        )
        .unwrap();
        let process_request = request(&command, "process").with_network_grant(
            CapabilityName::new("network/test"),
            CapabilityGrantRef::new("grant/network-test").unwrap(),
        );
        assert!(matches!(
            adapter
                .run(
                    &process_request,
                    Datum::String("operator".into()),
                    1,
                    2,
                    &ProcessCancellation::default()
                )
                .unwrap(),
            OperationOutcome::Uncertain { .. }
        ));
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn real_bwrap_path_is_networkless_bounded_and_independently_observed() {
        let work = root("bwrap-work");
        let scratch = root("bwrap-scratch");
        let resources = vec![
            CommandResource {
                source: "work".into(),
                guest_path: "/work".into(),
                access: ResourceAccess::Writable,
            },
            CommandResource {
                source: "scratch".into(),
                guest_path: "/scratch".into(),
                access: ResourceAccess::Writable,
            },
            CommandResource {
                source: "usr".into(),
                guest_path: "/usr".into(),
                access: ResourceAccess::ReadOnly,
            },
            CommandResource {
                source: "lib".into(),
                guest_path: "/lib".into(),
                access: ResourceAccess::ReadOnly,
            },
            CommandResource {
                source: "lib64".into(),
                guest_path: "/lib64".into(),
                access: ResourceAccess::ReadOnly,
            },
        ];
        let expected = Datum::Bytes(b"checked\n".to_vec()).content_id().unwrap();
        let command = spec(
            "printf 'checked\\n' > result; printf 'temporary\\n' > /scratch/ephemeral",
            &resources,
            OutputContract::new(
                [0],
                vec![OutputExpectation {
                    resource: "work".into(),
                    relative_path: "result".into(),
                    state: OutputState::FileContent(expected),
                }],
            )
            .unwrap(),
            "platform/sandbox/ubuntu-bwrap",
        );
        let source_roots = BTreeMap::from([
            ("work".into(), work.clone()),
            ("scratch".into(), scratch.clone()),
            ("usr".into(), PathBuf::from("/usr")),
            ("lib".into(), PathBuf::from("/lib")),
            ("lib64".into(), PathBuf::from("/lib64")),
        ]);
        let launcher = crate::BwrapLauncher::new(
            PathBuf::from("/usr/bin/bwrap"),
            PathBuf::from("/usr/bin/prlimit"),
            BTreeMap::from([(ProgramRef::new("shell").unwrap(), PathBuf::from("/bin/sh"))]),
            source_roots.clone(),
        );
        let mut registry = LauncherRegistry::default();
        registry.register(Arc::new(launcher)).unwrap();
        let mut adapter = LocalCheckAdapter::new(
            MemoryBackend::default(),
            Arc::new(RefusingProcess),
            registry,
            [command.clone()],
            source_roots,
        )
        .unwrap();
        let outcome = adapter
            .run(
                &request(&command, "real-bwrap"),
                Datum::String("operator/bootstrap".into()),
                1,
                10,
                &ProcessCancellation::default(),
            )
            .unwrap();
        let record = adapter
            .record(&request(&command, "real-bwrap"))
            .unwrap()
            .unwrap();
        assert!(
            matches!(outcome, OperationOutcome::Verified { .. }),
            "{outcome:?}; receipts: {:?}",
            record.receipts()
        );
        assert_eq!(fs::read(work.join("result")).unwrap(), b"checked\n");
        assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
        assert_eq!(record.dispatches().len(), 1);
        assert_eq!(record.receipts().len(), 1);
        fs::remove_dir_all(work).unwrap();
        fs::remove_dir_all(scratch).unwrap();
    }
}
