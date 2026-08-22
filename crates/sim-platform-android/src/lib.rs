#![deny(unsafe_code)]
//! Android AOT capsule using the unchanged SIM native byte-frame ABI.

use serde::{Deserialize, Serialize};
use sim_kernel::Expr;
use std::collections::{BTreeMap, BTreeSet};

mod ffi;

pub use ffi::{StaticAbiCapsule, sim_native_abi_v1};

pub const NATIVE_ABI_ENTRY: &str = "sim_native_abi_v1";
pub const ARTIFACT: &str = "artifact/sim-platform-android";
pub const LIFECYCLE_FUNCTION: &str = "platform/lifecycle";
pub const ACTIVATION_FUNCTION: &str = "platform/activation";
pub const TARGETS: [&str; 4] = [
    "aarch64-linux-android",
    "armv7-linux-androideabi",
    "x86_64-linux-android",
    "i686-linux-android",
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Lifecycle {
    Created,
    Active,
    Suspended,
    Stopped,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    Camera,
    Microphone,
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
                if matches!(state, Lifecycle::Suspended | Lifecycle::Stopped) {
                    self.resources.clear();
                    self.background_allowed = false;
                }
                true
            }
            (ACTIVATION_FUNCTION, Input::Activation { action, content }) => {
                if action.trim().is_empty() {
                    return Err("Android activation action must not be empty".into());
                }
                if content
                    .as_ref()
                    .is_some_and(|item| !valid_content_ref(item))
                {
                    return Err(
                        "Android activation contained an invalid bounded content ref".into(),
                    );
                }
                if matches!(
                    self.lifecycle,
                    Some(Lifecycle::Suspended | Lifecycle::Stopped)
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
                    return Err("Android notification channel must not be empty".into());
                }
                self.resources.insert(format!("notification:{channel}"))
            }
            (ACTIVATION_FUNCTION, Input::AudioDevice { id, connected }) => {
                if id.trim().is_empty() {
                    return Err("Android audio device id must not be empty".into());
                }
                if connected {
                    self.resources.insert(format!("audio:{id}"))
                } else {
                    self.resources.remove(&format!("audio:{id}"))
                }
            }
            (ACTIVATION_FUNCTION, Input::BackgroundExecution { allowed }) => {
                self.background_allowed = allowed;
                allowed
            }
            (LIFECYCLE_FUNCTION | ACTIVATION_FUNCTION, _) => {
                return Err(format!(
                    "typed input does not match Android function {function}"
                ));
            }
            _ => return Err(format!("unknown Android platform function: {function}")),
        };
        Ok(Output {
            lifecycle: self.lifecycle.unwrap_or(Lifecycle::Created),
            accepted,
            resources: self.resources.len(),
        })
    }
}

/// Encodes a typed Android input inside the canonical SIM binary frame codec.
///
/// # Errors
/// Returns an error if JSON or binary frame encoding fails.
pub fn encode_input_frame(input: &Input) -> Result<Vec<u8>, String> {
    encode_json_frame(input)
}

/// Decodes a typed Android output from the canonical SIM binary frame codec.
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
        return Err("Android ABI frame payload must be one typed JSON string".into());
    };
    serde_json::from_str(&json).map_err(|error| format!("invalid typed Android input: {error}"))
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
