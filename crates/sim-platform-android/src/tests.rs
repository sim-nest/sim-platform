use super::*;

fn send(capsule: &mut Capsule, function: &str, input: &Input) -> Output {
    let bytes = capsule
        .call(function, &serde_json::to_vec(input).unwrap())
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn native_static_and_modeled_paths_share_the_exact_frames() {
    let frame = Input::Activation {
        action: "android.intent.action.VIEW".into(),
        content: Some(ContentRef::Table {
            mount: "shared-documents".into(),
            key: vec!["inbox".into(), "note.siml".into()],
        }),
    };
    let bytes = serde_json::to_vec(&frame).unwrap();
    let native = Capsule::default()
        .call("platform/activation", &bytes)
        .unwrap();
    let static_path = Capsule::default()
        .call("platform/activation", &bytes)
        .unwrap();
    let modeled = Capsule::default()
        .call("platform/activation", &bytes)
        .unwrap();
    assert_eq!(native, static_path);
    assert_eq!(static_path, modeled);
}

#[test]
fn recreation_denial_suspension_activation_and_cleanup_are_bounded() {
    let mut capsule = Capsule::default();
    send(
        &mut capsule,
        "platform/lifecycle",
        &Input::Lifecycle {
            state: Lifecycle::Created,
        },
    );
    assert!(
        !send(
            &mut capsule,
            "platform/activation",
            &Input::Permission {
                permission: Permission::SharedDocument,
                granted: false
            }
        )
        .accepted
    );
    send(
        &mut capsule,
        "platform/lifecycle",
        &Input::Lifecycle {
            state: Lifecycle::Suspended,
        },
    );
    assert!(
        !send(
            &mut capsule,
            "platform/activation",
            &Input::Activation {
                action: "resume".into(),
                content: None
            }
        )
        .accepted
    );
    send(
        &mut capsule,
        "platform/lifecycle",
        &Input::Lifecycle {
            state: Lifecycle::Created,
        },
    );
    assert!(
        send(
            &mut capsule,
            "platform/activation",
            &Input::Activation {
                action: "resume".into(),
                content: None
            }
        )
        .accepted
    );
    send(
        &mut capsule,
        "platform/lifecycle",
        &Input::Lifecycle {
            state: Lifecycle::Stopped,
        },
    );
    assert!(capsule.resources.is_empty());
}

#[test]
fn paths_and_unbounded_inline_content_fail_closed() {
    assert!(!valid_content_ref(&ContentRef::Dir {
        mount: "shared".into(),
        relative: vec!["..".into()]
    }));
    assert!(!valid_content_ref(&ContentRef::Bytes {
        media_type: "application/octet-stream".into(),
        bytes: vec![0; 8 * 1024 * 1024 + 1]
    }));
}
