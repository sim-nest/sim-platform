// conformance: the Amazfit proxy stays bounded and does not claim an unproved route.

use super::*;

fn frame(session: u64, sequence: u64) -> Vec<u8> {
    serde_json::to_vec(&Envelope {
        version: PROTOCOL_VERSION,
        session,
        sequence,
        payload: WatchEvent::Button {
            key: "select".into(),
        },
    })
    .unwrap()
}
fn active() -> AmazfitCapsule {
    let mut c = AmazfitCapsule::default();
    c.lifecycle(Lifecycle::Active);
    c.set_consent(true);
    c.connect(7).unwrap();
    c
}

#[test]
fn malformed_oversized_and_wrong_version_fail_closed() {
    let mut c = active();
    assert_eq!(c.receive(b"{"), Err(ProxyError::Malformed));
    assert_eq!(
        c.receive(&vec![0; MAX_FRAME_BYTES + 1]),
        Err(ProxyError::Oversized)
    );
    let mut value: serde_json::Value = serde_json::from_slice(&frame(7, 1)).unwrap();
    value["version"] = 9.into();
    assert_eq!(
        c.receive(&serde_json::to_vec(&value).unwrap()),
        Err(ProxyError::WrongVersion)
    );
}

#[test]
fn reconnect_rejects_stale_session_and_duplicate_events() {
    let mut c = active();
    assert_eq!(c.receive(&frame(7, 1)), Ok(true));
    assert_eq!(c.receive(&frame(7, 1)), Ok(false));
    c.disconnect();
    c.connect(8).unwrap();
    assert_eq!(c.receive(&frame(7, 2)), Err(ProxyError::StaleSession));
    assert_eq!(c.receive(&frame(8, 1)), Ok(true));
}

#[test]
fn suspension_resume_and_consent_withdrawal_release_state() {
    let mut c = active();
    c.receive(&frame(7, 1)).unwrap();
    c.lifecycle(Lifecycle::Suspended);
    assert_eq!(c.next_event(), None);
    assert_eq!(c.receive(&frame(7, 2)), Err(ProxyError::Inactive));
    c.lifecycle(Lifecycle::Active);
    c.connect(8).unwrap();
    c.set_consent(false);
    assert_eq!(
        c.encode_command(1, HostCommand::Clear),
        Err(ProxyError::ConsentRequired)
    );
}

#[test]
fn queue_is_bounded_and_applies_backpressure_without_eviction() {
    let mut c = active();
    for n in 0..MAX_PENDING_EVENTS {
        assert_eq!(c.receive(&frame(7, n as u64)), Ok(true));
    }
    assert_eq!(c.receive(&frame(7, 99)), Err(ProxyError::QueueFull));
    assert_eq!(c.next_event().unwrap().sequence, 0);
}

#[test]
fn output_is_a_bounded_watch_8_surface_command() {
    let c = active();
    let bytes = c
        .encode_command(
            3,
            HostCommand::Glance {
                title: "SIM".into(),
                body: "Ready".into(),
            },
        )
        .unwrap();
    assert!(bytes.len() <= MAX_FRAME_BYTES);
    assert_eq!(
        serde_json::from_slice::<Envelope<HostCommand>>(&bytes)
            .unwrap()
            .session,
        7
    );
}
