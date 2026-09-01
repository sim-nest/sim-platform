use std::collections::{BTreeSet, VecDeque};

/// Application protocol version carried through the official Halo SDK link.
pub const PROTOCOL_VERSION: u8 = 1;
/// Header bytes before a fragment payload.
pub const HEADER_BYTES: usize = 18;
/// Largest application message accepted after reassembly.
pub const MAX_MESSAGE_BYTES: usize = 2048;
/// Largest official-link payload used by the capsule.
pub const MAX_FRAGMENT_BYTES: usize = 180;
/// Maximum accepted fragments in one message.
pub const MAX_FRAGMENTS: usize = 16;
/// Maximum queued normalized input events.
pub const MAX_PENDING_EVENTS: usize = 32;
/// `GLASSES_8` profile consumed by the host adapter.
pub const DEVICE_PROFILE: &str = "GLASSES_8";
/// Existing output protocol consumed by the host adapter.
pub const OUTPUT_PROTOCOL: &str = "Surface";

const MAGIC: [u8; 2] = *b"SH";

/// Officially documented host-library route. These differ only at the SDK edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostTransport {
    Android,
    Ios,
    WebBluetooth,
}

/// Lifecycle observed by the host capsule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lifecycle {
    Created,
    Active,
    Suspended,
    Destroyed,
}

/// Canonical proxy message class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MessageKind {
    Button = 1,
    Sensor = 2,
    LinkState = 3,
    Display = 16,
    Clear = 17,
}

impl TryFrom<u8> for MessageKind {
    type Error = ProxyError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Button),
            2 => Ok(Self::Sensor),
            3 => Ok(Self::LinkState),
            16 => Ok(Self::Display),
            17 => Ok(Self::Clear),
            _ => Err(ProxyError::Malformed),
        }
    }
}

/// Input normalized for the delivered `GLASSES_8` Device/Stream adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GlassesInput {
    Button { name: String, pressed: bool },
    Sensor { name: String, value_milli: i32 },
    LinkState { connected: bool },
}

/// Output accepted from the delivered Surface adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SurfaceCommand {
    Display { cells: Vec<u8> },
    Clear,
}

/// One complete canonical application message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Message {
    pub kind: MessageKind,
    pub session: u32,
    pub sequence: u32,
    pub payload: Vec<u8>,
}

/// Typed refusal at the capsule boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProxyError {
    Malformed,
    Oversized,
    WrongVersion,
    StaleSession,
    Replay,
    Inactive,
    ConsentRequired,
    QueueFull,
    FragmentOrder,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Assembly {
    kind: MessageKind,
    session: u32,
    sequence: u32,
    count: u8,
    next: u8,
    total: usize,
    payload: Vec<u8>,
}

/// Companion-owned proxy state. No host probing or fallback occurs here.
pub struct HaloCapsule {
    transport: HostTransport,
    lifecycle: Lifecycle,
    consent: bool,
    connected: bool,
    session: u32,
    seen: BTreeSet<u32>,
    assembly: Option<Assembly>,
    pending: VecDeque<GlassesInput>,
}

impl HaloCapsule {
    /// Creates a capsule for one explicitly selected official host route.
    #[must_use]
    pub fn new(transport: HostTransport) -> Self {
        Self {
            transport,
            lifecycle: Lifecycle::Created,
            consent: false,
            connected: false,
            session: 0,
            seen: BTreeSet::new(),
            assembly: None,
            pending: VecDeque::new(),
        }
    }

    /// Returns the explicitly selected official host route.
    #[must_use]
    pub fn transport(&self) -> HostTransport {
        self.transport
    }

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
    pub fn connect(&mut self, session: u32) -> Result<(), ProxyError> {
        if !self.consent {
            return Err(ProxyError::ConsentRequired);
        }
        if self.lifecycle != Lifecycle::Active {
            return Err(ProxyError::Inactive);
        }
        self.connected = true;
        self.session = session;
        self.seen.clear();
        self.assembly = None;
        self.pending.clear();
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.connected = false;
        self.seen.clear();
        self.assembly = None;
        self.pending.clear();
    }

    /// Accepts one ordered official-link notification fragment.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal for malformed, oversized, stale, replayed,
    /// unauthorized, inactive, out-of-order, or over-capacity input.
    pub fn receive_fragment(&mut self, bytes: &[u8]) -> Result<bool, ProxyError> {
        self.require_active()?;
        let fragment = decode_fragment(bytes)?;
        if fragment.session != self.session {
            return Err(ProxyError::StaleSession);
        }
        if self.seen.contains(&fragment.sequence) {
            return Err(ProxyError::Replay);
        }
        if fragment.index == 0 {
            self.assembly = Some(Assembly {
                kind: fragment.kind,
                session: fragment.session,
                sequence: fragment.sequence,
                count: fragment.count,
                next: 0,
                total: fragment.total,
                payload: Vec::new(),
            });
        }
        let assembly = self.assembly.as_mut().ok_or(ProxyError::FragmentOrder)?;
        if assembly.kind != fragment.kind
            || assembly.session != fragment.session
            || assembly.sequence != fragment.sequence
            || assembly.count != fragment.count
            || assembly.total != fragment.total
            || assembly.next != fragment.index
        {
            return Err(ProxyError::FragmentOrder);
        }
        if assembly.payload.len() + fragment.payload.len() > MAX_MESSAGE_BYTES {
            self.assembly = None;
            return Err(ProxyError::Oversized);
        }
        assembly.payload.extend_from_slice(fragment.payload);
        assembly.next += 1;
        if assembly.next != assembly.count {
            return Ok(false);
        }
        let Some(message) = self.assembly.take() else {
            return Err(ProxyError::FragmentOrder);
        };
        if message.payload.len() != message.total {
            return Err(ProxyError::Malformed);
        }
        if self.pending.len() == MAX_PENDING_EVENTS {
            return Err(ProxyError::QueueFull);
        }
        let input = decode_input(message.kind, &message.payload)?;
        self.seen.insert(message.sequence);
        self.pending.push_back(input);
        Ok(true)
    }

    pub fn next_input(&mut self) -> Option<GlassesInput> {
        self.pending.pop_front()
    }

    /// Encodes one Surface command into canonical fragments for any host SDK.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal when the capsule is unauthorized or inactive,
    /// or when the encoded command exceeds protocol bounds.
    pub fn encode_surface(
        &self,
        sequence: u32,
        command: &SurfaceCommand,
    ) -> Result<Vec<Vec<u8>>, ProxyError> {
        self.require_active()?;
        let (kind, payload) = encode_surface_payload(command)?;
        encode_message(&Message {
            kind,
            session: self.session,
            sequence,
            payload,
        })
    }

    fn require_active(&self) -> Result<(), ProxyError> {
        if !self.consent {
            Err(ProxyError::ConsentRequired)
        } else if !self.connected || self.lifecycle != Lifecycle::Active {
            Err(ProxyError::Inactive)
        } else {
            Ok(())
        }
    }
}

struct Fragment<'a> {
    kind: MessageKind,
    session: u32,
    sequence: u32,
    index: u8,
    count: u8,
    total: usize,
    payload: &'a [u8],
}

/// Canonical BLE application codec shared by Android, iOS, and Web Bluetooth.
///
/// # Errors
///
/// Returns [`ProxyError::Oversized`] when the message or resulting fragment
/// count exceeds the bounded wire format.
pub fn encode_message(message: &Message) -> Result<Vec<Vec<u8>>, ProxyError> {
    if message.payload.len() > MAX_MESSAGE_BYTES {
        return Err(ProxyError::Oversized);
    }
    let count = message.payload.len().max(1).div_ceil(MAX_FRAGMENT_BYTES);
    if count > MAX_FRAGMENTS {
        return Err(ProxyError::Oversized);
    }
    let total = u16::try_from(message.payload.len()).map_err(|_| ProxyError::Oversized)?;
    let mut frames = Vec::with_capacity(count);
    for index in 0..count {
        let start = index * MAX_FRAGMENT_BYTES;
        let end = (start + MAX_FRAGMENT_BYTES).min(message.payload.len());
        let mut frame = Vec::with_capacity(HEADER_BYTES + end.saturating_sub(start));
        frame.extend_from_slice(&MAGIC);
        frame.push(PROTOCOL_VERSION);
        frame.push(message.kind as u8);
        frame.extend_from_slice(&message.session.to_be_bytes());
        frame.extend_from_slice(&message.sequence.to_be_bytes());
        frame.extend_from_slice(&total.to_be_bytes());
        frame.push(u8::try_from(index).map_err(|_| ProxyError::Oversized)?);
        frame.push(u8::try_from(count).map_err(|_| ProxyError::Oversized)?);
        let payload = &message.payload[start..end];
        frame.extend_from_slice(
            &(u16::try_from(payload.len()).map_err(|_| ProxyError::Oversized)?).to_be_bytes(),
        );
        frame.extend_from_slice(payload);
        frames.push(frame);
    }
    Ok(frames)
}

fn decode_fragment(bytes: &[u8]) -> Result<Fragment<'_>, ProxyError> {
    if bytes.len() < HEADER_BYTES
        || bytes.len() > HEADER_BYTES + MAX_FRAGMENT_BYTES
        || bytes[..2] != MAGIC
    {
        return Err(ProxyError::Malformed);
    }
    if bytes[2] != PROTOCOL_VERSION {
        return Err(ProxyError::WrongVersion);
    }
    let kind = MessageKind::try_from(bytes[3])?;
    let session = u32::from_be_bytes(bytes[4..8].try_into().map_err(|_| ProxyError::Malformed)?);
    let sequence = u32::from_be_bytes(bytes[8..12].try_into().map_err(|_| ProxyError::Malformed)?);
    let total = usize::from(u16::from_be_bytes(
        bytes[12..14]
            .try_into()
            .map_err(|_| ProxyError::Malformed)?,
    ));
    let index = bytes[14];
    let count = bytes[15];
    let length = usize::from(u16::from_be_bytes(
        bytes[16..18]
            .try_into()
            .map_err(|_| ProxyError::Malformed)?,
    ));
    if count == 0
        || usize::from(count) > MAX_FRAGMENTS
        || index >= count
        || total > MAX_MESSAGE_BYTES
        || length != bytes.len() - HEADER_BYTES
    {
        return Err(ProxyError::Malformed);
    }
    Ok(Fragment {
        kind,
        session,
        sequence,
        index,
        count,
        total,
        payload: &bytes[HEADER_BYTES..],
    })
}

fn short(bytes: &[u8], max: usize) -> Result<String, ProxyError> {
    if bytes.is_empty() || bytes.len() > max {
        return Err(ProxyError::Malformed);
    }
    let value = std::str::from_utf8(bytes).map_err(|_| ProxyError::Malformed)?;
    if value.chars().any(char::is_control) {
        return Err(ProxyError::Malformed);
    }
    Ok(value.to_owned())
}

fn decode_input(kind: MessageKind, payload: &[u8]) -> Result<GlassesInput, ProxyError> {
    match kind {
        MessageKind::Button if payload.len() >= 2 => Ok(GlassesInput::Button {
            name: short(&payload[1..], 32)?,
            pressed: payload[0] != 0,
        }),
        MessageKind::Sensor if payload.len() >= 5 => Ok(GlassesInput::Sensor {
            name: short(&payload[4..], 48)?,
            value_milli: i32::from_be_bytes(
                payload[..4].try_into().map_err(|_| ProxyError::Malformed)?,
            ),
        }),
        MessageKind::LinkState if payload.len() == 1 => Ok(GlassesInput::LinkState {
            connected: payload[0] != 0,
        }),
        _ => Err(ProxyError::Malformed),
    }
}

fn encode_surface_payload(command: &SurfaceCommand) -> Result<(MessageKind, Vec<u8>), ProxyError> {
    match command {
        SurfaceCommand::Display { cells } if cells.len() <= 1024 => {
            Ok((MessageKind::Display, cells.clone()))
        }
        SurfaceCommand::Display { .. } => Err(ProxyError::Oversized),
        SurfaceCommand::Clear => Ok((MessageKind::Clear, Vec::new())),
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
