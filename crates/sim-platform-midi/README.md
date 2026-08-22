# sim-platform-midi

Capsule-owned BlueZ and RtMidi realization behind `sim-lib-midi-core::MidiPort`.
Discovery is bounded and registration-based; absent hardware returns `Unsupported`
and is never replaced by a probe-selected fallback.
