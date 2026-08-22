//! Browser/Wasm platform capsule.
//!
//! The capsule owns browser handle realization and lifecycle events. It does
//! not contain a DOM, canvas, WebGPU, render-tree, process, native-loader, or
//! ambient-filesystem abstraction; presentation remains in `sim-web`.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use sim_kernel::Expr;
use std::collections::{BTreeMap, BTreeSet};

/// The only semantic Wasm export. Allocation helpers are memory mechanics.
pub const NAMED_CALL_EXPORT: &str = "sim_browser_named_call";
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_HOST_PAYLOAD_BYTES: usize = 512 * 1024;
pub const MAX_HANDLES: usize = 256;

const CARD_FUNCTION: &str = "platform/card";
const HOST_FUNCTION: &str = "platform/host-call";
const COMPLETE_FUNCTION: &str = "platform/host-complete";

/// Browser APIs detected by the JavaScript shell before capsule creation.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[allow(clippy::struct_excessive_bools)] // This is a serialized API-availability bitmap, not mutable state.
pub struct BrowserApis {
    pub storage: bool,
    pub fetch: bool,
    pub websocket: bool,
    pub clipboard: bool,
    pub notification: bool,
    pub permissions: bool,
    pub wake_lock: bool,
}

/// Capability-honest, data-only browser service Card.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrowserCard {
    pub schema: String,
    pub site: String,
    pub services: Vec<String>,
    pub omitted_services: Vec<String>,
    pub presentation_owner: String,
}

impl BrowserCard {
    #[must_use]
    pub fn from_apis(apis: &BrowserApis) -> Self {
        let mut services = vec!["platform/lifecycle".into(), "platform/activation".into()];
        for (available, service) in [
            (apis.storage, "storage/handle"),
            (apis.fetch, "transport/fetch"),
            (apis.websocket, "transport/websocket"),
            (apis.clipboard, "platform/clipboard"),
            (apis.notification, "platform/notification"),
            (apis.permissions, "platform/permission"),
            (apis.wake_lock, "platform/wake-lock"),
        ] {
            if available {
                services.push(service.into());
            }
        }
        Self {
            schema: "sim.platform-browser-card/v1".into(),
            site: "platform/browser".into(),
            services,
            omitted_services: vec![
                "platform/process".into(),
                "loader/native-v1".into(),
                "storage/ambient-filesystem".into(),
            ],
            presentation_owner: "sim-web".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Lifecycle {
    #[default]
    Created,
    Active,
    Hidden,
    Suspended,
    Stopped,
}

/// Typed operations forwarded through domain-port-shaped identities.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum HostOperation {
    Activation {
        name: String,
        payload: Vec<u8>,
    },
    StorageOpen {
        namespace: String,
    },
    StorageRead {
        handle: u32,
        key: String,
    },
    StorageWrite {
        handle: u32,
        key: String,
        bytes: Vec<u8>,
    },
    StorageClose {
        handle: u32,
    },
    Fetch {
        url: String,
        method: String,
        body: Vec<u8>,
    },
    WebsocketOpen {
        url: String,
    },
    WebsocketSend {
        handle: u32,
        bytes: Vec<u8>,
    },
    WebsocketClose {
        handle: u32,
    },
    ClipboardRead,
    ClipboardWrite {
        text: String,
    },
    Notification {
        title: String,
        body: String,
    },
    Permission {
        name: String,
        request: bool,
    },
    WakeLock {
        acquire: bool,
    },
}

impl HostOperation {
    fn service(&self) -> &'static str {
        match self {
            Self::Activation { .. } => "platform/activation",
            Self::StorageOpen { .. }
            | Self::StorageRead { .. }
            | Self::StorageWrite { .. }
            | Self::StorageClose { .. } => "storage/handle",
            Self::Fetch { .. } => "transport/fetch",
            Self::WebsocketOpen { .. }
            | Self::WebsocketSend { .. }
            | Self::WebsocketClose { .. } => "transport/websocket",
            Self::ClipboardRead | Self::ClipboardWrite { .. } => "platform/clipboard",
            Self::Notification { .. } => "platform/notification",
            Self::Permission { .. } => "platform/permission",
            Self::WakeLock { .. } => "platform/wake-lock",
        }
    }

    fn validate(&self) -> Result<(), String> {
        let bytes = match self {
            Self::Activation { name, payload } => {
                bounded_text(name, "activation")?;
                payload.len()
            }
            Self::StorageOpen { namespace } => {
                bounded_text(namespace, "storage namespace")?;
                0
            }
            Self::StorageRead { key, .. } => {
                bounded_text(key, "storage key")?;
                0
            }
            Self::StorageWrite { key, bytes, .. } => {
                bounded_text(key, "storage key")?;
                bytes.len()
            }
            Self::Fetch { url, method, body } => {
                bounded_url(url)?;
                bounded_text(method, "fetch method")?;
                body.len()
            }
            Self::WebsocketOpen { url } => {
                bounded_url(url)?;
                0
            }
            Self::WebsocketSend { bytes, .. } => bytes.len(),
            Self::StorageClose { .. }
            | Self::WebsocketClose { .. }
            | Self::ClipboardRead
            | Self::WakeLock { .. } => 0,
            Self::ClipboardWrite { text } => text.len(),
            Self::Notification { title, body } => title.len().saturating_add(body.len()),
            Self::Permission { name, .. } => {
                bounded_text(name, "permission")?;
                0
            }
        };
        (bytes <= MAX_HOST_PAYLOAD_BYTES)
            .then_some(())
            .ok_or_else(|| "browser host payload exceeds the capsule limit".into())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Input {
    Card {
        apis: BrowserApis,
    },
    Lifecycle {
        state: Lifecycle,
    },
    HostCall {
        operation: HostOperation,
    },
    HostComplete {
        call_id: u64,
        ok: bool,
        payload: Vec<u8>,
        handle: Option<u32>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Output {
    Card {
        card: BrowserCard,
    },
    Lifecycle {
        state: Lifecycle,
    },
    HostCall {
        call_id: u64,
        service: String,
        operation: HostOperation,
    },
    Completed {
        call_id: u64,
        ok: bool,
        payload: Vec<u8>,
        handle: Option<u32>,
    },
}

pub struct Capsule {
    card: BrowserCard,
    lifecycle: Lifecycle,
    next_call: u64,
    pending: BTreeMap<u64, PendingCall>,
    handles: BTreeSet<u32>,
}

struct PendingCall {
    service: &'static str,
    close_handle: Option<u32>,
}

impl Capsule {
    #[must_use]
    pub fn new(apis: &BrowserApis) -> Self {
        Self {
            card: BrowserCard::from_apis(apis),
            lifecycle: Lifecycle::Created,
            next_call: 1,
            pending: BTreeMap::new(),
            handles: BTreeSet::new(),
        }
    }

    /// Calls the capsule through the same canonical binary frame used by other capsules.
    ///
    /// # Errors
    /// Refuses oversized or malformed frames, unknown names, unavailable
    /// services, invalid handles, and payloads outside the declared bounds.
    pub fn call_frame(&mut self, function: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
        if bytes.len() > MAX_FRAME_BYTES {
            return Err("browser input frame exceeds the capsule limit".into());
        }
        let input: Input = decode_frame(bytes)?;
        let output = self.dispatch(function, input)?;
        encode_frame(&output)
    }

    /// Dispatches a decoded request without crossing the Wasm memory membrane.
    ///
    /// # Errors
    /// Refuses mismatched input kinds and every condition documented by
    /// [`Self::call_frame`].
    pub fn dispatch(&mut self, function: &str, input: Input) -> Result<Output, String> {
        match (function, input) {
            (CARD_FUNCTION, Input::Card { apis }) => {
                self.card = BrowserCard::from_apis(&apis);
                Ok(Output::Card {
                    card: self.card.clone(),
                })
            }
            ("platform/lifecycle", Input::Lifecycle { state }) => {
                self.lifecycle = state;
                if matches!(state, Lifecycle::Suspended | Lifecycle::Stopped) {
                    self.pending.clear();
                    self.handles.clear();
                }
                Ok(Output::Lifecycle { state })
            }
            (HOST_FUNCTION, Input::HostCall { operation }) => {
                if matches!(self.lifecycle, Lifecycle::Suspended | Lifecycle::Stopped) {
                    return Err("browser capsule is not active".into());
                }
                operation.validate()?;
                let service = operation.service();
                if !self.card.services.iter().any(|item| item == service) {
                    return Err(format!("unsupported browser service: {service}"));
                }
                if self.pending.len() >= MAX_HANDLES {
                    return Err("browser pending-call limit reached".into());
                }
                for handle in operation_handles(&operation) {
                    if !self.handles.contains(&handle) {
                        return Err("unknown or closed browser handle".into());
                    }
                }
                let call_id = self.next_call;
                self.next_call = self
                    .next_call
                    .checked_add(1)
                    .ok_or("browser call id exhausted")?;
                let close_handle = match &operation {
                    HostOperation::StorageClose { handle }
                    | HostOperation::WebsocketClose { handle } => Some(*handle),
                    _ => None,
                };
                self.pending.insert(
                    call_id,
                    PendingCall {
                        service,
                        close_handle,
                    },
                );
                Ok(Output::HostCall {
                    call_id,
                    service: service.into(),
                    operation,
                })
            }
            (
                COMPLETE_FUNCTION,
                Input::HostComplete {
                    call_id,
                    ok,
                    payload,
                    handle,
                },
            ) => {
                if payload.len() > MAX_HOST_PAYLOAD_BYTES {
                    return Err("browser completion payload exceeds the capsule limit".into());
                }
                let pending = self
                    .pending
                    .remove(&call_id)
                    .ok_or("unknown or already-completed browser call")?;
                if ok && let Some(closed) = pending.close_handle {
                    self.handles.remove(&closed);
                }
                if let Some(value) = handle {
                    if self.handles.len() >= MAX_HANDLES {
                        return Err("browser handle limit reached".into());
                    }
                    if ok && matches!(pending.service, "storage/handle" | "transport/websocket") {
                        self.handles.insert(value);
                    }
                }
                Ok(Output::Completed {
                    call_id,
                    ok,
                    payload,
                    handle,
                })
            }
            (CARD_FUNCTION | "platform/lifecycle" | HOST_FUNCTION | COMPLETE_FUNCTION, _) => Err(
                format!("typed input does not match browser function {function}"),
            ),
            _ => Err(format!("unknown browser platform function: {function}")),
        }
    }
}

impl Default for Capsule {
    fn default() -> Self {
        Self::new(&BrowserApis::default())
    }
}

fn operation_handles(operation: &HostOperation) -> Vec<u32> {
    match operation {
        HostOperation::StorageRead { handle, .. }
        | HostOperation::StorageWrite { handle, .. }
        | HostOperation::StorageClose { handle }
        | HostOperation::WebsocketSend { handle, .. }
        | HostOperation::WebsocketClose { handle } => vec![*handle],
        _ => Vec::new(),
    }
}

fn bounded_text(value: &str, label: &str) -> Result<(), String> {
    (!value.trim().is_empty() && value.len() <= 4096)
        .then_some(())
        .ok_or_else(|| format!("invalid bounded browser {label}"))
}

fn bounded_url(value: &str) -> Result<(), String> {
    bounded_text(value, "URL")?;
    (value.starts_with("https://") || value.starts_with("wss://"))
        .then_some(())
        .ok_or_else(|| "browser transport URL must use https or wss".into())
}

/// Encodes a browser request in the canonical SIM binary frame codec.
///
/// # Errors
/// Returns an error if typed JSON or canonical frame encoding fails.
pub fn encode_input_frame(input: &Input) -> Result<Vec<u8>, String> {
    encode_frame(input)
}
/// Decodes a browser response from the canonical SIM binary frame codec.
///
/// # Errors
/// Returns an error for malformed frames or invalid typed responses.
pub fn decode_output_frame(bytes: &[u8]) -> Result<Output, String> {
    decode_frame(bytes)
}

fn encode_frame(value: &impl Serialize) -> Result<Vec<u8>, String> {
    let json = serde_json::to_string(value).map_err(|error| error.to_string())?;
    sim_codec_binary::encode_frame(&Expr::String(json))
        .map(|frame| frame.0)
        .map_err(|error| error.to_string())
}

fn decode_frame<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, String> {
    let (_, expr) = sim_codec_binary::decode_frame(sim_kernel::CodecId(0), bytes)
        .map_err(|error| format!("invalid SIM binary frame: {error}"))?;
    let Expr::String(json) = expr else {
        return Err("browser ABI frame payload must be one typed JSON string".into());
    };
    serde_json::from_str(&json).map_err(|error| format!("invalid typed browser frame: {error}"))
}

#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(test)]
mod tests;
