use std::collections::{BTreeMap, BTreeSet};

use sim_platform_core::*;

fn symbol(value: &str) -> OpenSymbol {
    OpenSymbol::new(value).unwrap()
}

fn card(site: &str) -> PlatformCard {
    PlatformCard {
        schema: symbol("platform/card/v1"),
        site: symbol(site),
        services: vec![ServiceOffer {
            service: symbol("platform/service/portable-eval"),
            port: FactPort::MachineLimits,
            evidence: EvidenceLevel::Attested,
        }],
        provenance: ContractProvenance {
            contract: symbol("platform/contract/capsule-v1"),
            content_digest: format!("sha256:contract-{site}"),
            issuer: symbol("platform/issuer/sim-platform"),
        },
    }
}

fn attestation(card: &PlatformCard, result: &str) -> CapsuleAttestation {
    CapsuleAttestation {
        capsule: card.site.clone(),
        artifact_digest: format!("sha256:artifact-{}", card.site.0),
        card_digest: stable_digest(&serde_json::to_vec(card).unwrap()),
        evidence: ExecutionEvidence {
            execution: symbol(&format!("execution/{}", card.site.0.replace('/', "-"))),
            activation: symbol(&format!("activation/{}", card.site.0.replace('/', "-"))),
            request_digest: "sha256:portable-input".into(),
            result_digest: result.into(),
            ledger_ref: symbol(&format!("ledger/{}", card.site.0.replace('/', "-"))),
        },
    }
}

fn content(id: &str, digest: &str, capabilities: &[&str]) -> BundleContent {
    BundleContent {
        id: symbol(id),
        content_digest: digest.into(),
        capabilities: capabilities.iter().map(|item| symbol(item)).collect(),
    }
}

fn compose<'a>(
    selected: &'a OpenSymbol,
    cards: &'a [PlatformCard],
    attestations: &'a [CapsuleAttestation],
    artifacts: &'a BTreeMap<OpenSymbol, String>,
    allowed: &'a BTreeSet<OpenSymbol>,
    required: &'a [OpenSymbol],
) -> Result<ComposedBundle, BundleRefusal> {
    compose_bundle(BundleComposition {
        capsule: selected,
        application: content(
            "application/portable",
            "sha256:portable-app",
            &["capability/eval"],
        ),
        libraries: vec![content("library/portable-core", "sha256:portable-lib", &[])],
        cards,
        attestations,
        declared_artifacts: artifacts,
        allowed_capabilities: allowed,
        required_services: required,
    })
}

#[test]
fn selected_capsule_and_explicit_content_form_one_descriptor_and_artifact() {
    let model = card("platform/site/model");
    let evidence = attestation(&model, "semantic:42");
    let artifacts = BTreeMap::from([(model.site.clone(), evidence.artifact_digest.clone())]);
    let allowed = BTreeSet::from([symbol("capability/eval")]);
    let bundle = compose(
        &model.site,
        std::slice::from_ref(&model),
        &[evidence],
        &artifacts,
        &allowed,
        &[symbol("platform/service/portable-eval")],
    )
    .unwrap();

    assert_eq!(bundle.bootstrap.capsule, model.site);
    assert_eq!(
        bundle.bootstrap.load_plan.application.content_digest,
        "sha256:portable-app"
    );
    assert_eq!(
        bundle.bootstrap.load_plan.libraries[0].content_digest,
        "sha256:portable-lib"
    );
    assert_eq!(
        bundle.capsule_artifact.content_digest,
        artifacts[&bundle.bootstrap.capsule]
    );
    assert_eq!(
        serde_json::to_value(&bundle)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .filter(|key| *key == "bootstrap")
            .count(),
        1
    );
}

#[test]
fn composition_rejects_every_integrity_and_authority_violation() {
    let model = card("platform/site/model");
    let evidence = attestation(&model, "semantic:42");
    let artifacts = BTreeMap::from([(model.site.clone(), evidence.artifact_digest.clone())]);
    let allowed = BTreeSet::from([symbol("capability/eval")]);
    let required = [symbol("platform/service/portable-eval")];
    let input = |cards: &[PlatformCard], attestations: &[CapsuleAttestation]| {
        compose(
            &model.site,
            cards,
            attestations,
            &artifacts,
            &allowed,
            &required,
        )
    };

    assert!(matches!(
        input(
            &[model.clone(), model.clone()],
            std::slice::from_ref(&evidence)
        ),
        Err(BundleRefusal::DuplicateProvider(_))
    ));
    assert!(matches!(
        input(std::slice::from_ref(&model), &[]),
        Err(BundleRefusal::MissingAttestation(_))
    ));
    assert!(matches!(
        input(
            std::slice::from_ref(&model),
            &[evidence.clone(), evidence.clone()]
        ),
        Err(BundleRefusal::DuplicateAttestation(_))
    ));
    assert!(matches!(
        compose(
            &model.site,
            std::slice::from_ref(&model),
            std::slice::from_ref(&evidence),
            &BTreeMap::new(),
            &allowed,
            &required
        ),
        Err(BundleRefusal::UndeclaredArtifact(_))
    ));
    let mut mismatched = evidence.clone();
    mismatched.artifact_digest = "sha256:wrong".into();
    assert!(matches!(
        input(std::slice::from_ref(&model), &[mismatched]),
        Err(BundleRefusal::EvidenceContentMismatch(_))
    ));
    assert!(matches!(
        compose(
            &model.site,
            std::slice::from_ref(&model),
            std::slice::from_ref(&evidence),
            &artifacts,
            &BTreeSet::new(),
            &required
        ),
        Err(BundleRefusal::CapabilityEscalation { .. })
    ));
    assert!(matches!(
        compose(
            &model.site,
            std::slice::from_ref(&model),
            &[evidence],
            &artifacts,
            &allowed,
            &[symbol("platform/service/missing")]
        ),
        Err(BundleRefusal::MissingRequiredService(_))
    ));
}

#[test]
fn portable_bundle_has_identical_semantics_across_retained_capsule_evidence() {
    let sites = [
        "platform/site/model",
        "platform/site/ubuntu-pc",
        "platform/site/linux-cross-x86",
        "platform/site/linux-cross-arm",
        "platform/site/android-emulator",
        "platform/site/ios-emulator",
        "platform/site/hosted-linux",
        "platform/site/physical-rpi",
    ];
    let cards = sites.map(card);
    let attestations = cards
        .iter()
        .map(|item| attestation(item, "semantic:42"))
        .collect::<Vec<_>>();
    let artifacts = attestations
        .iter()
        .map(|item| (item.capsule.clone(), item.artifact_digest.clone()))
        .collect::<BTreeMap<_, _>>();
    let allowed = BTreeSet::from([symbol("capability/eval")]);
    let required = [symbol("platform/service/portable-eval")];
    let bundles = cards
        .iter()
        .map(|item| {
            compose(
                &item.site,
                &cards,
                &attestations,
                &artifacts,
                &allowed,
                &required,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();

    assert!(bundles.iter().all(
        |bundle| bundle.bootstrap.load_plan.application.content_digest == "sha256:portable-app"
    ));
    assert!(
        bundles
            .iter()
            .all(|bundle| bundle.attestation.evidence.result_digest == "semantic:42")
    );
    let matrix = platform_support_matrix(&bundles);
    assert_eq!(matrix.len(), sites.len());
    assert!(matrix.iter().all(
        |row| row.contract_provenance.starts_with("sha256:contract-")
            && row.execution_evidence == "semantic:42"
    ));
}

#[test]
fn removed_selected_capsule_refuses_without_implicit_provider_selection() {
    let selected = symbol("platform/site/physical-rpi");
    let model = card("platform/site/model");
    let evidence = attestation(&model, "semantic:42");
    let artifacts = BTreeMap::from([(model.site.clone(), evidence.artifact_digest.clone())]);
    let refusal = compose(
        &selected,
        &[model],
        &[evidence],
        &artifacts,
        &BTreeSet::from([symbol("capability/eval")]),
        &[symbol("platform/service/portable-eval")],
    )
    .unwrap_err();
    assert_eq!(refusal, BundleRefusal::MissingCapsule(selected));
}
