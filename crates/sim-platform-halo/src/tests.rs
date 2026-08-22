use super::*;

fn active(transport: HostTransport) -> HaloCapsule {
    let mut capsule = HaloCapsule::new(transport);
    capsule.lifecycle(Lifecycle::Active);
    capsule.set_consent(true);
    capsule.connect(0x1020_3040).unwrap();
    capsule
}

fn input(kind: MessageKind, sequence: u32, payload: Vec<u8>) -> Vec<Vec<u8>> {
    encode_message(&Message {
        kind,
        session: 0x1020_3040,
        sequence,
        payload,
    })
    .unwrap()
}

#[test]
fn all_official_host_routes_share_one_codec() {
    for route in [
        HostTransport::Android,
        HostTransport::Ios,
        HostTransport::WebBluetooth,
    ] {
        let capsule = active(route);
        let frames = capsule
            .encode_surface(
                7,
                &SurfaceCommand::Display {
                    cells: vec![b'x'; 512],
                },
            )
            .unwrap();
        assert_eq!(frames.len(), 3);
        assert!(
            frames
                .iter()
                .all(|frame| frame.len() <= HEADER_BYTES + MAX_FRAGMENT_BYTES)
        );
    }
}

#[test]
fn fragmentation_normalizes_glasses_8_input() {
    let mut capsule = active(HostTransport::Android);
    let mut payload = vec![1];
    payload.extend(std::iter::repeat_n(b'a', 200));
    let frames = input(MessageKind::Button, 1, payload);
    assert_eq!(capsule.receive_fragment(&frames[0]), Ok(false));
    assert_eq!(
        capsule.receive_fragment(&frames[1]),
        Err(ProxyError::Malformed)
    );
    let frames = input(
        MessageKind::Button,
        2,
        [vec![1], b"select".to_vec()].concat(),
    );
    assert_eq!(capsule.receive_fragment(&frames[0]), Ok(true));
    assert_eq!(
        capsule.next_input(),
        Some(GlassesInput::Button {
            name: "select".into(),
            pressed: true
        })
    );
}

#[test]
fn malformed_order_replay_and_stale_sessions_fail_closed() {
    let mut capsule = active(HostTransport::Ios);
    assert_eq!(capsule.receive_fragment(b"bad"), Err(ProxyError::Malformed));
    let mut fragmented = input(
        MessageKind::Sensor,
        3,
        [0_i32.to_be_bytes().as_slice(), &[b's'; 200]].concat(),
    );
    assert_eq!(
        capsule.receive_fragment(&fragmented[1]),
        Err(ProxyError::FragmentOrder)
    );
    fragmented[0][2] = 9;
    assert_eq!(
        capsule.receive_fragment(&fragmented[0]),
        Err(ProxyError::WrongVersion)
    );
    let valid = input(MessageKind::LinkState, 4, vec![1]);
    assert_eq!(capsule.receive_fragment(&valid[0]), Ok(true));
    assert_eq!(capsule.receive_fragment(&valid[0]), Err(ProxyError::Replay));
    let mut stale = input(MessageKind::LinkState, 5, vec![1]);
    stale[0][7] ^= 1;
    assert_eq!(
        capsule.receive_fragment(&stale[0]),
        Err(ProxyError::StaleSession)
    );
}

#[test]
fn reconnect_suspend_and_consent_withdrawal_reap_state() {
    let mut capsule = active(HostTransport::WebBluetooth);
    let valid = input(MessageKind::LinkState, 1, vec![1]);
    capsule.receive_fragment(&valid[0]).unwrap();
    capsule.lifecycle(Lifecycle::Suspended);
    assert_eq!(capsule.next_input(), None);
    assert_eq!(
        capsule.receive_fragment(&valid[0]),
        Err(ProxyError::Inactive)
    );
    capsule.lifecycle(Lifecycle::Active);
    capsule.connect(9).unwrap();
    capsule.set_consent(false);
    assert_eq!(
        capsule.encode_surface(1, &SurfaceCommand::Clear),
        Err(ProxyError::ConsentRequired)
    );
}

#[test]
fn queue_and_surface_are_bounded_without_eviction() {
    let mut capsule = active(HostTransport::Android);
    for sequence in 0..MAX_PENDING_EVENTS as u32 {
        let frame = input(MessageKind::LinkState, sequence, vec![1]);
        assert_eq!(capsule.receive_fragment(&frame[0]), Ok(true));
    }
    let frame = input(MessageKind::LinkState, 99, vec![1]);
    assert_eq!(
        capsule.receive_fragment(&frame[0]),
        Err(ProxyError::QueueFull)
    );
    assert_eq!(
        capsule.next_input(),
        Some(GlassesInput::LinkState { connected: true })
    );
    assert_eq!(
        capsule.encode_surface(
            1,
            &SurfaceCommand::Display {
                cells: vec![0; 1025]
            }
        ),
        Err(ProxyError::Oversized)
    );
}
