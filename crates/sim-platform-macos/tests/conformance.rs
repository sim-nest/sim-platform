use sim_platform_macos::*;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Probe {
    calls: Arc<Mutex<Vec<&'static str>>>,
}
impl NativeServices for Probe {
    fn permission_status(&mut self, _: Permission) -> PermissionStatus {
        self.calls.lock().unwrap().push("status");
        PermissionStatus::Undetermined
    }
    fn request_permission(&mut self, _: Permission) -> PermissionStatus {
        self.calls.lock().unwrap().push("prompt");
        PermissionStatus::Granted
    }
    fn activate(&mut self) -> Result<(), &'static str> {
        self.calls.lock().unwrap().push("activate");
        Ok(())
    }
    fn cleanup(&mut self) {
        self.calls.lock().unwrap().push("cleanup");
    }
}

#[test]
fn service_manifest_is_complete_and_prompt_boundary_is_singular() {
    let required = [
        "filesystem",
        "process",
        "loader",
        "socket",
        "lifecycle",
        "activation",
        "clipboard",
        "notification",
        "permission-status",
        "permission-request",
        "audio",
        "midi",
        "compute",
    ];
    for service in required {
        assert!(
            SERVICES.iter().any(|binding| binding.service == service),
            "missing {service}"
        );
    }
    let prompt = SERVICES
        .iter()
        .filter(|binding| binding.prompt == PromptPolicy::PermissionRequestOnly)
        .collect::<Vec<_>>();
    assert_eq!(prompt.len(), 1);
    assert_eq!(prompt[0].service, "permission-request");
}

#[test]
fn status_never_prompts_and_request_requires_matching_capability() {
    let probe = Probe::default();
    let calls = probe.calls.clone();
    let mut capsule = MacosCapsule::new(probe, Vec::<String>::new());
    assert_eq!(
        capsule.permission_status(Permission::Camera),
        Ok(PermissionStatus::Undetermined)
    );
    assert!(matches!(
        capsule.request_permission(Permission::Camera),
        Err(CapsuleError::MissingCapability(_))
    ));
    assert_eq!(&*calls.lock().unwrap(), &["status"]);
    drop(capsule);

    let probe = Probe::default();
    let calls = probe.calls.clone();
    let mut capsule = MacosCapsule::new(probe, ["platform/permission-request/camera".into()]);
    assert_eq!(
        capsule.request_permission(Permission::Camera),
        Ok(PermissionStatus::Granted)
    );
    assert_eq!(&*calls.lock().unwrap(), &["prompt"]);
}

#[test]
fn lifecycle_activation_denial_suspension_and_cleanup_fail_closed() {
    let probe = Probe::default();
    let calls = probe.calls.clone();
    let mut capsule = MacosCapsule::new(probe, Vec::<String>::new());
    capsule.activate().unwrap();
    assert_eq!(capsule.lifecycle(), Lifecycle::Active);
    capsule.suspend().unwrap();
    assert_eq!(
        capsule.permission_status(Permission::Camera),
        Err(CapsuleError::Suspended)
    );
    capsule.resume().unwrap();
    capsule.stop();
    assert_eq!(capsule.activate(), Err(CapsuleError::Stopped));
    drop(capsule);
    assert_eq!(&*calls.lock().unwrap(), &["activate", "cleanup"]);
}

fn input(architecture: MacArchitecture) -> CapsuleInput {
    CapsuleInput {
        architecture,
        rind: b"bootstrap-rind".to_vec(),
        capsule: b"content-bound-capsule".to_vec(),
        resources: vec![
            ("z.txt".into(), b"z".to_vec()),
            ("a.txt".into(), b"a".to_vec()),
        ],
    }
}

#[test]
fn cross_builds_both_architectures_as_deterministic_unsigned_bundles() {
    for architecture in [MacArchitecture::X86_64, MacArchitecture::Aarch64] {
        let first = build_unsigned_bundle(input(architecture));
        let second = build_unsigned_bundle(input(architecture));
        assert_eq!(first, second);
        assert_eq!(first.target, architecture.target());
        assert!(
            first
                .files
                .iter()
                .any(|file| file.path.ends_with(BUNDLE_DESCRIPTOR_NAME)
                    && String::from_utf8_lossy(&file.bytes).contains(NATIVE_ABI_ENTRY))
        );
        assert!(!serde_json::to_string(&first).unwrap().contains("signing"));
    }
    assert_ne!(
        build_unsigned_bundle(input(MacArchitecture::X86_64)).identity,
        build_unsigned_bundle(input(MacArchitecture::Aarch64)).identity
    );
}

#[test]
fn card_and_native_sources_keep_darwin_inside_capsule() {
    let card = platform_card(MacArchitecture::Aarch64);
    assert_eq!(card.site.0, "platform/site/macos");
    assert!(include_str!("../native/sim_native_abi.c").contains("sim_native_abi_v1"));
    assert!(include_str!("../native/Services.swift").contains("sim_macos_permission_request"));
    assert!(include_str!("../native/sim.entitlements").contains("app-sandbox"));
}
