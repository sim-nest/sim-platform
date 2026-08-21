use sim_platform_core::{parse_bundle, parse_capsule, validate_bundle};

#[test]
fn curated_bundles_admit_exactly_their_named_capsule() {
    let model = parse_bundle(sim_platform_model::MODEL_BUNDLE_MANIFEST).unwrap();
    let fictional = parse_capsule(sim_platform_model::FICTIONAL_CAPSULE).unwrap();
    validate_bundle(&model, &fictional).unwrap();
    let ubuntu = parse_bundle(sim_platform_ubuntu_pc::UBUNTU_BUNDLE_MANIFEST).unwrap();
    let ubuntu_capsule = parse_capsule(sim_platform_ubuntu_pc::UBUNTU_CAPSULE_MANIFEST).unwrap();
    validate_bundle(&ubuntu, &ubuntu_capsule).unwrap();
    assert!(validate_bundle(&model, &ubuntu_capsule).is_err());
    assert_ne!(ubuntu.capsule, fictional.provider);
}

#[test]
fn fictional_capsule_extension_does_not_change_bootloader_or_consumer() {
    let bundle = parse_bundle(sim_platform_model::MODEL_BUNDLE_MANIFEST).unwrap();
    assert_eq!(bundle.capsule, "platform/site/fictional");
    assert!(
        !include_str!("../../sim-platform-bootstrap/src/lib.rs")
            .contains("platform/site/fictional")
    );
}
