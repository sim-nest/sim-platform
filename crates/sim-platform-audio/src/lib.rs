#![forbid(unsafe_code)]
//! Native media realization owned by the platform capsule.

use sim_audio_ports::{
    AudioDeviceCard, AudioDevicePort, AudioDeviceStream, CallbackEvent, CallbackOwner, DeviceEvent,
    NativeId, NativePluginInstance, NativePluginPort, NativeRefusal, PluginCard, RealtimeBudget,
};
use std::{collections::BTreeMap, sync::Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioApi {
    Alsa,
    Asio,
    CoreAudio,
    Jack,
    PipeWire,
    PortAudio,
    Cpal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginApi {
    Lv2,
    Vst3,
    Clap,
}

#[derive(Clone, Debug)]
pub struct UbuntuAudioBinding {
    pub api: AudioApi,
    pub card: AudioDeviceCard,
}

#[derive(Clone, Debug)]
pub struct UbuntuPluginBinding {
    pub api: PluginApi,
    pub card: PluginCard,
}

/// Ubuntu capsule. Native handles are supplied by the capsule loader and never
/// discovered through target predicates, environment variables, or fallback.
#[derive(Default)]
pub struct UbuntuMediaCapsule {
    audio: BTreeMap<NativeId, UbuntuAudioBinding>,
    plugins: BTreeMap<NativeId, UbuntuPluginBinding>,
    hotplug: Mutex<Vec<DeviceEvent>>,
}

impl UbuntuMediaCapsule {
    pub fn bind_audio(&mut self, binding: UbuntuAudioBinding) {
        self.audio.insert(binding.card.id.clone(), binding);
    }
    pub fn bind_plugin(&mut self, binding: UbuntuPluginBinding) {
        self.plugins.insert(binding.card.id.clone(), binding);
    }
    /// Queues one capsule-observed hotplug event.
    ///
    /// # Errors
    ///
    /// Returns [`NativeRefusal::Unavailable`] if another thread poisoned the
    /// bounded control-side event queue. The realtime callback never uses it.
    pub fn record_hotplug(&self, event: DeviceEvent) -> Result<(), NativeRefusal> {
        self.hotplug
            .lock()
            .map_err(|_| NativeRefusal::Unavailable)?
            .push(event);
        Ok(())
    }
}

impl AudioDevicePort for UbuntuMediaCapsule {
    fn cards(&self) -> Result<Vec<AudioDeviceCard>, NativeRefusal> {
        Ok(self.audio.values().map(|v| v.card.clone()).collect())
    }
    fn poll_hotplug(&self) -> Result<Vec<DeviceEvent>, NativeRefusal> {
        Ok(std::mem::take(
            &mut *self
                .hotplug
                .lock()
                .map_err(|_| NativeRefusal::Unavailable)?,
        ))
    }
    fn open(
        &self,
        id: &NativeId,
        sample_rate: u32,
        budget: RealtimeBudget,
    ) -> Result<Box<dyn AudioDeviceStream>, NativeRefusal> {
        budget.validate()?;
        let binding = self.audio.get(id).ok_or(NativeRefusal::Unsupported)?;
        if !binding.card.sample_rates.contains(&sample_rate) {
            return Err(NativeRefusal::Unsupported);
        }
        Ok(Box::new(UbuntuStream {
            owner: CallbackOwner::new(id.clone()),
            closed: false,
        }))
    }
}

impl NativePluginPort for UbuntuMediaCapsule {
    fn cards(&self) -> Result<Vec<PluginCard>, NativeRefusal> {
        Ok(self.plugins.values().map(|v| v.card.clone()).collect())
    }
    fn load(
        &self,
        id: &NativeId,
        expected_abi: u32,
        budget: RealtimeBudget,
    ) -> Result<Box<dyn NativePluginInstance>, NativeRefusal> {
        budget.validate()?;
        let binding = self.plugins.get(id).ok_or(NativeRefusal::Unsupported)?;
        if binding.card.abi != expected_abi {
            return Err(NativeRefusal::AbiMismatch {
                expected: expected_abi,
                found: binding.card.abi,
            });
        }
        Ok(Box::new(UbuntuPlugin {
            card: binding.card.clone(),
            budget,
            unloaded: false,
        }))
    }
}

struct UbuntuStream {
    owner: CallbackOwner,
    closed: bool,
}

impl AudioDeviceStream for UbuntuStream {
    fn callback_owner(&self) -> &CallbackOwner {
        &self.owner
    }

    fn poll_callback(&mut self) -> Result<Option<CallbackEvent>, NativeRefusal> {
        if self.closed {
            Err(NativeRefusal::AlreadyClosed)
        } else {
            Ok(None)
        }
    }

    fn cancel(&mut self) -> Result<(), NativeRefusal> {
        if self.closed {
            Err(NativeRefusal::AlreadyClosed)
        } else {
            self.closed = true;
            Err(NativeRefusal::Cancelled)
        }
    }

    fn close(&mut self) -> Result<(), NativeRefusal> {
        if self.closed {
            Err(NativeRefusal::AlreadyClosed)
        } else {
            self.closed = true;
            Ok(())
        }
    }
}

struct UbuntuPlugin {
    card: PluginCard,
    budget: RealtimeBudget,
    unloaded: bool,
}

impl NativePluginInstance for UbuntuPlugin {
    fn card(&self) -> &PluginCard {
        &self.card
    }

    fn process(&mut self, frames: u32) -> Result<(), NativeRefusal> {
        if self.unloaded {
            Err(NativeRefusal::AlreadyClosed)
        } else if frames > self.budget.max_frames {
            Err(NativeRefusal::BudgetExceeded)
        } else {
            Ok(())
        }
    }

    fn cancel(&mut self) -> Result<(), NativeRefusal> {
        if self.unloaded {
            Err(NativeRefusal::AlreadyClosed)
        } else {
            self.unloaded = true;
            Err(NativeRefusal::Cancelled)
        }
    }

    fn unload(&mut self) -> Result<(), NativeRefusal> {
        if self.unloaded {
            Err(NativeRefusal::AlreadyClosed)
        } else {
            self.unloaded = true;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
