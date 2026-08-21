use sim_codec::{Input, Output, decode_with_codec, encode_with_codec};
use sim_kernel::{Cx, DefaultFactory, EagerPolicy, EncodeOptions, Expr, ReadPolicy, Symbol};
use sim_platform_core::*;
use std::sync::Arc;

fn symbol(value: &str) -> OpenSymbol {
    OpenSymbol::new(value).unwrap()
}
fn provenance() -> ContractProvenance {
    ContractProvenance {
        contract: symbol("platform/contract/v1"),
        content_digest: "sha256:test".into(),
        issuer: symbol("platform/issuer/model"),
    }
}

#[test]
fn resolver_is_atomic_and_uses_ordered_substitutes_and_evidence() {
    let card = PlatformCard {
        schema: symbol("platform/card/v1"),
        site: symbol("platform/site/model"),
        provenance: provenance(),
        services: vec![
            ServiceOffer {
                service: symbol("clock/wall-model"),
                port: FactPort::WallClock,
                evidence: EvidenceLevel::Modeled,
            },
            ServiceOffer {
                service: symbol("entropy/model"),
                port: FactPort::Entropy,
                evidence: EvidenceLevel::Declared,
            },
        ],
    };
    let request = PlatformRequest {
        request: symbol("request/one"),
        requirements: vec![
            Requirement {
                service: symbol("clock/wall"),
                substitutes: vec![symbol("clock/wall-model")],
                optional: false,
                minimum_evidence: EvidenceLevel::Modeled,
            },
            Requirement {
                service: symbol("locale/preferred"),
                substitutes: vec![],
                optional: true,
                minimum_evidence: EvidenceLevel::Declared,
            },
        ],
    };
    let (bound, receipt) = platform_require(&card, &request).unwrap();
    assert_eq!(bound.bindings.len(), 1);
    assert_eq!(bound.bindings[0].bound, symbol("clock/wall-model"));
    assert_eq!(receipt.bindings, bound.bindings);

    let denied = PlatformRequest {
        request: symbol("request/two"),
        requirements: vec![Requirement {
            service: symbol("entropy/model"),
            substitutes: vec![],
            optional: false,
            minimum_evidence: EvidenceLevel::Attested,
        }],
    };
    let refusal = platform_require(&card, &denied).unwrap_err();
    assert_eq!(refusal.kind, RefusalKind::Unsupported);
}

#[test]
fn roadmap_requirement_is_host_independent_and_atomic() {
    let card = PlatformCard {
        schema: symbol("platform/card/v1"),
        site: symbol("platform/site/model"),
        provenance: provenance(),
        services: vec![
            ServiceOffer {
                service: symbol("platform/service/monotonic"),
                port: FactPort::MonotonicClock,
                evidence: EvidenceLevel::Modeled,
            },
            ServiceOffer {
                service: symbol("storage/service/config-dir"),
                port: FactPort::MachineLimits,
                evidence: EvidenceLevel::Modeled,
            },
            ServiceOffer {
                service: symbol("platform/service/none"),
                port: FactPort::LifecyclePressure,
                evidence: EvidenceLevel::Modeled,
            },
        ],
    };
    let request = PlatformRequest {
        request: symbol("request/small-api-roadmap"),
        requirements: vec![
            Requirement {
                service: symbol("platform/service/monotonic"),
                substitutes: vec![],
                optional: false,
                minimum_evidence: EvidenceLevel::Declared,
            },
            Requirement {
                service: symbol("storage/service/config-dir"),
                substitutes: vec![],
                optional: false,
                minimum_evidence: EvidenceLevel::Declared,
            },
            Requirement {
                service: symbol("device/service/gpio"),
                substitutes: vec![symbol("platform/service/none")],
                optional: true,
                minimum_evidence: EvidenceLevel::Declared,
            },
        ],
    };
    let (bound, _) = platform_require(&card, &request).unwrap();
    assert_eq!(bound.bindings[2].bound, symbol("platform/service/none"));

    let mut refused = request;
    refused.requirements[1].service = symbol("storage/service/missing-dir");
    assert_eq!(
        platform_require(&card, &refused).unwrap_err().kind,
        RefusalKind::Unsupported
    );
}

#[test]
fn records_round_trip_through_installed_general_expression_codecs() {
    let record = PlatformCard {
        schema: symbol("platform/card/v1"),
        site: symbol("platform/site/model"),
        provenance: provenance(),
        services: vec![ServiceOffer {
            service: symbol("clock/wall"),
            port: FactPort::WallClock,
            evidence: EvidenceLevel::Modeled,
        }],
    };
    // Records project as ordinary expression data; this descriptor exercises every
    // field category (open symbols, ordered lists, enums and provenance).
    let request = PlatformRequest {
        request: symbol("request/round-trip"),
        requirements: vec![Requirement {
            service: symbol("clock/wall"),
            substitutes: vec![],
            optional: false,
            minimum_evidence: EvidenceLevel::Modeled,
        }],
    };
    let (services, receipt) = platform_require(&record, &request).unwrap();
    let lifecycle = Lifecycle::Ready;
    let activation = Activation {
        id: symbol("activation/round-trip"),
        site: record.site.clone(),
        lifecycle: lifecycle.clone(),
        services,
    };
    let evidence = ExecutionEvidence {
        execution: symbol("execution/round-trip"),
        activation: activation.id.clone(),
        request_digest: "sha256:request".into(),
        result_digest: "sha256:result".into(),
        ledger_ref: symbol("ledger/round-trip"),
    };
    let expr = Expr::List(
        vec![
            ("card", serde_json::to_string(&record).unwrap()),
            ("request", serde_json::to_string(&request).unwrap()),
            ("receipt", serde_json::to_string(&receipt).unwrap()),
            ("lifecycle", serde_json::to_string(&lifecycle).unwrap()),
            ("activation", serde_json::to_string(&activation).unwrap()),
            ("attestation", serde_json::to_string(&evidence).unwrap()),
        ]
        .into_iter()
        .map(|(kind, json)| {
            Expr::List(vec![
                Expr::Symbol(Symbol::qualified("platform", kind)),
                Expr::String(json),
            ])
        })
        .collect(),
    );
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    let lisp = sim_codec_lisp::LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lisp).unwrap();
    let json = sim_codec_json::JsonCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&json).unwrap();
    let binary = sim_codec_binary::BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).unwrap();
    for codec in [
        Symbol::qualified("codec", "lisp"),
        Symbol::qualified("codec", "json"),
        Symbol::qualified("codec", "binary"),
    ] {
        let encoded = encode_with_codec(&mut cx, &codec, &expr, EncodeOptions::default()).unwrap();
        let input = match encoded {
            Output::Text(text) => Input::Text(text),
            Output::Bytes(bytes) => Input::Bytes(bytes),
        };
        let decoded = decode_with_codec(&mut cx, &codec, input, ReadPolicy::default()).unwrap();
        assert!(decoded.canonical_eq(&expr), "codec {codec}");
    }
}
