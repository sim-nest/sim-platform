// conformance: MIDI realization advertises only explicit capsule-owned bindings.

use super::*;
use sim_lib_midi_core::{MidiDirection, MidiTimestampSource};

fn card(id: &str, api: NativeMidiApi) -> NativeMidiBinding {
    NativeMidiBinding {
        api,
        card: MidiPortCard {
            id: MidiPortId(id.into()),
            label: id.into(),
            direction: MidiDirection::Duplex,
            transport: format!("{api:?}"),
            timestamp_source: MidiTimestampSource::Monotonic,
            hotplug: true,
        },
    }
}

#[test]
fn ubuntu_port_preserves_bytes_order_timestamps_hotplug_reconnect_and_cleanup() {
    let first = MidiPortMessage::new(41, vec![0x90, 60, 100]).unwrap();
    let second = MidiPortMessage::new(42, vec![0xf8]).unwrap();
    let mut port = UbuntuMidiPort::default();
    port.bind(
        card("alsa/0", NativeMidiApi::AlsaRtMidi),
        [
            NativeMidiEvent::Message(first.clone()),
            NativeMidiEvent::Message(second.clone()),
            NativeMidiEvent::DeviceLost,
        ],
    );
    let policy = MidiPortPolicy::bounded(1, 3, 1).unwrap();
    assert_eq!(
        port.cards(policy).unwrap()[0].id,
        MidiPortId("alsa/0".into())
    );
    let mut connection = port.open(&MidiPortId("alsa/0".into()), policy).unwrap();
    assert_eq!(connection.receive().unwrap(), Some(first));
    assert_eq!(connection.receive().unwrap(), Some(second));
    assert_eq!(connection.receive(), Err(MidiPortRefusal::DeviceLost));
    connection.reconnect().unwrap();
    connection
        .send(MidiPortMessage::new(43, vec![0x80, 60, 0]).unwrap())
        .unwrap();
    connection.close().unwrap();
    assert_eq!(connection.close(), Err(MidiPortRefusal::AlreadyClosed));
}

#[test]
fn absent_denied_invalid_and_backpressured_ports_fail_closed_without_fallback() {
    let policy = MidiPortPolicy::bounded(1, 1, 0).unwrap();
    assert_eq!(UbuntuMidiPort::default().cards(policy), Ok(Vec::new()));
    assert!(matches!(
        UbuntuMidiPort::default().open(&MidiPortId("unknown".into()), policy),
        Err(MidiPortRefusal::Unsupported)
    ));
    assert_eq!(
        UbuntuMidiPort::denied().cards(policy),
        Err(MidiPortRefusal::Denied)
    );
    assert_eq!(
        MidiPortMessage::new(0, vec![0x90, 0x80]),
        Err(MidiPortRefusal::InvalidMessage)
    );
    let mut port = UbuntuMidiPort::default();
    port.bind(
        card("bluez/0", NativeMidiApi::BluezBleMidi),
        [
            NativeMidiEvent::Message(MidiPortMessage::new(1, vec![0xf8]).unwrap()),
            NativeMidiEvent::Message(MidiPortMessage::new(2, vec![0xf8]).unwrap()),
        ],
    );
    assert!(matches!(
        port.open(&MidiPortId("bluez/0".into()), policy),
        Err(MidiPortRefusal::Backpressure)
    ));
}
