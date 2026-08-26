use serde::{Deserialize, Serialize};

/// Privacy-safe Android route classes. No device id, address, or product name crosses the shell.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioRouteClass {
    Handset,
    Wired,
    ClassicBluetooth,
    LeAudio,
    Usb,
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouteEvidence {
    InitialQuery,
    DeviceCallback,
    CommunicationCallback,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RoutingContract {
    Api28To30CommunicationMode,
    Api31To35CommunicationDevice,
}

impl RoutingContract {
    /// Selects the bounded Android routing contract for `api_level`.
    ///
    /// # Errors
    ///
    /// Returns an error outside the explicitly supported API 28 through 35
    /// range.
    pub fn for_api(api_level: u16) -> Result<Self, String> {
        match api_level {
            28..=30 => Ok(Self::Api28To30CommunicationMode),
            31..=35 => Ok(Self::Api31To35CommunicationDevice),
            _ => Err(format!("unsupported Android audio API level {api_level}")),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PcmContract {
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub frames_per_chunk: u16,
    pub queue_capacity_chunks: u16,
}

impl PcmContract {
    fn validate(&self) -> Result<(), String> {
        if !(8_000..=192_000).contains(&self.sample_rate_hz)
            || !(1..=8).contains(&self.channels)
            || !(1..=4096).contains(&self.frames_per_chunk)
            || !(1..=256).contains(&self.queue_capacity_chunks)
        {
            return Err("Android audio PCM contract exceeds canonical bounded limits".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudioSessionSpec {
    pub turn_content_id: String,
    pub api_level: u16,
    pub admitted: bool,
    pub private_output: bool,
    pub pcm: PcmContract,
    pub armed_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteObservation {
    pub capture: Vec<AudioRouteClass>,
    pub render: Vec<AudioRouteClass>,
    pub generation: u64,
    pub observed_at_ms: u64,
    pub evidence: RouteEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
// These are independent attested route conditions, not a hidden state machine.
#[allow(clippy::struct_excessive_bools)]
pub struct AudioRouteReceipt {
    pub turn_content_id: String,
    pub capture: Vec<AudioRouteClass>,
    pub render: Vec<AudioRouteClass>,
    pub duplex: bool,
    pub generation: u64,
    pub observed_at_ms: u64,
    pub fresh: bool,
    pub evidence: RouteEvidence,
    pub routing_contract: RoutingContract,
    pub focus_held: bool,
    pub communication_route_held: bool,
    pub private_output_paused: bool,
    pub pcm: PcmContract,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioStopReason {
    Stop,
    Cancellation,
    BackgroundExpiry,
    PermissionLoss,
    RouteLoss,
    FocusConflict,
    ProcessDeath,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum AudioInput {
    Arm { spec: AudioSessionSpec },
    Route { observation: RouteObservation },
    Stop { reason: AudioStopReason },
    Tick { now_ms: u64 },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AndroidAudioState {
    session: Option<AudioSessionSpec>,
    route: Option<RouteObservation>,
    focus_held: bool,
    communication_route_held: bool,
}

impl AndroidAudioState {
    pub(crate) fn dispatch(
        &mut self,
        input: AudioInput,
    ) -> Result<Option<AudioRouteReceipt>, String> {
        match input {
            AudioInput::Arm { spec } => {
                RoutingContract::for_api(spec.api_level)?;
                spec.pcm.validate()?;
                if !spec.admitted {
                    return Err("Android audio session must be admitted before it is armed".into());
                }
                if spec.turn_content_id.len() != 64
                    || !spec
                        .turn_content_id
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit())
                    || spec.expires_at_ms <= spec.armed_at_ms
                {
                    return Err("Android audio session identity or expiry is invalid".into());
                }
                self.release();
                self.session = Some(spec);
                self.focus_held = true;
                self.communication_route_held = true;
                self.receipt()
            }
            AudioInput::Route { mut observation } => {
                normalize_classes(&mut observation.capture);
                normalize_classes(&mut observation.render);
                if self
                    .route
                    .as_ref()
                    .is_some_and(|current| observation.generation <= current.generation)
                {
                    return Err("Android audio route generation must increase".into());
                }
                self.route = Some(observation);
                let route_lost = self
                    .route
                    .as_ref()
                    .is_some_and(|route| route.capture.is_empty() && route.render.is_empty());
                let mut receipt = self.receipt()?;
                if route_lost {
                    self.release();
                    if let Some(receipt) = &mut receipt {
                        receipt.focus_held = false;
                        receipt.communication_route_held = false;
                        receipt.private_output_paused =
                            receipt.private_output_paused || receipt.render.is_empty();
                    }
                }
                Ok(receipt)
            }
            AudioInput::Stop { .. } => {
                self.release();
                Ok(None)
            }
            AudioInput::Tick { now_ms } => {
                if self
                    .session
                    .as_ref()
                    .is_some_and(|session| now_ms >= session.expires_at_ms)
                {
                    self.release();
                    Ok(None)
                } else {
                    self.receipt_at(now_ms)
                }
            }
        }
    }

    pub(crate) fn release(&mut self) {
        self.session = None;
        self.focus_held = false;
        self.communication_route_held = false;
    }

    pub(crate) fn armed(&self) -> bool {
        self.session.is_some()
    }

    fn receipt(&self) -> Result<Option<AudioRouteReceipt>, String> {
        let now = self.route.as_ref().map_or(0, |route| route.observed_at_ms);
        self.receipt_at(now)
    }

    fn receipt_at(&self, now_ms: u64) -> Result<Option<AudioRouteReceipt>, String> {
        let Some(session) = &self.session else {
            return Ok(None);
        };
        let contract = RoutingContract::for_api(session.api_level)?;
        let route = self.route.clone().unwrap_or(RouteObservation {
            capture: Vec::new(),
            render: Vec::new(),
            generation: 0,
            observed_at_ms: 0,
            evidence: RouteEvidence::InitialQuery,
        });
        let fresh = route.generation > 0 && now_ms.saturating_sub(route.observed_at_ms) <= 2_000;
        let duplex = route
            .capture
            .iter()
            .any(|class| route.render.contains(class));
        let uncertain = !fresh || route.render.is_empty() || (session.private_output && !duplex);
        Ok(Some(AudioRouteReceipt {
            turn_content_id: session.turn_content_id.clone(),
            capture: route.capture,
            render: route.render,
            duplex,
            generation: route.generation,
            observed_at_ms: route.observed_at_ms,
            fresh,
            evidence: route.evidence,
            routing_contract: contract,
            focus_held: self.focus_held,
            communication_route_held: self.communication_route_held,
            private_output_paused: session.private_output && uncertain,
            pcm: session.pcm.clone(),
        }))
    }
}

fn normalize_classes(classes: &mut Vec<AudioRouteClass>) {
    classes.sort_unstable();
    classes.dedup();
}
