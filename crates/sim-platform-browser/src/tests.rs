// conformance: browser capability discovery and lifecycle fail closed.

use super::*;

fn full_apis() -> BrowserApis {
    BrowserApis {
        storage: true,
        fetch: true,
        websocket: true,
        clipboard: true,
        notification: true,
        permissions: true,
        wake_lock: true,
    }
}

fn call(capsule: &mut Capsule, function: &str, input: &Input) -> Result<Output, String> {
    let bytes = encode_input_frame(input)?;
    decode_output_frame(&capsule.call_frame(function, &bytes)?)
}

#[test]
fn card_is_capability_honest_and_has_no_surface_or_native_authority() {
    let card = BrowserCard::from_apis(&BrowserApis {
        fetch: true,
        ..BrowserApis::default()
    });
    assert_eq!(
        card.services,
        [
            "platform/lifecycle",
            "platform/activation",
            "transport/fetch"
        ]
    );
    assert_eq!(card.presentation_owner, "sim-web");
    let rendered = serde_json::to_string(&card).unwrap();
    for forbidden in ["dom", "canvas", "webgpu", "render-tree"] {
        assert!(!rendered.to_lowercase().contains(forbidden));
    }
    for omitted in [
        "platform/process",
        "loader/native-v1",
        "storage/ambient-filesystem",
    ] {
        assert!(card.omitted_services.iter().any(|item| item == omitted));
    }
}

#[test]
fn window_worker_lifecycle_restarts_release_all_authority() {
    for _context in ["window", "worker"] {
        let mut capsule = Capsule::new(&full_apis());
        call(
            &mut capsule,
            "platform/lifecycle",
            &Input::Lifecycle {
                state: Lifecycle::Active,
            },
        )
        .unwrap();
        let opened = call(
            &mut capsule,
            HOST_FUNCTION,
            &Input::HostCall {
                operation: HostOperation::StorageOpen {
                    namespace: "offline".into(),
                },
            },
        )
        .unwrap();
        let Output::HostCall { call_id, .. } = opened else {
            panic!()
        };
        call(
            &mut capsule,
            COMPLETE_FUNCTION,
            &Input::HostComplete {
                call_id,
                ok: true,
                payload: vec![],
                handle: Some(7),
            },
        )
        .unwrap();
        call(
            &mut capsule,
            "platform/lifecycle",
            &Input::Lifecycle {
                state: Lifecycle::Stopped,
            },
        )
        .unwrap();
        assert!(
            call(
                &mut capsule,
                HOST_FUNCTION,
                &Input::HostCall {
                    operation: HostOperation::StorageRead {
                        handle: 7,
                        key: "lost".into()
                    }
                }
            )
            .is_err()
        );
        call(
            &mut capsule,
            "platform/lifecycle",
            &Input::Lifecycle {
                state: Lifecycle::Active,
            },
        )
        .unwrap();
        assert!(
            call(
                &mut capsule,
                HOST_FUNCTION,
                &Input::HostCall {
                    operation: HostOperation::StorageRead {
                        handle: 7,
                        key: "lost".into()
                    }
                }
            )
            .is_err()
        );
    }
}

#[test]
fn denied_missing_and_hostile_operations_fail_closed() {
    let mut capsule = Capsule::new(&BrowserApis {
        permissions: true,
        ..BrowserApis::default()
    });
    assert!(
        call(
            &mut capsule,
            HOST_FUNCTION,
            &Input::HostCall {
                operation: HostOperation::Fetch {
                    url: "https://example.invalid".into(),
                    method: "GET".into(),
                    body: vec![]
                }
            }
        )
        .is_err()
    );
    let request = call(
        &mut capsule,
        HOST_FUNCTION,
        &Input::HostCall {
            operation: HostOperation::Permission {
                name: "camera".into(),
                request: true,
            },
        },
    )
    .unwrap();
    let Output::HostCall { call_id, .. } = request else {
        panic!()
    };
    assert_eq!(
        call(
            &mut capsule,
            COMPLETE_FUNCTION,
            &Input::HostComplete {
                call_id,
                ok: false,
                payload: b"denied".to_vec(),
                handle: None
            }
        )
        .unwrap(),
        Output::Completed {
            call_id,
            ok: false,
            payload: b"denied".to_vec(),
            handle: None
        }
    );
    assert!(
        call(
            &mut capsule,
            HOST_FUNCTION,
            &Input::HostCall {
                operation: HostOperation::ClipboardWrite {
                    text: "x".repeat(MAX_HOST_PAYLOAD_BYTES + 1)
                }
            }
        )
        .is_err()
    );
    assert!(
        call(
            &mut capsule,
            HOST_FUNCTION,
            &Input::HostCall {
                operation: HostOperation::Fetch {
                    url: "file:///etc/passwd".into(),
                    method: "GET".into(),
                    body: vec![]
                }
            }
        )
        .is_err()
    );
    assert!(
        Capsule::default()
            .call_frame(CARD_FUNCTION, &vec![0; MAX_FRAME_BYTES + 1])
            .is_err()
    );
}

#[test]
fn every_operation_uses_the_canonical_frame_and_named_call_surface() {
    assert_eq!(NAMED_CALL_EXPORT, "sim_browser_named_call");
    let mut capsule = Capsule::new(&full_apis());
    let output = call(
        &mut capsule,
        HOST_FUNCTION,
        &Input::HostCall {
            operation: HostOperation::WakeLock { acquire: true },
        },
    )
    .unwrap();
    assert!(matches!(output, Output::HostCall { service, .. } if service == "platform/wake-lock"));
    assert!(include_str!("../contract/wasm-imports.json").contains("sim_browser_named_call"));
}
