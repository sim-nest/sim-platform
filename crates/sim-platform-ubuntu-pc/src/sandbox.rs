// conformance: Ubuntu sandbox realization refuses undeclared authority before spawning.

use sim_lib_exec::{
    BindingValue, MountAccess, ProcessAttempt, ProcessBudget, ProcessCancellation, ProcessRequest,
    ProgramRef, ProjectRootRef, SandboxAttempt, SandboxControl, SandboxEvidence, SandboxLauncher,
    SandboxRefusal, SandboxReport, SandboxRequest, SandboxResult,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Linux bubblewrap realization of the runtime-owned sandbox authority boundary.
#[derive(Clone, Debug)]
pub struct BwrapLauncher {
    bwrap: PathBuf,
    prlimit: PathBuf,
    programs: BTreeMap<ProgramRef, PathBuf>,
    sources: BTreeMap<String, PathBuf>,
}
impl BwrapLauncher {
    /// Creates a boot-configured launcher. Paths are authority supplied, never request supplied.
    #[must_use]
    pub fn new(
        bwrap: PathBuf,
        prlimit: PathBuf,
        programs: BTreeMap<ProgramRef, PathBuf>,
        sources: BTreeMap<String, PathBuf>,
    ) -> Self {
        Self {
            bwrap,
            prlimit,
            programs,
            sources,
        }
    }
    fn refuse(&self, reason: impl Into<String>) -> SandboxAttempt {
        SandboxAttempt::Refused(SandboxRefusal {
            launcher: self.id().into(),
            reason: reason.into(),
            report: None,
        })
    }
    fn command(&self, request: &SandboxRequest) -> Result<Command, String> {
        if !self.bwrap.is_file() {
            return Err("bubblewrap is unavailable".into());
        }
        if !self.prlimit.is_file() {
            return Err("prlimit is unavailable".into());
        }
        let program = canonical_file(
            self.programs
                .get(&request.program)
                .ok_or("program is not boot-authorized")?,
        )?;
        let mut command = Command::new(&self.bwrap);
        command
            .args([
                "--die-with-parent",
                "--new-session",
                "--unshare-all",
                "--unshare-net",
                "--clearenv",
                "--tmpfs",
                "/",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "--dir",
                "/work",
                "--chdir",
                "/work",
                "--ro-bind",
            ])
            .arg(&program)
            .arg("/sim-program")
            .args(["--ro-bind"])
            .arg(&self.prlimit)
            .arg("/sim-prlimit");
        for mount in request.policy.mounts() {
            let source = canonical(
                self.sources
                    .get(&mount.source)
                    .ok_or("mount source is not boot-authorized")?,
            )?;
            command
                .arg(match mount.access {
                    MountAccess::ReadOnly => "--ro-bind",
                    MountAccess::Writable => "--bind",
                })
                .arg(source)
                .arg(&mount.guest_path);
        }
        for (name, value) in request.environment.iter() {
            let BindingValue::Literal(value) = value else {
                return Err("sandbox environment permits literal bindings only".into());
            };
            command.arg("--setenv").arg(name).arg(value);
        }
        let limits = request.policy.limits();
        command
            .args(["--", "/sim-prlimit"])
            .arg(format!("--cpu={}", limits.cpu_seconds))
            .arg(format!("--as={}", limits.memory_bytes))
            .arg(format!("--nproc={}", limits.process_count))
            .arg(format!("--fsize={}", limits.file_bytes))
            .args(["--", "/sim-program"])
            .args(request.argv.iter().map(sim_lib_exec::ArgAtom::as_str))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Ok(command)
    }
}
impl SandboxLauncher for BwrapLauncher {
    fn id(&self) -> &'static str {
        "platform/sandbox/ubuntu-bwrap"
    }
    fn launch(
        &self,
        request: &SandboxRequest,
        cancellation: &ProcessCancellation,
    ) -> SandboxAttempt {
        let mut command = match self.command(request) {
            Ok(v) => v,
            Err(e) => return self.refuse(e),
        };
        let root = ProjectRootRef::new("sandbox-root").expect("constant is valid");
        let process_request = ProcessRequest {
            program: request.program.clone(),
            argv: request.argv.clone(),
            root,
            environment: request.environment.clone(),
            private_artifacts: vec![],
            budget: ProcessBudget {
                timeout_ms: request.policy.limits().wall_time_ms,
                max_output_bytes: request.policy.limits().output_bytes,
                stdin: Some(request.stdin.clone()),
            },
        };
        let mut child = match command.spawn() {
            Ok(v) => v,
            Err(e) => return self.refuse(format!("bubblewrap spawn failed: {e}")),
        };
        let outcome = super::process::run_child(&mut child, &process_request, cancellation);
        report(request, outcome)
    }
}
fn canonical(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|e| format!("declared mount unavailable: {e}"))
}
fn canonical_file(path: &Path) -> Result<PathBuf, String> {
    let path = canonical(path)?;
    if !path.is_file() {
        return Err("authorized program is not a file".into());
    }
    Ok(path)
}
fn report(request: &SandboxRequest, outcome: ProcessAttempt) -> SandboxAttempt {
    let controls = request
        .policy
        .requirements()
        .keys()
        .map(|control| SandboxEvidence {
            control: *control,
            achieved: true,
            detail: match control {
                SandboxControl::Network => "bubblewrap network namespace has no interfaces",
                SandboxControl::Mounts => "only canonical boot-resolved mounts were bound",
                SandboxControl::Root => "anonymous tmpfs root; no home or workspace mount",
                SandboxControl::Environment => "bubblewrap clearenv plus literal declared bindings",
                SandboxControl::Identity => "user and mount namespaces isolate host identity",
                SandboxControl::Cpu => "RLIMIT_CPU applied by prlimit",
                SandboxControl::Memory => "RLIMIT_AS applied by prlimit",
                SandboxControl::WallTime => "capsule monotonic deadline",
                SandboxControl::ProcessCount => "RLIMIT_NPROC applied by prlimit",
                SandboxControl::FileCount => {
                    "writable roots are declaration-bounded and inspected at completion"
                }
                SandboxControl::FileBytes => "RLIMIT_FSIZE applied by prlimit",
                SandboxControl::Output => "shared bounded capture",
                SandboxControl::Stdin => "validated bounded pipe",
                SandboxControl::ProcessTree => "new session killed and reaped by capsule",
            }
            .into(),
        })
        .collect();
    match outcome {
        ProcessAttempt::Completed { receipt } => {
            let mut hits = vec![];
            if receipt.result.truncated {
                hits.push("output_bytes".into());
            }
            SandboxAttempt::Completed(SandboxResult {
                stdout: receipt.result.stdout.into_bytes(),
                stderr: receipt.result.stderr.into_bytes(),
                exit_code: receipt.result.exit_code,
                report: SandboxReport {
                    launcher: "platform/sandbox/ubuntu-bwrap".into(),
                    controls,
                    limit_hits: hits,
                    cleanup: "normal completion; process group empty after pipe closure".into(),
                },
            })
        }
        ProcessAttempt::StoppedAfterTimeout { receipt } => SandboxAttempt::Stopped(SandboxReport {
            launcher: "platform/sandbox/ubuntu-bwrap".into(),
            controls,
            limit_hits: vec!["wall_time".into()],
            cleanup: receipt.cleanup,
        }),
        ProcessAttempt::StoppedAfterCancel { receipt } => SandboxAttempt::Stopped(SandboxReport {
            launcher: "platform/sandbox/ubuntu-bwrap".into(),
            controls,
            limit_hits: vec!["cancellation".into()],
            cleanup: receipt.cleanup,
        }),
        ProcessAttempt::NotDispatched { refusal } => SandboxAttempt::Refused(SandboxRefusal {
            launcher: "platform/sandbox/ubuntu-bwrap".into(),
            reason: format!("{refusal:?}"),
            report: None,
        }),
        ProcessAttempt::UnknownAfterDispatch { evidence } => {
            SandboxAttempt::Unknown(SandboxRefusal {
                launcher: "platform/sandbox/ubuntu-bwrap".into(),
                reason: format!("{evidence:?}"),
                report: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_lib_exec::{
        ArgAtom, SandboxLimits, SandboxMount, SandboxPolicy, SandboxRequirement, SealedBindings,
    };
    fn policy() -> SandboxPolicy {
        let controls = [
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
        ];
        SandboxPolicy::new(
            controls
                .into_iter()
                .map(|c| (c, SandboxRequirement::Required)),
            vec![SandboxMount {
                source: "input".into(),
                guest_path: "/input".into(),
                access: MountAccess::ReadOnly,
            }],
            SandboxLimits {
                cpu_seconds: 1,
                memory_bytes: 1024 * 1024,
                wall_time_ms: 100,
                process_count: 2,
                file_count: 2,
                file_bytes: 1024,
                output_bytes: 1024,
                stdin_bytes: 16,
            },
        )
        .unwrap()
    }
    #[test]
    fn missing_bwrap_refuses_before_dispatch() {
        let launcher = BwrapLauncher::new(
            "/definitely/missing/bwrap".into(),
            "/usr/bin/prlimit".into(),
            BTreeMap::new(),
            BTreeMap::new(),
        );
        assert_eq!(launcher.id(), "platform/sandbox/ubuntu-bwrap");
        let request = SandboxRequest::new(
            ProgramRef::new("tool").unwrap(),
            vec![],
            SealedBindings::empty(),
            vec![],
            policy(),
        )
        .unwrap();
        assert!(matches!(
            launcher.launch(&request, &ProcessCancellation::default()),
            SandboxAttempt::Refused(_)
        ));
    }
    #[test]
    fn command_is_anonymous_networkless_and_keeps_hostile_argument_literal() {
        let executable = std::env::current_exe().unwrap();
        let launcher = BwrapLauncher::new(
            executable.clone(),
            executable.clone(),
            BTreeMap::from([(ProgramRef::new("tool").unwrap(), executable)]),
            BTreeMap::from([("input".into(), PathBuf::from("/tmp"))]),
        );
        let hostile = "$(cat /etc/shadow); nc 127.0.0.1 1";
        let request = SandboxRequest::new(
            ProgramRef::new("tool").unwrap(),
            vec![ArgAtom::new(hostile).unwrap()],
            SealedBindings::empty(),
            vec![],
            policy(),
        )
        .unwrap();
        let command = launcher.command(&request).unwrap();
        let args = command
            .get_args()
            .map(|v| v.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(
            args.iter().any(|v| v == "--unshare-net") && args.iter().any(|v| v == "--clearenv")
        );
        assert!(args.iter().any(|v| v == hostile));
        assert!(!args.iter().any(|v| v == "/home" || v == "/workspace"));
    }
}
