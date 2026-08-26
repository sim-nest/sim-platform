// conformance: process realization evidence is sanitized and independently verifiable.

use sim_platform_ubuntu_pc::PhysicalAttestation;
#[test]
fn committed_physical_attestation_is_sanitized_and_offline_verifiable() {
    let source = include_str!("../../../acceptance/ubuntu-pc.sx");
    for forbidden in ["/home/", "hostname", "username", "serial", "@"] {
        assert!(!source.contains(forbidden));
    }
    let value = PhysicalAttestation {
        schema: "sim.platform-physical-attestation/v1".into(),
        provider: "ubuntu-pc".into(),
        registered_capability: "linux-x86_64".into(),
        source_content: "fnv1a64:41fdd80c770ae9f2".into(),
        artifact_content: "fnv1a64:018c6b634aafe833".into(),
        card_content: "fnv1a64:4be8e55bd66dd4d9".into(),
        result_content: "fnv1a64:6fc0b36648cf320d".into(),
        source: "ubuntu-25.10|x86_64|registered-control-local".into(),
        artifact: "sim-platform-ubuntu-pc|workspace-test-artifact".into(),
        card: "ubuntu-pc|x86_64|desktop+headless".into(),
        result: "clock+entropy+mount+headless-unsupported+cleanup|pass".into(),
        checks: vec![
            "clock".into(),
            "entropy".into(),
            "mount".into(),
            "cleanup".into(),
        ],
    };
    value.validate().unwrap();
}
