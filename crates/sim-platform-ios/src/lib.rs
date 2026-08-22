#![deny(unsafe_code)]
//! iOS AOT capsule using the unchanged SIM native byte-frame ABI.

use serde::{Deserialize, Serialize};
use sim_kernel::Expr;
use std::collections::{BTreeMap, BTreeSet};

mod ffi;

pub use ffi::{StaticAbiCapsule, sim_native_abi_v1};

pub const NATIVE_ABI_ENTRY: &str = "sim_native_abi_v1";
pub const ARTIFACT: &str = "artifact/sim-platform-ios";
pub const LIFECYCLE_FUNCTION: &str = "platform/lifecycle";
pub const ACTIVATION_FUNCTION: &str = "platform/activation";
pub const TARGETS: [&str; 3] = [
    "aarch64-apple-ios",
    "aarch64-apple-ios-sim",
    "x86_64-apple-ios",
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Lifecycle {
    Connected,
    Active,
    Suspended,
    Disconnected,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    Camera,
    Microphone,
    Notifications,
    SharedDocument,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Service {
    Audio,
    Notifications,
    SharedDocument,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ContentRef {
    Table {
        mount: String,
        key: Vec<String>,
    },
    Dir {
        mount: String,
        relative: Vec<String>,
    },
    Bytes {
        media_type: String,
        bytes: Vec<u8>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Input {
    Lifecycle {
        state: Lifecycle,
    },
    Activation {
        action: String,
        content: Option<ContentRef>,
    },
    Permission {
        permission: Permission,
        granted: bool,
    },
    Notification {
        channel: String,
        payload: Vec<u8>,
    },
    AudioDevice {
        id: String,
        connected: bool,
    },
    BackgroundExecution {
        allowed: bool,
    },
    DocumentGrant {
        id: String,
        active: bool,
    },
    Service {
        service: Service,
    },
    MemoryPressure,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Output {
    pub lifecycle: Lifecycle,
    pub accepted: bool,
    pub resources: usize,
}

#[derive(Default)]
pub struct Capsule {
    lifecycle: Option<Lifecycle>,
    permissions: BTreeMap<Permission, bool>,
    resources: BTreeSet<String>,
    background_allowed: bool,
    document_grants: BTreeSet<String>,
}

impl Capsule {
    /// Calls a named capsule function with a canonical SIM binary frame.
    ///
    /// # Errors
    /// Returns a closed error for malformed frames, wrong input kinds, unknown
    /// functions, invalid content references, or frame-encoding failures.
    pub fn call_frame(&mut self, function: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
        let input = decode_input_frame(bytes)?;
        let output = self.dispatch(function, input)?;
        encode_output_frame(&output)
    }

    /// Dispatches one already-decoded typed platform input.
    ///
    /// # Errors
    /// Returns a closed error if the function and input type disagree or an
    /// activation carries an invalid bounded content reference.
    pub fn dispatch(&mut self, function: &str, input: Input) -> Result<Output, String> {
        let accepted = match (function, input) {
            (LIFECYCLE_FUNCTION, Input::Lifecycle { state }) => {
                self.lifecycle = Some(state);
                if matches!(state, Lifecycle::Suspended | Lifecycle::Disconnected) {
                    self.resources.clear();
                    self.background_allowed = false;
                    self.document_grants.clear();
                }
                true
            }
            (ACTIVATION_FUNCTION, Input::Activation { action, content }) => {
                if action.trim().is_empty() {
                    return Err("iOS activation action must not be empty".into());
                }
                if content
                    .as_ref()
                    .is_some_and(|item| !valid_content_ref(item))
                {
                    return Err("iOS activation contained an invalid bounded content ref".into());
                }
                if matches!(
                    self.lifecycle,
                    Some(Lifecycle::Suspended | Lifecycle::Disconnected)
                ) {
                    false
                } else {
                    self.resources.insert(format!("activation:{action}"));
                    true
                }
            }
            (
                ACTIVATION_FUNCTION,
                Input::Permission {
                    permission,
                    granted,
                },
            ) => {
                self.permissions.insert(permission, granted);
                granted
            }
            (ACTIVATION_FUNCTION, Input::Notification { channel, .. }) => {
                if channel.trim().is_empty() {
                    return Err("iOS notification channel must not be empty".into());
                }
                if self.available(Service::Notifications) {
                    self.resources.insert(format!("notification:{channel}"))
                } else {
                    false
                }
            }
            (ACTIVATION_FUNCTION, Input::AudioDevice { id, connected }) => {
                if id.trim().is_empty() {
                    return Err("iOS audio device id must not be empty".into());
                }
                if connected && self.available(Service::Audio) {
                    self.resources.insert(format!("audio:{id}"))
                } else {
                    self.resources.remove(&format!("audio:{id}"))
                }
            }
            (ACTIVATION_FUNCTION, Input::BackgroundExecution { allowed }) => {
                self.background_allowed = allowed && self.is_active();
                self.background_allowed
            }
            (ACTIVATION_FUNCTION, Input::DocumentGrant { id, active }) => {
                if id.trim().is_empty() {
                    return Err("iOS document grant id must not be empty".into());
                }
                if active && self.is_active() {
                    self.document_grants.insert(id)
                } else {
                    self.document_grants.remove(&id);
                    false
                }
            }
            (ACTIVATION_FUNCTION, Input::Service { service }) => self.available(service),
            (ACTIVATION_FUNCTION, Input::MemoryPressure) => {
                self.resources.clear();
                true
            }
            (LIFECYCLE_FUNCTION | ACTIVATION_FUNCTION, _) => {
                return Err(format!(
                    "typed input does not match iOS function {function}"
                ));
            }
            _ => return Err(format!("unknown iOS platform function: {function}")),
        };
        Ok(Output {
            lifecycle: self.lifecycle.unwrap_or(Lifecycle::Connected),
            accepted,
            resources: self.resources.len(),
        })
    }

    fn is_active(&self) -> bool {
        matches!(
            self.lifecycle,
            Some(Lifecycle::Active | Lifecycle::Connected)
        )
    }

    fn available(&self, service: Service) -> bool {
        if !self.is_active() {
            return false;
        }
        match service {
            Service::Audio => self
                .permissions
                .get(&Permission::Microphone)
                .copied()
                .unwrap_or(false),
            Service::Notifications => self
                .permissions
                .get(&Permission::Notifications)
                .copied()
                .unwrap_or(false),
            Service::SharedDocument => !self.document_grants.is_empty(),
        }
    }
}

/// Encodes a typed iOS input inside the canonical SIM binary frame codec.
///
/// # Errors
/// Returns an error if JSON or binary frame encoding fails.
pub fn encode_input_frame(input: &Input) -> Result<Vec<u8>, String> {
    encode_json_frame(input)
}

/// Decodes a typed iOS output from the canonical SIM binary frame codec.
///
/// # Errors
/// Returns an error if the frame is malformed or does not contain a valid
/// typed output packet.
pub fn decode_output_frame(bytes: &[u8]) -> Result<Output, String> {
    decode_json_frame(bytes)
}

fn encode_output_frame(output: &Output) -> Result<Vec<u8>, String> {
    encode_json_frame(output)
}

fn decode_input_frame(bytes: &[u8]) -> Result<Input, String> {
    decode_json_frame(bytes)
}

fn encode_json_frame(value: &impl Serialize) -> Result<Vec<u8>, String> {
    let json = serde_json::to_string(value).map_err(|error| error.to_string())?;
    sim_codec_binary::encode_frame(&Expr::String(json))
        .map(|frame| frame.0)
        .map_err(|error| error.to_string())
}

fn decode_json_frame<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, String> {
    let (_, expr) = sim_codec_binary::decode_frame(sim_kernel::CodecId(0), bytes)
        .map_err(|error| format!("invalid SIM binary frame: {error}"))?;
    let Expr::String(json) = expr else {
        return Err("iOS ABI frame payload must be one typed JSON string".into());
    };
    serde_json::from_str(&json).map_err(|error| format!("invalid typed iOS input: {error}"))
}

fn valid_content_ref(reference: &ContentRef) -> bool {
    match reference {
        ContentRef::Table { mount, key }
        | ContentRef::Dir {
            mount,
            relative: key,
        } => {
            !mount.is_empty()
                && mount.len() <= 128
                && !mount.contains('/')
                && key.len() <= 32
                && key.iter().all(|part| {
                    !part.is_empty()
                        && part.len() <= 255
                        && part != ".."
                        && !part.contains(['/', '\\'])
                })
        }
        ContentRef::Bytes { media_type, bytes } => {
            !media_type.trim().is_empty()
                && media_type.len() <= 255
                && bytes.len() <= 8 * 1024 * 1024
        }
    }
}

#[cfg(test)]
mod tests;
