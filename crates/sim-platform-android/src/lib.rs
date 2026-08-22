#![forbid(unsafe_code)]
//! Android AOT capsule using the unchanged SIM native byte-frame ABI.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const NATIVE_ABI_ENTRY: &str = "sim_native_abi_v1";
pub const ARTIFACT: &str = "artifact/sim-platform-android";
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

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
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
    /// Calls a named function with the bytes carried by `NativeLibAbiV1::call`.
    pub fn call(&mut self, function: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
        if function != "platform/lifecycle" && function != "platform/activation" {
            return Err(format!("unknown Android platform function: {function}"));
        }
        let input: Input =
            serde_json::from_slice(bytes).map_err(|e| format!("invalid typed input: {e}"))?;
        let accepted = match input {
            Input::Lifecycle { state } => {
                self.lifecycle = Some(state);
                if state == Lifecycle::Stopped {
                    self.resources.clear();
                }
                true
            }
            Input::Activation { action, content } => {
                if self.lifecycle == Some(Lifecycle::Suspended)
                    || self.lifecycle == Some(Lifecycle::Stopped)
                {
                    false
                } else {
                    self.resources.insert(format!("activation:{action}"));
                    content.as_ref().is_none_or(valid_content_ref)
                }
            }
            Input::Permission {
                permission,
                granted,
            } => {
                self.permissions.insert(permission, granted);
                granted
            }
            Input::Notification { channel, .. } => {
                self.resources.insert(format!("notification:{channel}"))
            }
            Input::AudioDevice { id, connected } => {
                if connected {
                    self.resources.insert(format!("audio:{id}"))
                } else {
                    self.resources.remove(&format!("audio:{id}"))
                }
            }
            Input::BackgroundExecution { allowed } => {
                self.background_allowed = allowed;
                allowed
            }
        };
        serde_json::to_vec(&Output {
            lifecycle: self.lifecycle.unwrap_or(Lifecycle::Created),
            accepted,
            resources: self.resources.len(),
        })
        .map_err(|e| e.to_string())
    }
}

fn valid_content_ref(reference: &ContentRef) -> bool {
    match reference {
        ContentRef::Table { mount, key }
        | ContentRef::Dir {
            mount,
            relative: key,
        } => {
            !mount.is_empty()
                && key.len() <= 32
                && key
                    .iter()
                    .all(|part| !part.is_empty() && part != ".." && !part.contains('/'))
        }
        ContentRef::Bytes { bytes, .. } => bytes.len() <= 8 * 1024 * 1024,
    }
}

#[cfg(test)]
mod tests;
