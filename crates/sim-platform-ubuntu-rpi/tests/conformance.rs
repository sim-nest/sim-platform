use sim_platform_ubuntu_rpi::{
    Binding, BuildEvidence, PiRefusal, PiService, TargetAttestation, hostile_model_profile,
    register,
};
use std::collections::BTreeSet;

#[test]
fn hostile_profile_observed_services_exactly_equal_card() {
    let profile = hostile_model_profile();
    let observed = PiService::ALL
        .into_iter()
        .filter(|service| profile.require(*service).is_ok())
        .map(|service| service.symbol())
        .collect::<BTreeSet<_>>();
    let registered = register(profile.clone());
    let card = registered
        .card
        .services
        .iter()
        .map(|offer| offer.service.0.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(observed, card);
    assert_eq!(
        profile.require(PiService::Gpio),
        Ok(&Binding::Api("model/gpio-v1".into()))
    );
    for absent in PiService::ALL
        .into_iter()
        .filter(|service| *service != PiService::Gpio)
    {
        assert_eq!(
            profile.require(absent),
            Err(PiRefusal::Unsupported { service: absent })
        );
    }
}

#[test]
fn ubuntu_derivation_does_not_inherit_linux_services() {
    let profile = sim_platform_ubuntu_rpi::UbuntuRpiProfile::from_ubuntu_headless("c03115");
    assert!(profile.bindings.is_empty());
    assert!(profile.permissions.is_empty());
    assert_eq!(register(profile).card.services, Vec::new());
}

#[test]
fn evidence_cannot_claim_unregistered_physical_testing() {
    let cross: TargetAttestation = serde_json::from_str(include_str!(
        "../attestations/aarch64-unknown-linux-gnu.json"
    ))
    .expect("committed cross-build attestation decodes");
    assert_eq!(cross.targets, ["aarch64-unknown-linux-gnu"]);
    assert_eq!(cross.evidence, BuildEvidence::CrossBuilt);
    assert_eq!(cross.registered_host, None);
    assert_eq!(cross.validate(), Ok(()));
    let false_physical = TargetAttestation {
        evidence: BuildEvidence::Physical,
        ..cross
    };
    assert!(false_physical.validate().is_err());
}

#[test]
fn pure_product_closure_contains_no_pi_source_fact() {
    for source in [
        include_str!("../../sim-lib-platform/Cargo.toml"),
        include_str!("../../sim-platform-model/Cargo.toml"),
        include_str!("../../sim-platform-core/Cargo.toml"),
    ] {
        assert!(!source.contains("sim-platform-ubuntu-rpi"));
        assert!(!source.contains("raspberry"));
        assert!(!source.contains("/dev/"));
    }
}
