use sim_kernel::{
    Consistency, Cx, DefaultFactory, EagerPolicy, EvalFabric, EvalMode, EvalRequest, Expr, Symbol,
};
use sim_platform_core::{FactPort, Lifecycle, OpenSymbol, RefusalKind};
use sim_platform_model::{
    InjectedFault, LifecycleStep, MachineLimits, ModelConfig, ModelRequest, PlatformService,
    PlatformSite, SeededPlatform,
};
use std::{collections::BTreeMap, process::Command, sync::Arc};

fn config() -> ModelConfig {
    ModelConfig {
        seed: 7,
        wall_epoch_ns: 100,
        locale: "sv-SE".into(),
        timezone: "Europe/Stockholm".into(),
        limits: MachineLimits {
            memory_bytes: 1024,
            storage_bytes: 2048,
            parallelism: 2,
        },
        lifecycle: vec![LifecycleStep {
            at_mono_ns: 5,
            state: Lifecycle::Pressured,
        }],
        mounts: BTreeMap::from([(OpenSymbol("mount/data".into()), vec![1, 2, 3])]),
        fault: None,
    }
}

#[test]
fn model_controls_time_entropy_mounts_lifecycle_limits_and_faults() {
    let model = SeededPlatform::new(config());
    assert_eq!(
        model
            .apply(&ModelRequest::ReadFact(FactPort::WallClock))
            .unwrap()
            .result,
        sim_platform_model::ModelResult::Integer(100)
    );
    model
        .apply(&ModelRequest::AdvanceTime { nanos: 5 })
        .unwrap();
    assert_eq!(
        model.apply(&ModelRequest::Lifecycle).unwrap().result,
        sim_platform_model::ModelResult::Lifecycle(Lifecycle::Ready)
    );
    assert_eq!(
        model
            .apply(&ModelRequest::ReadMount {
                mount: OpenSymbol("mount/data".into())
            })
            .unwrap()
            .result,
        sim_platform_model::ModelResult::Bytes(vec![1, 2, 3])
    );
    for fault in [
        InjectedFault::Unsupported,
        InjectedFault::InsufficientEvidence,
        InjectedFault::Timer,
        InjectedFault::Entropy,
        InjectedFault::Mount,
        InjectedFault::Lifecycle,
        InjectedFault::Limits,
    ] {
        let mut cfg = config();
        cfg.fault = Some(fault.clone());
        let model = SeededPlatform::new(cfg);
        let request = match fault {
            InjectedFault::Timer => ModelRequest::AdvanceTime { nanos: 1 },
            InjectedFault::Entropy => ModelRequest::Entropy { bytes: 1 },
            InjectedFault::Mount => ModelRequest::ReadMount {
                mount: OpenSymbol("mount/data".into()),
            },
            InjectedFault::Lifecycle => ModelRequest::Lifecycle,
            InjectedFault::Limits => ModelRequest::Limits,
            _ => ModelRequest::ReadFact(FactPort::WallClock),
        };
        assert!(model.apply(&request).is_err(), "fault {fault:?}");
    }
}

#[test]
fn site_binds_only_explicit_services_and_shape_checks_result() {
    let model = SeededPlatform::new(config());
    let service_symbol = Symbol::qualified("platform", "wall-clock");
    let service = Arc::new(PlatformService {
        symbol: service_symbol.clone(),
        model,
    });
    let site = PlatformSite::new(
        Symbol::qualified("platform", "site-model"),
        BTreeMap::from([(service_symbol.clone(), service)]),
        vec![],
    );
    let mut cx = Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(0x504c_4154),
    );
    let request = EvalRequest {
        expr: Expr::Symbol(service_symbol),
        result_shape: None,
        required_capabilities: vec![],
        deadline: None,
        consistency: Consistency::LocalOnly,
        mode: EvalMode::Eval,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: false,
    };
    assert!(
        site.realize(&mut cx, request)
            .unwrap()
            .value
            .object()
            .downcast_ref::<PlatformService>()
            .is_some()
    );
    let missing = EvalRequest {
        expr: Expr::Symbol(Symbol::qualified("platform", "missing")),
        result_shape: None,
        required_capabilities: vec![],
        deadline: None,
        consistency: Consistency::LocalOnly,
        mode: EvalMode::Eval,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: false,
    };
    assert!(site.realize(&mut cx, missing).is_err());
}

#[test]
fn identical_stream_is_byte_identical_in_two_processes() {
    let executable = env!("CARGO_BIN_EXE_determinism-probe");
    let first = Command::new(executable).output().unwrap();
    let second = Command::new(executable).output().unwrap();
    assert!(first.status.success() && second.status.success());
    assert_eq!(first.stdout, second.stdout);
    let replies: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert!(
        replies
            .as_array()
            .unwrap()
            .iter()
            .all(|reply| reply.get("receipt").is_some()
                && reply.get("evidence").unwrap().get("ledger_ref").is_some())
    );
}

#[test]
fn unbound_service_is_unsupported() {
    let model = SeededPlatform::new(config());
    let refusal = model
        .apply(&ModelRequest::ReadMount {
            mount: OpenSymbol("mount/unbound".into()),
        })
        .unwrap_err();
    assert_eq!(refusal.kind, RefusalKind::Unsupported);
}
