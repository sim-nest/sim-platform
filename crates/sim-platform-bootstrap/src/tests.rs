use super::*;
use sim_kernel::{AbiVersion, LibManifest, LibSource, LibTarget, Symbol, Version};
use std::sync::{Arc, Mutex};

struct FakeLoader {
    paths: Arc<Mutex<Vec<PathBuf>>>,
}
struct FakeLib;
impl Lib for FakeLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("platform", "fictional"),
            version: Version("1.0.0".into()),
            abi: AbiVersion { major: 1, minor: 0 },
            target: LibTarget::Native,
            requires: vec![],
            capabilities: vec![],
            exports: vec![],
        }
    }
    fn load(
        &self,
        _: &mut sim_kernel::LoadCx,
        _: &mut sim_kernel::Linker<'_>,
    ) -> sim_kernel::Result<()> {
        Ok(())
    }
}
impl LibLoader for FakeLoader {
    fn can_load(&self, _: &LibSource) -> bool {
        true
    }
    fn load(&self, _: &mut Cx, source: LibSource) -> sim_kernel::Result<Box<dyn Lib>> {
        let path = sim_run_loaders::path_from_source(&source)
            .unwrap()
            .expect("bootstrap must supply a path");
        self.paths.lock().unwrap().push(path);
        Ok(Box::new(FakeLib))
    }
}

fn fixture(
    service: &str,
) -> (
    tempfile::TempDir,
    PathBuf,
    BootstrapEnvelope,
    BootstrapPolicy,
    FakeLoader,
) {
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("sim");
    fs::write(&executable, b"frame").unwrap();
    fs::write(dir.path().join("capsule.so"), b"fictional-native-capsule").unwrap();
    let digest = format!("sha256:{:x}", Sha256::digest(b"fictional-native-capsule"));
    fs::write(
        dir.path().join("capsule.toml"),
        format!(
            "schema = \"sim.platform-capsule/v1\"\nprovider = \"fictional-desktop\"\nservices = [\"{service}\"]\nloader_kinds = [\"loader/native-v1\"]\n"
        ),
    )
    .unwrap();
    fs::write(
        dir.path().join(BUNDLE_DESCRIPTOR_NAME),
        format!(
            "schema = \"sim.platform-bundle/v1\"\ncapsule = \"capsule.toml\"\nartifact = \"capsule.so\"\nloader = \"loader/native-v1\"\nartifact_content = \"{digest}\"\nentry = \"sim_native_abi_v1\"\n"
        ),
    )
    .unwrap();
    let envelope = BootstrapEnvelope {
        argv: vec!["sim".into()],
        stdio: BootstrapStdio::default(),
        bundle_identity: "pending".into(),
        capsule_card: CapsuleManifest {
            schema: "sim.platform-capsule/v1".into(),
            provider: "pending".into(),
            services: vec![],
            shells: vec![],
            loader_kinds: vec!["loader/native-v1".into()],
        },
        preopened_roots: vec![],
        config_roots: crate::BootstrapConfigRoots {
            home: None,
            work: dir.path().join("config"),
        },
        kernel_seed: 7,
    };
    let policy = BootstrapPolicy {
        allowed_services: BTreeSet::from([service.into()]),
    };
    let loader = FakeLoader {
        paths: Arc::new(Mutex::new(vec![])),
    };
    (dir, executable, envelope, policy, loader)
}

#[test]
fn admits_only_the_named_adjacent_content_bound_artifact() {
    let (_dir, executable, envelope, policy, loader) = fixture("platform/display");
    let (mut cx, _) = Cx::new_seated(
        Arc::new(sim_kernel::NoopEvalPolicy),
        Arc::new(sim_kernel::DefaultFactory),
    );
    let booted = bootstrap(&executable, envelope, &policy, &mut cx, &loader).unwrap();
    assert_eq!(booted.envelope.capsule_card.provider, "fictional-desktop");
    assert_eq!(
        loader.paths.lock().unwrap().as_slice(),
        &[executable.parent().unwrap().join("capsule.so")]
    );
}

#[test]
fn refuses_changed_and_over_capable_capsules_without_fallback() {
    let (dir, executable, envelope, policy, loader) = fixture("platform/display");
    fs::write(dir.path().join("capsule.so"), b"changed").unwrap();
    let (mut cx, _) = Cx::new_seated(
        Arc::new(sim_kernel::NoopEvalPolicy),
        Arc::new(sim_kernel::DefaultFactory),
    );
    assert!(matches!(
        bootstrap(&executable, envelope.clone(), &policy, &mut cx, &loader),
        Err(BootstrapError::ContentMismatch { .. })
    ));
    assert!(loader.paths.lock().unwrap().is_empty());
    let (_dir, executable, envelope, _, loader) = fixture("platform/secret-admin");
    assert!(matches!(
        bootstrap(
            &executable,
            envelope,
            &BootstrapPolicy {
                allowed_services: BTreeSet::new()
            },
            &mut cx,
            &loader
        ),
        Err(BootstrapError::OverCapable(_))
    ));
}

#[test]
fn fictional_desktop_is_entirely_fixture_data() {
    let source = include_str!("lib.rs");
    assert!(!source.contains("fictional-desktop"));
    assert!(!source.contains("platform/display"));
}

#[test]
fn bootstrap_source_has_no_ambient_or_domain_discovery() {
    let source = include_str!("lib.rs");
    for forbidden in [
        "current_",
        "std::env",
        "var_os",
        "PATH",
        "target_os",
        "reqwest",
    ] {
        assert!(
            !source.contains(forbidden),
            "forbidden ambient operation: {forbidden}"
        );
    }
}
