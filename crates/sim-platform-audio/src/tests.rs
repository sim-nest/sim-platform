// conformance: media realization exposes only explicit native bindings and bounded callbacks.

use super::*;

#[test]
fn ubuntu_capsule_enumerates_explicit_bindings_and_refuses_absent_apis() {
    let mut capsule = UbuntuMediaCapsule::default();
    capsule.bind_audio(UbuntuAudioBinding {
        api: AudioApi::PipeWire,
        card: AudioDeviceCard {
            id: NativeId("pw/0".into()),
            label: "PipeWire 0".into(),
            input_channels: 2,
            output_channels: 2,
            sample_rates: vec![48_000],
            hotplug: true,
        },
    });
    capsule.bind_plugin(UbuntuPluginBinding {
        api: PluginApi::Clap,
        card: PluginCard {
            id: NativeId("clap/gain".into()),
            label: "Gain".into(),
            abi: 3,
            audio_inputs: 2,
            audio_outputs: 2,
            isolated: true,
        },
    });
    assert_eq!(AudioDevicePort::cards(&capsule).unwrap().len(), 1);
    assert_eq!(NativePluginPort::cards(&capsule).unwrap().len(), 1);
    assert!(matches!(
        AudioDevicePort::open(
            &capsule,
            &NativeId("alsa/missing".into()),
            48_000,
            RealtimeBudget::strict(64, 2, 4)
        ),
        Err(NativeRefusal::Unsupported)
    ));
    assert!(matches!(
        capsule.load(
            &NativeId("vst3/missing".into()),
            3,
            RealtimeBudget::strict(64, 2, 4)
        ),
        Err(NativeRefusal::Unsupported)
    ));
    assert!(matches!(
        capsule.load(
            &NativeId("clap/gain".into()),
            2,
            RealtimeBudget::strict(64, 2, 4)
        ),
        Err(NativeRefusal::AbiMismatch {
            expected: 2,
            found: 3
        })
    ));
    let mut stream = AudioDevicePort::open(
        &capsule,
        &NativeId("pw/0".into()),
        48_000,
        RealtimeBudget::strict(64, 2, 4),
    )
    .unwrap();
    stream.close().unwrap();
    assert_eq!(stream.close(), Err(NativeRefusal::AlreadyClosed));
    let mut plugin = capsule
        .load(
            &NativeId("clap/gain".into()),
            3,
            RealtimeBudget::strict(64, 2, 4),
        )
        .unwrap();
    plugin.process(64).unwrap();
    assert_eq!(plugin.process(65), Err(NativeRefusal::BudgetExceeded));
    plugin.unload().unwrap();
    assert_eq!(plugin.unload(), Err(NativeRefusal::AlreadyClosed));
}

#[test]
fn hotplug_is_deterministic_and_drained_once() {
    let capsule = UbuntuMediaCapsule::default();
    let id = NativeId("pw/new".into());
    capsule
        .record_hotplug(DeviceEvent::Removed(id.clone()))
        .unwrap();
    assert_eq!(
        capsule.poll_hotplug().unwrap(),
        vec![DeviceEvent::Removed(id)]
    );
    assert!(capsule.poll_hotplug().unwrap().is_empty());
}
