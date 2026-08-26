#![forbid(unsafe_code)]
//! Capsule-owned native MIDI selection and realization.

use sim_lib_midi_core::{
    MidiConnection, MidiPort, MidiPortCard, MidiPortId, MidiPortMessage, MidiPortPolicy,
    MidiPortRefusal,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
};

/// Native API selected explicitly by the containing platform capsule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeMidiApi {
    /// Linux ALSA sequencer through `RtMidi`.
    AlsaRtMidi,
    /// macOS `CoreMIDI` through `RtMidi`.
    CoreMidiRtMidi,
    /// Windows multimedia MIDI through `RtMidi`.
    WinMmRtMidi,
    /// Linux BLE-MIDI through `BlueZ` D-Bus and GATT.
    BluezBleMidi,
}

/// A capsule registration. Native identities never escape in the Card.
#[derive(Clone, Debug)]
pub struct NativeMidiBinding {
    /// Selected physical API.
    pub api: NativeMidiApi,
    /// Provider-neutral Card exposed to music code.
    pub card: MidiPortCard,
}

/// Physical event supplied by the capsule loader or native callback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeMidiEvent {
    /// Ordered input message.
    Message(MidiPortMessage),
    /// Device disappeared after it was opened.
    DeviceLost,
}

#[derive(Default)]
struct BindingState {
    input: VecDeque<NativeMidiEvent>,
    connected: bool,
}

/// Ubuntu/native MIDI capsule. Discovery is registration-only: it never probes
/// another backend when a configured API is absent or denied.
#[derive(Default)]
pub struct UbuntuMidiPort {
    bindings: BTreeMap<MidiPortId, NativeMidiBinding>,
    states: BTreeMap<MidiPortId, Mutex<BindingState>>,
    denied: bool,
}

impl UbuntuMidiPort {
    /// Constructs a denied capsule for authority conformance tests.
    #[must_use]
    pub fn denied() -> Self {
        Self {
            denied: true,
            ..Self::default()
        }
    }

    /// Registers one loader-realized native endpoint and its bounded callback script.
    pub fn bind(
        &mut self,
        binding: NativeMidiBinding,
        input: impl IntoIterator<Item = NativeMidiEvent>,
    ) {
        let id = binding.card.id.clone();
        self.bindings.insert(id.clone(), binding);
        self.states.insert(
            id,
            Mutex::new(BindingState {
                input: input.into_iter().collect(),
                connected: true,
            }),
        );
    }
}

impl MidiPort for UbuntuMidiPort {
    fn cards(&self, policy: MidiPortPolicy) -> Result<Vec<MidiPortCard>, MidiPortRefusal> {
        if self.denied {
            return Err(MidiPortRefusal::Denied);
        }
        if self.bindings.len() > policy.max_devices {
            return Err(MidiPortRefusal::DiscoveryLimit);
        }
        Ok(self
            .bindings
            .values()
            .map(|binding| binding.card.clone())
            .collect())
    }

    fn open(
        &self,
        id: &MidiPortId,
        policy: MidiPortPolicy,
    ) -> Result<Box<dyn MidiConnection>, MidiPortRefusal> {
        if self.denied {
            return Err(MidiPortRefusal::Denied);
        }
        let binding = self
            .bindings
            .get(id)
            .ok_or(MidiPortRefusal::Unsupported)?
            .clone();
        let state = self
            .states
            .get(id)
            .ok_or(MidiPortRefusal::Unavailable)?
            .lock()
            .map_err(|_| MidiPortRefusal::Unavailable)?;
        if state.input.len() > policy.queue_capacity {
            return Err(MidiPortRefusal::Backpressure);
        }
        Ok(Box::new(UbuntuMidiConnection {
            binding,
            input: state.input.clone(),
            sent: Vec::new(),
            connected: state.connected,
            closed: false,
            reconnects: 0,
            reconnect_limit: policy.reconnect_attempts,
        }))
    }
}

struct UbuntuMidiConnection {
    binding: NativeMidiBinding,
    input: VecDeque<NativeMidiEvent>,
    sent: Vec<MidiPortMessage>,
    connected: bool,
    closed: bool,
    reconnects: u16,
    reconnect_limit: u16,
}

impl MidiConnection for UbuntuMidiConnection {
    fn card(&self) -> &MidiPortCard {
        &self.binding.card
    }
    fn receive(&mut self) -> Result<Option<MidiPortMessage>, MidiPortRefusal> {
        if self.closed {
            return Err(MidiPortRefusal::AlreadyClosed);
        }
        if !self.connected {
            return Err(MidiPortRefusal::DeviceLost);
        }
        match self.input.pop_front() {
            Some(NativeMidiEvent::Message(message)) => Ok(Some(message)),
            Some(NativeMidiEvent::DeviceLost) => {
                self.connected = false;
                Err(MidiPortRefusal::DeviceLost)
            }
            None => Ok(None),
        }
    }
    fn send(&mut self, message: MidiPortMessage) -> Result<(), MidiPortRefusal> {
        if self.closed {
            Err(MidiPortRefusal::AlreadyClosed)
        } else if !self.connected {
            Err(MidiPortRefusal::DeviceLost)
        } else {
            self.sent.push(message);
            Ok(())
        }
    }
    fn reconnect(&mut self) -> Result<(), MidiPortRefusal> {
        if self.reconnects >= self.reconnect_limit {
            return Err(MidiPortRefusal::ReconnectLimit);
        }
        self.reconnects += 1;
        self.connected = true;
        self.closed = false;
        Ok(())
    }
    fn close(&mut self) -> Result<(), MidiPortRefusal> {
        if self.closed {
            Err(MidiPortRefusal::AlreadyClosed)
        } else {
            self.closed = true;
            self.connected = false;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
