//! conformance: real owner commands cross the networkless durable local-check adapter.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use sim_kernel::Datum;
use sim_lib_exec::{
    ArgAtom, BuildSourceRef, CapabilityGrantRef, CleanupContract, CommandInvocation,
    CommandReplayPolicy, CommandResource, CommandRoute, CommandSpec, LauncherRegistry,
    LocalCheckRequest, MountAccess, NetworkAccess, OutputContract, OutputExpectation, OutputState,
    PacketRef, ProcessAttempt, ProcessBudget, ProcessCancellation, ProcessPort, ProcessRefusal,
    ProcessRequest, ProgramRef, ProjectRootRef, ResourceAccess, SandboxControl, SandboxLimits,
    SandboxMount, SandboxPolicy, SandboxRequirement, SealedBindings,
};
use sim_lib_journal::MemoryBackend;
use sim_lib_operation_gate::OperationOutcome;
use sim_platform_ubuntu_pc::{BwrapLauncher, LocalCheckAdapter};

const FORMAT_COMMAND: &str = "cargo fmt --all";
const FAILURE_COMMAND: &str = "cargo test --workspace --features deliberate-failure";
const VALIDATION_COMMAND: &str = "cargo fmt --all --check && cargo test --workspace";
const DOCS_COMMAND: &str = "cargo doc --workspace --no-deps";
const FORMATTED_SOURCE: &str = r#"pub fn answer() -> u32 {
    42
}
#[cfg(all(test, feature = "deliberate-failure"))]
mod tests {
    #[test]
    fn deliberate_failure() {
        assert_eq!(1, 2);
    }
}
"#;

#[derive(Default)]
struct RefusingProcess;

impl ProcessPort for RefusingProcess {
    fn run(&self, _: &ProcessRequest, _: &ProcessCancellation) -> ProcessAttempt {
        ProcessAttempt::NotDispatched {
            refusal: ProcessRefusal::Refused("sandbox route required".into()),
        }
    }
}

struct Fixture {
    root: PathBuf,
    work: PathBuf,
    scratch: PathBuf,
    cargo_home: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "sim-local-check-specimen-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is after epoch")
                .as_nanos()
        ));
        let work = root.join("work");
        let scratch = root.join("scratch");
        let cargo_home = root.join("cargo-home");
        fs::create_dir_all(work.join("src")).expect("create fixture source root");
        fs::create_dir_all(&scratch).expect("create fixture scratch root");
        fs::create_dir_all(&cargo_home).expect("create fixture Cargo home");
        fs::write(
            work.join("Cargo.toml"),
            r#"[package]
name = "local-check-owner"
version = "0.1.0"
edition = "2024"

[features]
deliberate-failure = []
"#,
        )
        .expect("write fixture manifest");
        fs::write(
            work.join("sim-owner.toml"),
            format!(
                "validation_command = {VALIDATION_COMMAND:?}\ndocs_command = {DOCS_COMMAND:?}\n"
            ),
        )
        .expect("write exact owner commands");
        fs::write(
            work.join("src/lib.rs"),
            "pub fn answer( )->u32{42}\n#[cfg(all(test,feature=\"deliberate-failure\"))]mod tests{#[test]fn deliberate_failure(){assert_eq!(1,2);}}\n",
        )
        .expect("write deliberately unformatted source");
        Self {
            root,
            work,
            scratch,
            cargo_home,
        }
    }

    fn prepare_scratch(&self) {
        fs::create_dir_all(self.scratch.join("tmp")).expect("create bounded temp root");
        fs::create_dir_all(self.scratch.join("home")).expect("create bounded home root");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn toolchain_root() -> PathBuf {
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .expect("query the bootstrap toolchain root");
    assert!(output.status.success(), "rustc --print sysroot failed");
    PathBuf::from(
        std::str::from_utf8(&output.stdout)
            .expect("toolchain root is UTF-8")
            .trim(),
    )
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

fn resources(include_lib64: bool) -> Vec<CommandResource> {
    let mut resources = vec![
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
            source: "cargo-home".into(),
            guest_path: "/cargo-home".into(),
            access: ResourceAccess::Writable,
        },
        CommandResource {
            source: "toolchain".into(),
            guest_path: "/toolchain".into(),
            access: ResourceAccess::ReadOnly,
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
    ];
    if include_lib64 {
        resources.push(CommandResource {
            source: "lib64".into(),
            guest_path: "/lib64".into(),
            access: ResourceAccess::ReadOnly,
        });
    }
    resources
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
            cpu_seconds: 120,
            memory_bytes: 4 * 1024 * 1024 * 1024,
            wall_time_ms: 120_000,
            process_count: 128,
            file_count: 50_000,
            file_bytes: 1024 * 1024 * 1024,
            output_bytes: 1024 * 1024,
            stdin_bytes: 1,
        },
    )
    .expect("complete local-check sandbox policy")
}

fn command(
    script: &'static str,
    accepted_exit: i32,
    outputs: Vec<OutputExpectation>,
    resources: &[CommandResource],
) -> CommandSpec {
    CommandSpec::new(
        ProgramRef::new("owner-shell").expect("constant program identity"),
        ProjectRootRef::new("work").expect("constant project identity"),
        CommandInvocation::Interpreter {
            flags: vec![ArgAtom::new("-c").expect("constant shell flag")],
            script: script.as_bytes().to_vec(),
        },
        SealedBindings::literals([
            ("CC".into(), "/usr/bin/gcc".into()),
            ("CARGO_HOME".into(), "/cargo-home".into()),
            ("CARGO_TARGET_DIR".into(), "/scratch/target".into()),
            ("CARGO_TERM_COLOR".into(), "never".into()),
            ("HOME".into(), "/scratch/home".into()),
            ("PATH".into(), "/toolchain/bin:/usr/bin:/bin".into()),
            ("RUSTC".into(), "/toolchain/bin/rustc".into()),
            ("RUSTDOC".into(), "/toolchain/bin/rustdoc".into()),
            ("RUSTFLAGS".into(), "-C linker=/usr/bin/gcc".into()),
            ("TMPDIR".into(), "/scratch/tmp".into()),
        ])
        .expect("bounded sealed environment"),
        resources.to_vec(),
        ProcessBudget {
            timeout_ms: 120_000,
            max_output_bytes: 1024 * 1024,
            stdin: None,
        },
        OutputContract::new([accepted_exit], outputs).expect("exact output contract"),
        CleanupContract::process_group(["scratch".into()]).expect("scratch cleanup contract"),
        NetworkAccess::Absent,
        CommandRoute::Sandbox {
            launcher: "platform/sandbox/ubuntu-bwrap".into(),
            policy: policy(resources),
        },
        CommandReplayPolicy::ExactlyOnce,
    )
    .expect("valid exact command")
}

fn request(command: &CommandSpec, name: &str) -> LocalCheckRequest {
    LocalCheckRequest::new(
        PacketRef::new(format!("packet/nv12.05/{name}")).expect("packet identity"),
        command.id().clone(),
        BuildSourceRef::new("source/local-check-owner-v1").expect("source identity"),
        CapabilityGrantRef::new("grant/local-check-owner-v1").expect("grant identity"),
    )
}

fn assert_verified(
    adapter: &mut LocalCheckAdapter<MemoryBackend>,
    command: &CommandSpec,
    name: &str,
    tick: u64,
) {
    let outcome = adapter
        .run(
            &request(command, name),
            Datum::String("operator/bootstrap-observer".into()),
            tick,
            tick + 100,
            &ProcessCancellation::default(),
        )
        .expect("local command admitted");
    let record = adapter
        .record(&request(command, name))
        .expect("reconstruct command lifecycle")
        .expect("command lifecycle exists");
    assert!(
        matches!(outcome, OperationOutcome::Verified { .. }),
        "{name} did not verify: {outcome:?}; receipts: {:?}",
        record.receipts()
    );
}

fn resource_roots(
    fixture: &Fixture,
    toolchain: PathBuf,
    include_lib64: bool,
) -> BTreeMap<String, PathBuf> {
    [
        ("work".into(), fixture.work.clone()),
        ("scratch".into(), fixture.scratch.clone()),
        ("cargo-home".into(), fixture.cargo_home.clone()),
        ("toolchain".into(), toolchain),
        ("usr".into(), PathBuf::from("/usr")),
        ("lib".into(), PathBuf::from("/lib")),
    ]
    .into_iter()
    .chain(include_lib64.then(|| ("lib64".into(), PathBuf::from("/lib64"))))
    .collect()
}

fn assert_scratch_empty(fixture: &Fixture) {
    assert_eq!(
        fs::read_dir(&fixture.scratch)
            .expect("inspect scratch")
            .count(),
        0
    );
}

fn assert_owner_command_bytes(validation: &CommandSpec, docs: &CommandSpec, fixture: &Fixture) {
    let owner_manifest = fs::read_to_string(fixture.work.join("sim-owner.toml"))
        .expect("read owner command manifest");
    assert!(owner_manifest.contains(&format!("validation_command = {VALIDATION_COMMAND:?}")));
    assert!(owner_manifest.contains(&format!("docs_command = {DOCS_COMMAND:?}")));
    for (command, script) in [(validation, VALIDATION_COMMAND), (docs, DOCS_COMMAND)] {
        let CommandInvocation::Interpreter {
            script: installed, ..
        } = command.invocation()
        else {
            panic!("owner command must remain an interpreter script");
        };
        assert_eq!(installed, script.as_bytes(), "owner bytes changed");
    }
}

#[test]
fn exact_owner_commands_run_through_the_networkless_durable_adapter() {
    let fixture = Fixture::new();
    let toolchain = toolchain_root();
    let include_lib64 = Path::new("/lib64").is_dir();
    let resources = resources(include_lib64);
    let roots = resource_roots(&fixture, toolchain, include_lib64);
    let formatted_id = Datum::Bytes(FORMATTED_SOURCE.as_bytes().to_vec())
        .content_id()
        .expect("formatted source has a semantic identity");
    let format = command(
        FORMAT_COMMAND,
        0,
        vec![OutputExpectation {
            resource: "work".into(),
            relative_path: "src/lib.rs".into(),
            state: OutputState::FileContent(formatted_id),
        }],
        &resources,
    );
    let deliberate_failure = command(FAILURE_COMMAND, 101, vec![], &resources);
    let validation = command(VALIDATION_COMMAND, 0, vec![], &resources);
    let docs = command(DOCS_COMMAND, 0, vec![], &resources);
    let mut registry = LauncherRegistry::default();
    registry
        .register(Arc::new(BwrapLauncher::new(
            PathBuf::from("/usr/bin/bwrap"),
            PathBuf::from("/usr/bin/prlimit"),
            BTreeMap::from([(
                ProgramRef::new("owner-shell").expect("program identity"),
                PathBuf::from("/bin/sh"),
            )]),
            roots.clone(),
        )))
        .expect("register Ubuntu sandbox launcher");
    let mut adapter = LocalCheckAdapter::new(
        MemoryBackend::default(),
        Arc::new(RefusingProcess),
        registry,
        [
            format.clone(),
            deliberate_failure.clone(),
            validation.clone(),
            docs.clone(),
        ],
        roots,
    )
    .expect("construct local-check adapter");

    let before = fs::read(fixture.work.join("src/lib.rs")).expect("read initial source");
    fixture.prepare_scratch();
    assert_verified(&mut adapter, &format, "format", 1);
    let after = fs::read(fixture.work.join("src/lib.rs")).expect("read formatted source");
    assert_ne!(
        before, after,
        "formatter mutation must be independently visible"
    );
    assert_eq!(after, FORMATTED_SOURCE.as_bytes());
    assert_scratch_empty(&fixture);

    fixture.prepare_scratch();
    assert_verified(&mut adapter, &deliberate_failure, "deliberate-failure", 201);
    assert_scratch_empty(&fixture);

    fixture.prepare_scratch();
    assert_verified(&mut adapter, &validation, "validation", 401);
    assert_scratch_empty(&fixture);

    fixture.prepare_scratch();
    assert_verified(&mut adapter, &docs, "docs", 601);
    assert_scratch_empty(&fixture);
    assert_owner_command_bytes(&validation, &docs, &fixture);
}
