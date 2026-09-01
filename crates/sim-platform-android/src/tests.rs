#![allow(unsafe_code)]

// conformance: Android AOT bindings preserve lifecycle, authority, and refusal semantics.

use std::ffi::CString;

use sim_kernel::{NativeAbiBorrowedBytes, NativeAbiCallResponse, NativeLibAbiV1};

use super::*;
use crate::{
    AudioRouteClass, AudioSessionSpec, AudioStopReason, PcmContract, RouteEvidence,
    RouteObservation, RoutingContract, StaticAbiCapsule, sim_native_abi_v1,
};

fn send(capsule: &mut Capsule, function: &str, input: &Input) -> Output {
    let frame = encode_input_frame(input).unwrap();
    decode_output_frame(&capsule.call_frame(function, &frame).unwrap()).unwrap()
}

fn take_raw_response(
    abi: &NativeLibAbiV1,
    response: &NativeAbiCallResponse,
) -> Result<Vec<u8>, String> {
    if !response.error.is_null() {
        unsafe {
            (abi.destroy_error)(response.error);
        }
        return Err("raw ABI call failed".into());
    }
    assert!(!response.bytes.ptr.is_null());
    assert!(response.bytes.len <= response.bytes.cap);
    let bytes = unsafe {
        std::slice::from_raw_parts(response.bytes.ptr.cast_const(), response.bytes.len).to_vec()
    };
    unsafe {
        (abi.destroy_bytes)(response.bytes);
    }
    Ok(bytes)
}

fn call_exported_native(function: &str, frame: &[u8]) -> Vec<u8> {
    let abi = unsafe { &*sim_native_abi_v1() };
    assert_eq!(abi.abi_major, sim_kernel::NATIVE_LIB_ABI_V1_MAJOR);
    assert!(abi.struct_size >= std::mem::size_of::<NativeLibAbiV1>());
    let instance = unsafe { (abi.instantiate)() };
    assert!(!instance.is_null());
    let function = CString::new(function).unwrap();
    let response = unsafe {
        (abi.call)(
            instance,
            function.as_ptr(),
            NativeAbiBorrowedBytes::borrow(frame),
        )
    };
    let bytes = take_raw_response(abi, &response).unwrap();
    unsafe {
        (abi.destroy_instance)(instance);
    }
    bytes
}

#[test]
fn native_static_and_modeled_paths_share_the_exact_sim_frame() {
    let input = Input::Activation {
        action: "android.intent.action.VIEW".into(),
        content: Some(ContentRef::Table {
            mount: "shared-documents".into(),
            key: vec!["inbox".into(), "note.siml".into()],
        }),
    };
    let frame = encode_input_frame(&input).unwrap();

    let modeled = Capsule::default()
        .call_frame(ACTIVATION_FUNCTION, &frame)
        .unwrap();
    let mut static_capsule = StaticAbiCapsule::new().unwrap();
    let static_path = static_capsule.call(ACTIVATION_FUNCTION, &frame).unwrap();
    let native = call_exported_native(ACTIVATION_FUNCTION, &frame);

    assert_eq!(native, static_path);
    assert_eq!(static_path, modeled);
    assert_eq!(
        decode_output_frame(&native).unwrap(),
        Output {
            lifecycle: Lifecycle::Created,
            accepted: true,
            resources: 1,
            snapshot_content_id: None,
            resumed_turn_content_id: None,
            audio_route: None,
            speech: None,
        }
    );
}

fn audio_spec(api_level: u16, private_output: bool) -> AudioSessionSpec {
    AudioSessionSpec {
        turn_content_id: content_id('d'),
        api_level,
        admitted: true,
        private_output,
        pcm: PcmContract {
            sample_rate_hz: 48_000,
            channels: 2,
            frames_per_chunk: 480,
            queue_capacity_chunks: 8,
        },
        armed_at_ms: 1_000,
        expires_at_ms: 10_000,
    }
}

fn audio(capsule: &mut Capsule, input: AudioInput) -> Output {
    send(capsule, AUDIO_FUNCTION, &Input::Audio { input })
}

#[test]
fn supported_api_levels_select_the_official_focus_and_route_contract() {
    for api in 28..=35 {
        let mut capsule = Capsule::default();
        let receipt = audio(
            &mut capsule,
            AudioInput::Arm {
                spec: audio_spec(api, false),
            },
        )
        .audio_route
        .unwrap();
        let expected = if api <= 30 {
            RoutingContract::Api28To30CommunicationMode
        } else {
            RoutingContract::Api31To35CommunicationDevice
        };
        assert_eq!(receipt.routing_contract, expected);
        assert!(receipt.focus_held && receipt.communication_route_held);
    }
    let mut capsule = Capsule::default();
    assert!(
        capsule
            .dispatch(
                AUDIO_FUNCTION,
                Input::Audio {
                    input: AudioInput::Arm {
                        spec: audio_spec(27, false)
                    }
                }
            )
            .is_err()
    );
}

#[test]
fn route_receipts_are_private_independent_fresh_and_keep_turn_identity() {
    let mut capsule = Capsule::default();
    audio(
        &mut capsule,
        AudioInput::Arm {
            spec: audio_spec(35, true),
        },
    );
    let output_only = audio(
        &mut capsule,
        AudioInput::Route {
            observation: RouteObservation {
                capture: vec![AudioRouteClass::Handset],
                render: vec![AudioRouteClass::ClassicBluetooth],
                generation: 1,
                observed_at_ms: 1_100,
                evidence: RouteEvidence::DeviceCallback,
            },
        },
    )
    .audio_route
    .unwrap();
    assert_eq!(output_only.turn_content_id, content_id('d'));
    assert!(!output_only.duplex, "an output route never implies input");
    assert!(output_only.private_output_paused);
    let changed = audio(
        &mut capsule,
        AudioInput::Route {
            observation: RouteObservation {
                capture: vec![AudioRouteClass::LeAudio],
                render: vec![AudioRouteClass::LeAudio],
                generation: 2,
                observed_at_ms: 1_200,
                evidence: RouteEvidence::CommunicationCallback,
            },
        },
    )
    .audio_route
    .unwrap();
    assert_eq!(changed.turn_content_id, output_only.turn_content_id);
    assert_eq!(changed.generation, 2);
    assert!(changed.duplex && !changed.private_output_paused && changed.fresh);
    let json = serde_json::to_string(&changed).unwrap();
    for forbidden in ["device-id", "address", "name", "pair"] {
        assert!(!json.contains(forbidden));
    }
}

#[test]
fn every_terminal_path_releases_focus_and_communication_routing() {
    for reason in [
        AudioStopReason::Stop,
        AudioStopReason::Cancellation,
        AudioStopReason::BackgroundExpiry,
        AudioStopReason::PermissionLoss,
        AudioStopReason::RouteLoss,
        AudioStopReason::FocusConflict,
        AudioStopReason::ProcessDeath,
    ] {
        let mut capsule = Capsule::default();
        audio(
            &mut capsule,
            AudioInput::Arm {
                spec: audio_spec(35, false),
            },
        );
        let stopped = audio(&mut capsule, AudioInput::Stop { reason });
        assert!(!stopped.accepted);
        assert!(stopped.audio_route.is_none());
    }
    let mut capsule = Capsule::default();
    audio(
        &mut capsule,
        AudioInput::Arm {
            spec: audio_spec(35, false),
        },
    );
    assert!(
        audio(&mut capsule, AudioInput::Tick { now_ms: 10_000 })
            .audio_route
            .is_none()
    );
    for event in [
        AndroidEvent::BackgroundRestriction,
        AndroidEvent::Suspend,
        AndroidEvent::ProcessDeath,
        AndroidEvent::MemoryPressure,
    ] {
        audio(
            &mut capsule,
            AudioInput::Arm {
                spec: audio_spec(35, false),
            },
        );
        send(&mut capsule, CONTINUITY_FUNCTION, &Input::Event { event });
        assert!(
            audio(&mut capsule, AudioInput::Tick { now_ms: 1_100 })
                .audio_route
                .is_none()
        );
    }
}

#[test]
fn handset_wired_classic_le_output_only_conflict_unplug_denial_and_death_are_closed() {
    for class in [
        AudioRouteClass::Handset,
        AudioRouteClass::Wired,
        AudioRouteClass::ClassicBluetooth,
        AudioRouteClass::LeAudio,
    ] {
        let mut capsule = Capsule::default();
        audio(
            &mut capsule,
            AudioInput::Arm {
                spec: audio_spec(31, false),
            },
        );
        let receipt = audio(
            &mut capsule,
            AudioInput::Route {
                observation: RouteObservation {
                    capture: vec![class],
                    render: vec![class],
                    generation: 1,
                    observed_at_ms: 1_050,
                    evidence: RouteEvidence::DeviceCallback,
                },
            },
        )
        .audio_route
        .unwrap();
        assert!(receipt.duplex);
        assert_eq!(receipt.pcm.queue_capacity_chunks, 8);
    }
    let mut denied = audio_spec(35, false);
    denied.admitted = false;
    assert!(
        Capsule::default()
            .dispatch(
                AUDIO_FUNCTION,
                Input::Audio {
                    input: AudioInput::Arm { spec: denied }
                }
            )
            .is_err()
    );
    let mut capsule = Capsule::default();
    audio(
        &mut capsule,
        AudioInput::Arm {
            spec: audio_spec(35, true),
        },
    );
    let uncertain = audio(
        &mut capsule,
        AudioInput::Route {
            observation: RouteObservation {
                capture: vec![],
                render: vec![],
                generation: 1,
                observed_at_ms: 1_100,
                evidence: RouteEvidence::DeviceCallback,
            },
        },
    )
    .audio_route
    .unwrap();
    assert!(
        uncertain.private_output_paused,
        "unplug uncertainty pauses private output"
    );
    assert!(!uncertain.focus_held && !uncertain.communication_route_held);
    assert!(
        audio(&mut capsule, AudioInput::Tick { now_ms: 1_200 })
            .audio_route
            .is_none(),
        "route loss stops the session"
    );
}

fn content_id(byte: char) -> String {
    std::iter::repeat_n(byte, 64).collect()
}

fn bind_plan() -> BindPlan {
    let providers = REQUIRED_SERVICES
        .into_iter()
        .map(|service| (format!("provider/{service}"), format!("provider/{service}")))
        .collect();
    let services = REQUIRED_SERVICES
        .into_iter()
        .map(|service| (service.to_owned(), format!("provider/{service}")))
        .collect();
    BindPlan {
        snapshot: ProviderSnapshot {
            content_id: content_id('a'),
            providers,
        },
        app_private_mount: "app-private".into(),
        services,
    }
}

#[test]
fn provider_snapshot_binds_all_required_services_or_nothing() {
    let mut capsule = Capsule::default();
    let mut incomplete = bind_plan();
    incomplete.services.remove("permissions");
    assert!(
        capsule
            .dispatch(CONTINUITY_FUNCTION, Input::Bind { plan: incomplete })
            .is_err()
    );
    let bound = send(
        &mut capsule,
        CONTINUITY_FUNCTION,
        &Input::Bind { plan: bind_plan() },
    );
    assert!(bound.accepted);
    assert_eq!(bound.snapshot_content_id, Some(content_id('a')));
}

#[test]
fn killed_submitted_turn_restores_once_from_content_and_journal_ids() {
    let mut capsule = Capsule::default();
    send(
        &mut capsule,
        CONTINUITY_FUNCTION,
        &Input::Bind { plan: bind_plan() },
    );
    let turn = content_id('b');
    let first = send(
        &mut capsule,
        CONTINUITY_FUNCTION,
        &Input::SubmitTurn {
            content_id: turn.clone(),
        },
    );
    assert_eq!(first.resumed_turn_content_id, None);
    send(
        &mut capsule,
        CONTINUITY_FUNCTION,
        &Input::Event {
            event: AndroidEvent::ProcessDeath,
        },
    );
    send(
        &mut capsule,
        CONTINUITY_FUNCTION,
        &Input::Event {
            event: AndroidEvent::Restart,
        },
    );
    send(
        &mut capsule,
        CONTINUITY_FUNCTION,
        &Input::Bind { plan: bind_plan() },
    );
    let restored = send(
        &mut capsule,
        CONTINUITY_FUNCTION,
        &Input::Restore {
            plan: RestorePlan {
                snapshot_content_id: content_id('a'),
                journal_content_id: content_id('c'),
                pending_turn_content_id: Some(turn.clone()),
                permission_observations: BTreeMap::from([(Permission::SharedDocument, false)]),
            },
        },
    );
    assert_eq!(restored.resumed_turn_content_id, Some(turn.clone()));
    assert_eq!(
        capsule.permissions.get(&Permission::SharedDocument),
        Some(&false)
    );
    let duplicate = send(
        &mut capsule,
        CONTINUITY_FUNCTION,
        &Input::Restore {
            plan: RestorePlan {
                snapshot_content_id: content_id('a'),
                journal_content_id: content_id('c'),
                pending_turn_content_id: Some(turn),
                permission_observations: BTreeMap::new(),
            },
        },
    );
    assert_eq!(
        duplicate.resumed_turn_content_id, None,
        "the same journal turn resumes once"
    );
}

#[test]
fn every_android_disruption_is_an_explicit_rust_input() {
    let mut capsule = Capsule::default();
    for event in [
        AndroidEvent::Rotation,
        AndroidEvent::ActivityRecreation,
        AndroidEvent::BackgroundRestriction,
        AndroidEvent::Suspend,
        AndroidEvent::ProcessDeath,
        AndroidEvent::Restart,
        AndroidEvent::MemoryPressure,
    ] {
        assert!(send(&mut capsule, CONTINUITY_FUNCTION, &Input::Event { event }).accepted);
    }
}

#[test]
fn exported_manifest_requires_the_desktop_platform_library() {
    let mut capsule = StaticAbiCapsule::new().unwrap();
    let bytes = capsule.manifest().unwrap();
    let (_, expr) = sim_codec_binary::decode_frame(sim_kernel::CodecId(0), &bytes).unwrap();
    let rendered = format!("{expr:?}");
    assert!(rendered.contains("android-capsule"));
    assert!(rendered.contains("sim"));
    assert!(rendered.contains("platform"));
    assert!(rendered.contains("lifecycle"));
    assert!(rendered.contains("activation"));
}

#[test]
fn recreation_denial_suspension_activation_and_cleanup_are_bounded() {
    let mut capsule = Capsule::default();
    send(
        &mut capsule,
        LIFECYCLE_FUNCTION,
        &Input::Lifecycle {
            state: Lifecycle::Created,
        },
    );
    assert!(
        !send(
            &mut capsule,
            ACTIVATION_FUNCTION,
            &Input::Permission {
                permission: Permission::SharedDocument,
                granted: false,
            },
        )
        .accepted
    );
    assert!(
        send(
            &mut capsule,
            ACTIVATION_FUNCTION,
            &Input::Activation {
                action: "open".into(),
                content: None,
            },
        )
        .accepted
    );
    let suspended = send(
        &mut capsule,
        LIFECYCLE_FUNCTION,
        &Input::Lifecycle {
            state: Lifecycle::Suspended,
        },
    );
    assert_eq!(suspended.resources, 0);
    assert!(
        !send(
            &mut capsule,
            ACTIVATION_FUNCTION,
            &Input::Activation {
                action: "resume".into(),
                content: None,
            },
        )
        .accepted
    );
    send(
        &mut capsule,
        LIFECYCLE_FUNCTION,
        &Input::Lifecycle {
            state: Lifecycle::Created,
        },
    );
    assert!(
        send(
            &mut capsule,
            ACTIVATION_FUNCTION,
            &Input::Activation {
                action: "resume".into(),
                content: None,
            },
        )
        .accepted
    );
    let stopped = send(
        &mut capsule,
        LIFECYCLE_FUNCTION,
        &Input::Lifecycle {
            state: Lifecycle::Stopped,
        },
    );
    assert_eq!(stopped.resources, 0);
}

#[test]
fn paths_unbounded_content_and_wrong_function_types_fail_closed() {
    let invalid_path = Input::Activation {
        action: "open".into(),
        content: Some(ContentRef::Dir {
            mount: "shared".into(),
            relative: vec!["..".into()],
        }),
    };
    assert!(
        Capsule::default()
            .dispatch(ACTIVATION_FUNCTION, invalid_path)
            .is_err()
    );
    let unbounded = Input::Activation {
        action: "open".into(),
        content: Some(ContentRef::Bytes {
            media_type: "application/octet-stream".into(),
            bytes: vec![0; 8 * 1024 * 1024 + 1],
        }),
    };
    assert!(
        Capsule::default()
            .dispatch(ACTIVATION_FUNCTION, unbounded)
            .is_err()
    );
    assert!(
        Capsule::default()
            .dispatch(
                LIFECYCLE_FUNCTION,
                Input::Activation {
                    action: "wrong-function".into(),
                    content: None,
                },
            )
            .is_err()
    );
}

#[test]
fn committed_cross_build_rows_never_claim_hosted_or_modeled_execution() {
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../contract/android-build-provenance.json")).unwrap();
    assert_eq!(
        contract["official_remote_recheck"],
        "terminal-closeout-required"
    );
    assert!(contract["official_response_sha256"].is_null());

    for row in [
        include_str!("../attestations/aarch64-linux-android.json"),
        include_str!("../attestations/armv7-linux-androideabi.json"),
        include_str!("../attestations/x86_64-linux-android.json"),
        include_str!("../attestations/i686-linux-android.json"),
    ] {
        let row: serde_json::Value = serde_json::from_str(row).unwrap();
        assert_eq!(row["evidence"], "cross-built");
        assert_eq!(row["registered_host"], serde_json::Value::Null);
        assert_eq!(row["hosted_ci"], false);
        assert_eq!(row["hosted_receipt"], serde_json::Value::Null);
        for digest in [
            "target_spec_sha256",
            "artifact_sha256",
            "undefined_imports_sha256",
        ] {
            assert_eq!(row[digest].as_str().unwrap().len(), 64);
        }
    }
}
