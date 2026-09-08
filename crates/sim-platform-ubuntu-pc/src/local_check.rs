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

mod evidence;

use evidence::{
    node, outcome_evidence, record_process, record_sandbox, refusal_datum,
    validate_command_resources,
};

#[cfg(test)]
mod tests;
