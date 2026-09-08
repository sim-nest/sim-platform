use super::*;
use sim_lib_exec::{
    ArgAtom, BuildSourceRef, CapabilityGrantRef, CleanupContract, CommandInvocation,
    CommandReplayPolicy, CommandResource, MountAccess, NetworkAccess, OutputContract,
    OutputExpectation, PacketRef, ProcResult, ProcessBudget, ProcessReceipt, ProgramRef,
    ProjectRootRef, SandboxControl, SandboxEvidence, SandboxLimits, SandboxMount, SandboxPolicy,
    SandboxReport, SandboxRequirement, SandboxResult, SealedBindings,
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
