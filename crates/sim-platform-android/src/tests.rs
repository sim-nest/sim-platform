#![allow(unsafe_code)]

use std::ffi::CString;

use sim_kernel::{NativeAbiBorrowedBytes, NativeAbiCallResponse, NativeLibAbiV1};

use super::*;

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
        }
    );
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
