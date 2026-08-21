use sim_platform_core::{
    EvidenceLevel, OpenSymbol, PlatformRequest, RefusalKind, Requirement, platform_require,
};
use sim_platform_ubuntu_pc::{Architecture, ProfileKind, UbuntuPcProfile, linux::*, register};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone)]
struct TestPortal {
    cleanup: Arc<AtomicUsize>,
    permission: Permission,
}
impl Portal for TestPortal {
    fn call(&mut self, op: DesktopOperation) -> Result<PortalReply, PortalError> {
        match op {
            DesktopOperation::PermissionRequest(_) if self.permission == Permission::Denied => {
                Err(PortalError::Denied)
            }
            DesktopOperation::PermissionStatus(_) => Ok(PortalReply::Permission(self.permission)),
            _ => Ok(PortalReply::Accepted),
        }
    }
    fn cancel(&mut self, _: u64) {}
    fn cleanup(&mut self) {
        self.cleanup.fetch_add(1, Ordering::SeqCst);
    }
}
fn facts() -> HostFacts {
    HostFacts {
        locale: "sv-SE".into(),
        timezone: "Europe/Stockholm".into(),
        memory_bytes: 64 << 30,
        storage_bytes: 1 << 40,
        parallelism: 12,
        pressure: 7,
    }
}
fn mounts() -> XdgMounts {
    XdgMounts::new(
        PathBuf::from("config"),
        PathBuf::from("cache"),
        PathBuf::from("data"),
        PathBuf::from("state"),
        PathBuf::from("tmp"),
    )
}
fn budget() -> Budget {
    Budget {
        requests: 16,
        queue: 2,
        entropy_bytes: 16,
        timer_ns: 1_000,
    }
}

#[test]
fn registers_both_architectures_as_data_and_distinct_cards() {
    for architecture in [Architecture::X86_64, Architecture::Aarch64] {
        let desktop = register(UbuntuPcProfile {
            architecture,
            kind: ProfileKind::Desktop,
        });
        let headless = register(UbuntuPcProfile {
            architecture,
            kind: ProfileKind::Headless,
        });
        assert!(desktop.card.services.len() > headless.card.services.len());
        assert_eq!(desktop.profile.architecture, architecture);
        let req = PlatformRequest {
            request: OpenSymbol("request/notify".into()),
            requirements: vec![Requirement {
                service: OpenSymbol("platform/notify".into()),
                substitutes: vec![],
                optional: false,
                minimum_evidence: EvidenceLevel::Declared,
            }],
        };
        assert!(platform_require(&desktop.card, &req).is_ok());
        assert_eq!(
            platform_require(&headless.card, &req).unwrap_err().kind,
            RefusalKind::Unsupported
        );
    }
}
#[test]
fn clocks_seeded_entropy_locale_pressure_limits_and_preopened_mounts_work() {
    let mut capsule = Capsule::new(facts(), mounts(), Some(HeadlessPortal), budget(), 7);
    assert!(matches!(
        capsule.apply(1, Request::WallClock),
        Ok(Reply::Integer(_))
    ));
    assert!(matches!(
        capsule.apply(2, Request::MonotonicClock),
        Ok(Reply::Integer(_))
    ));
    assert!(matches!(
        capsule.apply(3, Request::Timer(10)),
        Ok(Reply::Integer(_))
    ));
    assert_eq!(
        capsule.apply(4, Request::Entropy(4)).unwrap(),
        Reply::Bytes(vec![199, 196, 31, 227])
    );
    assert_eq!(
        capsule.apply(5, Request::Locale).unwrap(),
        Reply::Text("sv-SE".into())
    );
    assert_eq!(
        capsule.apply(6, Request::Timezone).unwrap(),
        Reply::Text("Europe/Stockholm".into())
    );
    assert_eq!(
        capsule.apply(7, Request::Pressure).unwrap(),
        Reply::Integer(7)
    );
    assert!(matches!(
        capsule.apply(8, Request::Limits),
        Ok(Reply::Limits {
            parallelism: 12,
            ..
        })
    ));
    assert!(matches!(
        capsule.apply(9, Request::Mount(OpenSymbol("mount/xdg-config".into()))),
        Ok(Reply::Mount(_))
    ));
    assert!(
        capsule
            .apply(10, Request::Mount(OpenSymbol("mount/home".into())))
            .is_err()
    );
}
#[test]
fn denial_revocation_suspension_budgets_cancellation_queues_and_cleanup_fail_closed() {
    let cleaned = Arc::new(AtomicUsize::new(0));
    {
        let portal = TestPortal {
            cleanup: cleaned.clone(),
            permission: Permission::Denied,
        };
        let mut capsule = Capsule::new(
            facts(),
            mounts(),
            Some(portal),
            Budget {
                requests: 3,
                queue: 1,
                entropy_bytes: 1,
                timer_ns: 0,
            },
            9,
        );
        assert!(
            capsule
                .apply(
                    1,
                    Request::Desktop(DesktopOperation::PermissionRequest("camera".into()))
                )
                .is_err()
        );
        capsule.revoke(2);
        assert!(capsule.apply(2, Request::Locale).is_err());
        capsule.suspend();
        assert!(capsule.apply(3, Request::Locale).is_err());
        capsule.resume();
        assert!(capsule.apply(4, Request::Entropy(2)).is_err());
        assert!(capsule.apply(5, Request::Timer(1)).is_err());
    }
    assert_eq!(cleaned.load(Ordering::SeqCst), 1);
}
#[test]
fn bootstrap_descriptor_and_source_boundary_are_fixed() {
    assert_eq!(
        sim_platform_ubuntu_pc::BUNDLE_DESCRIPTOR,
        "sim.platform-bundle.toml"
    );
    let linux = include_str!("../../sim-platform-linux/src/lib.rs");
    for forbidden in [
        "Command::new",
        "std::process",
        "std::env",
        "/dev/",
        "/sys/",
        "/proc/",
    ] {
        assert!(
            !linux.contains(forbidden),
            "forbidden probe or shell: {forbidden}"
        );
    }
}
