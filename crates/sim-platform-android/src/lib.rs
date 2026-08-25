#![deny(unsafe_code)]
//! Android AOT capsule using the unchanged SIM native byte-frame ABI.

use serde::{Deserialize, Serialize};
use sim_kernel::Expr;
use std::collections::{BTreeMap, BTreeSet};

mod audio;
mod ffi;
mod speech;

pub use audio::{
    AudioInput, AudioRouteClass, AudioRouteReceipt, AudioSessionSpec, AudioStopReason, PcmContract,
    RouteEvidence, RouteObservation, RoutingContract,
};
pub use ffi::{StaticAbiCapsule, sim_native_abi_v1};
pub use speech::{
    LocalSpeechEvidence, SpeechFallback, SpeechInput, SpeechKind, SpeechLanguage, SpeechOutput,
    SpeechStopReason, SpeechTier,
};

pub const NATIVE_ABI_ENTRY: &str = "sim_native_abi_v1";
pub const ARTIFACT: &str = "artifact/sim-platform-android";
pub const LIFECYCLE_FUNCTION: &str = "platform/lifecycle";
pub const ACTIVATION_FUNCTION: &str = "platform/activation";
pub const CONTINUITY_FUNCTION: &str = "platform/continuity";
pub const AUDIO_FUNCTION: &str = "platform/android-audio";
pub const SPEECH_FUNCTION: &str = "platform/android-speech";
pub const REQUIRED_SERVICES: [&str; 6] = [
    "mount/app-private",
    "lifecycle",
    "foreground",
    "permissions",
    "activations",
    "provider-table",
];
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

/// Android facts are inputs to Rust, never implicit control flow in Kotlin.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AndroidEvent {
    Rotation,
    ActivityRecreation,
    BackgroundRestriction,
    Suspend,
    ProcessDeath,
    Restart,
    MemoryPressure,
}

/// One immutable, content-addressed view of all platform providers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderSnapshot {
    pub content_id: String,
    pub providers: BTreeMap<String, String>,
}

/// Complete Android authority bundle. Partial installation is forbidden.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BindPlan {
    pub snapshot: ProviderSnapshot,
    pub app_private_mount: String,
    pub services: BTreeMap<String, String>,
}

/// Portable continuity record. Every identity is a content id or journal id.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RestorePlan {
    pub snapshot_content_id: String,
    pub journal_content_id: String,
    pub pending_turn_content_id: Option<String>,
    pub permission_observations: BTreeMap<Permission, bool>,
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
    Bind {
        plan: BindPlan,
    },
    Event {
        event: AndroidEvent,
    },
    Restore {
        plan: RestorePlan,
    },
    SubmitTurn {
        content_id: String,
    },
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
    Audio {
        input: AudioInput,
    },
    Speech {
        input: SpeechInput,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_content_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_turn_content_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_route: Option<AudioRouteReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech: Option<SpeechOutput>,
}

#[derive(Default)]
pub struct Capsule {
    lifecycle: Option<Lifecycle>,
    permissions: BTreeMap<Permission, bool>,
    resources: BTreeSet<String>,
    background_allowed: bool,
    binding: Option<BindPlan>,
    pending_turn_content_id: Option<String>,
    resumed_turns: BTreeSet<String>,
    audio: audio::AndroidAudioState,
    speech: speech::AndroidSpeechState,
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
        let mut resumed_turn_content_id = None;
        let mut audio_route = None;
        let mut speech = None;
        let accepted = match (function, input) {
            (CONTINUITY_FUNCTION, input) => {
                let outcome = self.dispatch_continuity(input)?;
                resumed_turn_content_id = outcome.1;
                outcome.0
            }
            (LIFECYCLE_FUNCTION, Input::Lifecycle { state }) => {
                self.lifecycle = Some(state);
                if matches!(state, Lifecycle::Suspended | Lifecycle::Stopped) {
                    self.resources.clear();
                    self.background_allowed = false;
                    self.audio.release();
                    self.speech.release(SpeechStopReason::LifecycleExpiry);
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
                if permission == Permission::Microphone && !granted {
                    self.speech.release(SpeechStopReason::PermissionLoss);
                }
                granted
            }
            (ACTIVATION_FUNCTION, Input::Notification { channel, .. }) => {
                if channel.trim().is_empty() {
                    return Err("Android notification channel must not be empty".into());
                }
                self.resources.insert(format!("notification:{channel}"))
            }
            (AUDIO_FUNCTION, Input::Audio { input }) => {
                audio_route = self.audio.dispatch(input)?;
                self.audio.armed()
            }
            (SPEECH_FUNCTION, Input::Speech { input }) => {
                speech = Some(self.speech.dispatch(input)?);
                speech.as_ref().is_some_and(SpeechOutput::is_available)
            }
            (ACTIVATION_FUNCTION, Input::BackgroundExecution { allowed }) => {
                self.background_allowed = allowed;
                allowed
            }
            (LIFECYCLE_FUNCTION | ACTIVATION_FUNCTION | AUDIO_FUNCTION | SPEECH_FUNCTION, _) => {
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
            snapshot_content_id: self
                .binding
                .as_ref()
                .map(|plan| plan.snapshot.content_id.clone()),
            resumed_turn_content_id,
            audio_route,
            speech,
        })
    }

    fn dispatch_continuity(&mut self, input: Input) -> Result<(bool, Option<String>), String> {
        match input {
            Input::Bind { plan } => {
                validate_bind_plan(&plan)?;
                self.binding = Some(plan);
                Ok((true, None))
            }
            Input::Event { event } => {
                self.apply_android_event(event);
                Ok((true, None))
            }
            Input::Restore { plan } => {
                validate_restore_plan(&plan)?;
                let binding = self.binding.as_ref().ok_or_else(|| {
                    "Android restore requires an atomically bound provider snapshot".to_owned()
                })?;
                if binding.snapshot.content_id != plan.snapshot_content_id {
                    return Err("Android restore snapshot does not match bound providers".into());
                }
                self.permissions = plan.permission_observations;
                self.pending_turn_content_id = plan.pending_turn_content_id;
                let resumed = self
                    .pending_turn_content_id
                    .take()
                    .filter(|id| self.resumed_turns.insert(id.clone()));
                Ok((true, resumed))
            }
            Input::SubmitTurn { content_id } => {
                validate_content_id("turn", &content_id)?;
                self.pending_turn_content_id = Some(content_id);
                Ok((true, None))
            }
            _ => Err("typed input does not match Android continuity function".into()),
        }
    }

    fn apply_android_event(&mut self, event: AndroidEvent) {
        match event {
            AndroidEvent::Rotation | AndroidEvent::ActivityRecreation => {}
            AndroidEvent::BackgroundRestriction | AndroidEvent::Suspend => {
                self.background_allowed = false;
                self.lifecycle = Some(Lifecycle::Suspended);
                self.resources.clear();
                self.audio.release();
                self.speech.release(SpeechStopReason::LifecycleExpiry);
            }
            AndroidEvent::ProcessDeath => {
                self.binding = None;
                self.resources.clear();
                self.audio.release();
                self.speech.release(SpeechStopReason::LifecycleExpiry);
                self.lifecycle = Some(Lifecycle::Stopped);
            }
            AndroidEvent::Restart => self.lifecycle = Some(Lifecycle::Created),
            AndroidEvent::MemoryPressure => {
                self.resources.clear();
                self.audio.release();
                self.speech.release(SpeechStopReason::LifecycleExpiry);
            }
        }
    }
}

fn validate_bind_plan(plan: &BindPlan) -> Result<(), String> {
    validate_content_id("provider snapshot", &plan.snapshot.content_id)?;
    if plan.app_private_mount != "app-private" {
        return Err("Android continuity root requires the app-private mount".into());
    }
    let declared: BTreeSet<_> = plan.services.keys().map(String::as_str).collect();
    let required: BTreeSet<_> = REQUIRED_SERVICES.into_iter().collect();
    if declared != required {
        return Err("Android bind must resolve every required service atomically".into());
    }
    if plan.services.values().any(|provider| {
        plan.snapshot
            .providers
            .get(provider)
            .is_none_or(|value| value != provider)
    }) {
        return Err("Android bind references a provider outside its immutable snapshot".into());
    }
    Ok(())
}

fn validate_restore_plan(plan: &RestorePlan) -> Result<(), String> {
    validate_content_id("provider snapshot", &plan.snapshot_content_id)?;
    validate_content_id("journal", &plan.journal_content_id)?;
    if let Some(id) = &plan.pending_turn_content_id {
        validate_content_id("pending turn", id)?;
    }
    Ok(())
}

fn validate_content_id(kind: &str, id: &str) -> Result<(), String> {
    if id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!("Android {kind} must be a 64-digit content id"))
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
