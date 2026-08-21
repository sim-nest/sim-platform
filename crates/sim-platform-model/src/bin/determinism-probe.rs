use sim_platform_core::{FactPort, Lifecycle, OpenSymbol};
use sim_platform_model::{LifecycleStep, MachineLimits, ModelConfig, ModelRequest, SeededPlatform};
use std::collections::BTreeMap;

fn main() {
    let model = SeededPlatform::new(ModelConfig {
        seed: 42,
        wall_epoch_ns: 1_700_000_000_000_000_000,
        locale: "en-SE".into(),
        timezone: "Europe/Stockholm".into(),
        limits: MachineLimits {
            memory_bytes: 1 << 30,
            storage_bytes: 1 << 32,
            parallelism: 4,
        },
        lifecycle: vec![LifecycleStep {
            at_mono_ns: 10,
            state: Lifecycle::Pressured,
        }],
        mounts: BTreeMap::from([(OpenSymbol("mount/fixture".into()), b"fixture".to_vec())]),
        fault: None,
    });
    let stream = [
        ModelRequest::ReadFact(FactPort::WallClock),
        ModelRequest::Entropy { bytes: 16 },
        ModelRequest::AdvanceTime { nanos: 10 },
        ModelRequest::Lifecycle,
        ModelRequest::ReadMount {
            mount: OpenSymbol("mount/fixture".into()),
        },
    ];
    let replies: Vec<_> = stream
        .iter()
        .map(|request| model.apply(request).unwrap())
        .collect();
    println!("{}", serde_json::to_string(&replies).unwrap());
}
