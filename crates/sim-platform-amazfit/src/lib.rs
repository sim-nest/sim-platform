#![forbid(unsafe_code)]
//! Bounded Amazfit proxy state machine hosted by the Android capsule.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};

/// Maximum accepted official-wire frame size.
pub const MAX_FRAME_BYTES: usize = 4096;
/// Maximum pending host-bound events; newest data never displaces accepted work.
pub const MAX_PENDING_EVENTS: usize = 32;
/// Wire protocol version shared with the committed Zepp shell.
pub const PROTOCOL_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Lifecycle {
    Created,
    Active,
    Suspended,
    Destroyed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum WatchEvent {
    Button {
        key: String,
    },
    Touch {
        action: String,
        x: i16,
        y: i16,
    },
    Sensor {
        sensor: String,
        value_milli: i32,
        monotonic_ms: u64,
    },
    Acknowledgement {
        action: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum HostCommand {
    Glance { title: String, body: String },
    Haptic { pattern: String },
    Clear,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Envelope<T> {
    pub version: u8,
    pub session: u64,
    pub sequence: u64,
    pub payload: T,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProxyError {
    Malformed,
    Oversized,
    WrongVersion,
    StaleSession,
    Inactive,
    ConsentRequired,
    QueueFull,
}

/// Companion-owned proxy. Domain consumers see only `WATCH_8` contracts produced by the host adapter.
pub struct AmazfitCapsule {
    lifecycle: Lifecycle,
    consent: bool,
    connected: bool,
    session: u64,
    seen: BTreeSet<u64>,
    pending: VecDeque<Envelope<WatchEvent>>,
}

impl Default for AmazfitCapsule {
    fn default() -> Self {
        Self {
            lifecycle: Lifecycle::Created,
            consent: false,
            connected: false,
            session: 0,
            seen: BTreeSet::new(),
            pending: VecDeque::new(),
        }
    }
}

impl AmazfitCapsule {
    pub fn lifecycle(&mut self, state: Lifecycle) {
        self.lifecycle = state;
        if matches!(state, Lifecycle::Suspended | Lifecycle::Destroyed) {
            self.disconnect();
        }
    }

    pub fn set_consent(&mut self, granted: bool) {
        self.consent = granted;
        if !granted {
            self.disconnect();
        }
    }

    /// Activates a consented capsule for `session`.
    ///
    /// # Errors
    ///
    /// Returns [`ProxyError::ConsentRequired`] without consent or
    /// [`ProxyError::Inactive`] unless the capsule lifecycle is active.
    pub fn connect(&mut self, session: u64) -> Result<(), ProxyError> {
        if !self.consent {
            return Err(ProxyError::ConsentRequired);
        }
        if self.lifecycle != Lifecycle::Active {
            return Err(ProxyError::Inactive);
        }
        self.connected = true;
        self.session = session;
        self.seen.clear();
        self.pending.clear();
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.connected = false;
        self.seen.clear();
        self.pending.clear();
    }

    /// Accepts one bounded event frame from the companion transport.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal for malformed, oversized, stale,
    /// unauthorized, inactive, or over-capacity input.
    pub fn receive(&mut self, frame: &[u8]) -> Result<bool, ProxyError> {
        if frame.len() > MAX_FRAME_BYTES {
            return Err(ProxyError::Oversized);
        }
        if !self.consent {
            return Err(ProxyError::ConsentRequired);
        }
        if !self.connected || self.lifecycle != Lifecycle::Active {
            return Err(ProxyError::Inactive);
        }
        let event: Envelope<WatchEvent> =
            serde_json::from_slice(frame).map_err(|_| ProxyError::Malformed)?;
        if event.version != PROTOCOL_VERSION {
            return Err(ProxyError::WrongVersion);
        }
        if event.session != self.session {
            return Err(ProxyError::StaleSession);
        }
        validate_event(&event.payload)?;
        if self.seen.contains(&event.sequence) {
            return Ok(false);
        }
        if self.pending.len() == MAX_PENDING_EVENTS {
            return Err(ProxyError::QueueFull);
        }
        self.seen.insert(event.sequence);
        self.pending.push_back(event);
        Ok(true)
    }

    pub fn next_event(&mut self) -> Option<Envelope<WatchEvent>> {
        self.pending.pop_front()
    }

    /// Encodes one host command into the bounded companion wire format.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal when the capsule is unauthorized or inactive,
    /// the command is invalid, or the encoded frame exceeds protocol bounds.
    pub fn encode_command(
        &self,
        sequence: u64,
        command: HostCommand,
    ) -> Result<Vec<u8>, ProxyError> {
        if !self.consent {
            return Err(ProxyError::ConsentRequired);
        }
        if !self.connected || self.lifecycle != Lifecycle::Active {
            return Err(ProxyError::Inactive);
        }
        validate_command(&command)?;
        let bytes = serde_json::to_vec(&Envelope {
            version: PROTOCOL_VERSION,
            session: self.session,
            sequence,
            payload: command,
        })
        .map_err(|_| ProxyError::Malformed)?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(ProxyError::Oversized);
        }
        Ok(bytes)
    }
}

fn short(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn validate_event(event: &WatchEvent) -> Result<(), ProxyError> {
    let valid = match event {
        WatchEvent::Button { key } => short(key, 32),
        WatchEvent::Touch { action, .. } => short(action, 32),
        WatchEvent::Sensor { sensor, .. } => short(sensor, 48),
        WatchEvent::Acknowledgement { action } => short(action, 64),
    };
    if valid {
        Ok(())
    } else {
        Err(ProxyError::Malformed)
    }
}
fn validate_command(command: &HostCommand) -> Result<(), ProxyError> {
    let valid = match command {
        HostCommand::Glance { title, body } => short(title, 80) && short(body, 512),
        HostCommand::Haptic { pattern } => short(pattern, 32),
        HostCommand::Clear => true,
    };
    if valid {
        Ok(())
    } else {
        Err(ProxyError::Malformed)
    }
}

#[cfg(test)]
mod tests;
