#![forbid(unsafe_code)]
//! Least-authority WASI capsule.
//!
// conformance: WASI imports and authority remain closed and explicitly bound.
//! WASI names are declarations at the artifact boundary. Each admitted import
//! is paired with the existing SIM service it realizes; this crate does not
//! introduce a filesystem, socket, clock, entropy, or process abstraction.

mod wasi;

pub use wasi::*;

#[cfg(test)]
mod tests {
    use super::*;
    use sim_wasm_abi::{AbiValue, Handle};
    use std::collections::{BTreeMap, BTreeSet};

    fn profile() -> WasiProfile {
        WasiProfile {
            preopens: BTreeMap::from([(
                "input".into(),
                PreopenRef::Dir {
                    table: "artifacts".into(),
                    key: "input-v1".into(),
                },
            )]),
            capabilities: ALLOWED_IMPORTS
                .iter()
                .map(|item| item.service.capability().to_owned())
                .collect(),
            wall_clock_ticks: 7,
            monotonic_ticks: 11,
            entropy_seed: [19; 32],
            socket_endpoints: BTreeSet::from(["127.0.0.1:9443".into()]),
        }
    }

    #[test]
    fn artifact_inspection_admits_only_declared_imports() {
        let admitted = wat::parse_str(
            r#"(module
          (import "wasi:clocks/monotonic-clock@0.2.0" "now" (func))
          (import "wasi:random/random@0.2.0" "get-random-bytes" (func)))"#,
        )
        .unwrap();
        assert_eq!(inspect_artifact(&admitted).unwrap().len(), 2);

        let ambient = wat::parse_str(
            r#"(module (import "wasi:cli/environment@0.2.0" "get-environment" (func)))"#,
        )
        .unwrap();
        assert!(matches!(
            inspect_artifact(&ambient),
            Err(WasiRefusal::UndeclaredImport { .. })
        ));
    }

    #[test]
    fn component_manifest_declares_every_admitted_import_mapping() {
        for declaration in ALLOWED_IMPORTS {
            let row = format!(
                "{}#{} => {}",
                declaration.module,
                declaration.name,
                declaration.service.identity()
            );
            assert!(
                CAPSULE_MANIFEST.contains(&row),
                "missing manifest row: {row}"
            );
        }
        for exclusion in [
            "process-create",
            "native-dynamic-load",
            "ui",
            "notification",
            "device",
        ] {
            assert!(CAPSULE_MANIFEST.contains(exclusion));
        }
    }

    #[test]
    fn preopens_and_sockets_are_exact_bundle_authority() {
        let capsule = WasiCapsule::new(profile());
        assert!(matches!(
            capsule.preopen("input"),
            Ok(PreopenRef::Dir { .. })
        ));
        assert!(matches!(
            capsule.preopen("ambient"),
            Err(WasiRefusal::UnknownPreopen { .. })
        ));
        assert_eq!(capsule.connect("127.0.0.1:9443"), Ok(()));
        assert!(matches!(
            capsule.connect("example.com:443"),
            Err(WasiRefusal::UnknownSocket { .. })
        ));
    }

    #[test]
    fn card_excludes_ambient_and_unsupported_services() {
        let card = platform_card(&profile());
        let offered: BTreeSet<_> = card
            .services
            .iter()
            .map(|offer| offer.service.0.as_str())
            .collect();
        for absent in [
            "process/create",
            "loader/native",
            "ui/window",
            "platform/notification",
            "device/discover",
        ] {
            assert!(!offered.contains(absent));
        }
        assert!(matches!(
            WasiCapsule::new(profile()).unsupported("process/create"),
            WasiRefusal::UnsupportedService { .. }
        ));
    }

    #[test]
    fn model_is_deterministic_and_profile_closed() {
        let first = WasiCapsule::new(profile());
        let second = WasiCapsule::new(profile());
        assert_eq!(
            first.model_digest(b"same request"),
            second.model_digest(b"same request")
        );
        assert_ne!(
            first.model_digest(b"same request"),
            second.model_digest(b"other request")
        );
        assert!(serde_json::from_str::<WasiProfile>(r#"{"preopens":{},"capabilities":[],"wall_clock_ticks":0,"monotonic_ticks":0,"entropy_seed":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"socket_endpoints":[],"ambient_home":"/tmp"}"#).is_err());
    }

    struct Hosted;
    impl RealWasiRuntime for Hosted {
        fn run_frame_corpus(&self, values: &[AbiValue]) -> Result<Vec<u8>, String> {
            let mut receipt = Vec::new();
            for value in values {
                let frame =
                    sim_wasm_abi::encode_value_frame(value).map_err(|error| error.to_string())?;
                let decoded =
                    sim_wasm_abi::decode_value_frame(&frame).map_err(|error| error.to_string())?;
                if decoded != *value {
                    return Err("hosted runtime frame mismatch".into());
                }
                receipt.extend_from_slice(frame.bytes());
            }
            Ok(receipt)
        }
    }

    #[test]
    fn abi_corpus_retains_modeled_and_hosted_grades() {
        let values = [
            AbiValue::Handle(Handle(42)),
            AbiValue::Error("typed refusal".into()),
        ];
        let modeled = run_in_memory_frame_corpus(&values).unwrap();
        let hosted = run_real_frame_corpus(&Hosted, &values).unwrap();
        assert_eq!(modeled.grade, EvidenceGrade::InMemory);
        assert_eq!(hosted.grade, EvidenceGrade::RealRuntime);
        assert_eq!(modeled.cases, hosted.cases);
        assert_eq!(modeled.digest, hosted.digest);
    }
}
