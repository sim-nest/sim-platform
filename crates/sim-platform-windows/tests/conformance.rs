use sim_platform_windows::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
struct Probe {
    calls: Arc<Mutex<Vec<&'static str>>>,
}
impl NativeServices for Probe {
    fn dispatch(&mut self, request: NativeRequest) -> Result<(), &'static str> {
        self.calls.lock().unwrap().push(match request {
            NativeRequest::Launch => "launch",
            NativeRequest::Activate => "activate",
            NativeRequest::Audio => "audio",
            NativeRequest::Midi => "midi",
            NativeRequest::Compute => "compute",
            NativeRequest::PermissionRequest => "permission-request",
            _ => "other",
        });
        Ok(())
    }
    fn cancel_process_tree(&mut self) {
        self.calls.lock().unwrap().push("cancel-tree");
    }
    fn cleanup(&mut self) {
        self.calls.lock().unwrap().push("cleanup");
    }
}
fn profile(grants: &[&str]) -> WindowsProfile {
    WindowsProfile {
        preopened_dirs: BTreeMap::from([(
            "workspace".into(),
            r"C:\SIM\Workspace".encode_utf16().collect(),
        )]),
        grants: grants.iter().map(|s| (*s).into()).collect::<BTreeSet<_>>(),
        max_processes: 8,
        max_output_bytes: 65_536,
    }
}

#[test]
fn card_and_api_map_cover_every_required_service_without_portable_os_branch() {
    let required = [
        "filesystem",
        "process",
        "loader",
        "socket",
        "lifecycle",
        "activation",
        "permission-status",
        "permission-request",
        "clipboard",
        "notification",
        "audio",
        "midi",
        "compute",
    ];
    assert_eq!(
        required,
        SERVICES
            .iter()
            .map(|s| s.service)
            .collect::<Vec<_>>()
            .as_slice()
    );
    assert_eq!(platform_card().site.0, "platform/site/windows");
    assert!(
        SERVICES
            .iter()
            .all(|s| !s.api.is_empty() && !s.permission.is_empty())
    );
    let portable = [
        include_str!("../../sim-platform-core/src/lib.rs"),
        include_str!("../../sim-lib-platform/src/lib.rs"),
    ]
    .join("\n");
    assert!(!portable.contains(".exe"));
    assert!(!portable.contains("WindowsProfile"));
}

#[test]
fn path_boundary_preserves_identity_and_contains_windows_spelling() {
    let capsule = WindowsCapsule::new(Probe::default(), profile(&[])).unwrap();
    let forward: Vec<u16> = "models/input.bin".encode_utf16().collect();
    let path = capsule.resolve_preopen("workspace", &forward).unwrap();
    assert_eq!(
        String::from_utf16(path.api_units()).unwrap(),
        r"models\input.bin"
    );
    assert_ne!(path.identity(), "");
    let traversal: Vec<u16> = "..\\secret".encode_utf16().collect();
    assert!(capsule.resolve_preopen("workspace", &traversal).is_err());
    let drive: Vec<u16> = r"D:\escape".encode_utf16().collect();
    assert!(capsule.resolve_preopen("workspace", &drive).is_err());
}

#[test]
fn hostile_profiles_fail_closed_before_native_dispatch() {
    assert!(matches!(
        WindowsCapsule::new(
            Probe::default(),
            WindowsProfile {
                max_processes: u32::MAX,
                ..profile(&[])
            }
        ),
        Err(CapsuleError::InvalidBudget)
    ));
    let probe = Probe::default();
    let calls = probe.calls.clone();
    let mut capsule =
        WindowsCapsule::new(probe, profile(&["platform/activation", "platform/process"])).unwrap();
    assert!(matches!(
        capsule.request("audio", NativeRequest::Audio),
        Err(CapsuleError::MissingCapability(_))
    ));
    capsule.activate().unwrap();
    capsule.request("process", NativeRequest::Launch).unwrap();
    capsule.cancel_process_tree().unwrap();
    capsule.suspend().unwrap();
    assert_eq!(
        capsule.request("process", NativeRequest::Launch),
        Err(CapsuleError::Suspended)
    );
    capsule.stop();
    assert_eq!(
        &*calls.lock().unwrap(),
        &["activate", "launch", "cancel-tree", "cleanup"]
    );
}

#[test]
fn package_is_content_bound_and_shell_enters_only_the_existing_abi() {
    let first = build_unsigned_package(b"rind".to_vec(), b"capsule".to_vec());
    let second = build_unsigned_package(b"rind".to_vec(), b"capsule".to_vec());
    assert_eq!(first, second);
    assert_eq!(first.target, TARGET);
    let shell = include_str!("../native/sim_native_abi.c");
    assert!(shell.contains(NATIVE_ABI_ENTRY) && shell.contains("sim_bootstrap_rind"));
    assert!(!shell.contains("LoadLibrary"));
    let permissions = include_str!("../package/AppxManifest.xml");
    for capability in [
        "runFullTrust",
        "broadFileSystemAccess",
        "microphone",
        "midi",
    ] {
        assert!(permissions.contains(capability));
    }
}

#[test]
fn recipes_derive_their_expected_service_set_from_the_card() {
    let generated = generated_service_set();
    assert_eq!(
        include_str!("../recipes/generated-services.txt").trim(),
        generated
    );
    for recipe in ["launch", "process-cancellation", "media"] {
        assert!(include_str!("../recipes/01-basics/chapter.toml").contains(recipe));
    }
}

#[test]
fn import_attestation_is_cross_build_only_and_allowlisted() {
    let value: serde_json::Value =
        serde_json::from_str(include_str!("../attestations/x86_64-pc-windows-msvc.json")).unwrap();
    assert_eq!(value["evidence"], "cross-built");
    assert_eq!(value["execution"], "not-claimed");
    let allowed = [
        "KERNEL32.dll",
        "USER32.dll",
        "OLE32.dll",
        "WS2_32.dll",
        "MMDevAPI.dll",
        "D3D12.dll",
        "DXGI.dll",
    ];
    assert!(
        value["imports"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| allowed.contains(&v.as_str().unwrap()))
    );
}
