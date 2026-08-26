use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_PCM_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SPEAK_BYTES: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpeechKind {
    Transcribe,
    Speak,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpeechTier {
    OnDevice,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SpeechLanguage {
    pub tag: String,
    pub installed: bool,
}

/// Evidence produced by the Android shell. Optimistic configuration is insufficient.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
// These are independent evidence claims whose conjunction is validated below.
#[allow(clippy::struct_excessive_bools)]
pub struct LocalSpeechEvidence {
    pub kind: SpeechKind,
    pub implementation: String,
    pub explicitly_on_device: bool,
    pub remotely_backed: bool,
    pub discovery_contacted_provider: bool,
    pub prompted: bool,
    pub downloaded: bool,
    pub network_denied_self_test: bool,
    pub languages: Vec<SpeechLanguage>,
}

impl LocalSpeechEvidence {
    fn validate(&mut self) -> Result<(), String> {
        self.implementation = self.implementation.trim().to_owned();
        self.languages.sort_by(|a, b| a.tag.cmp(&b.tag));
        self.languages.dedup_by(|a, b| a.tag == b.tag);
        let language_tags_valid = self.languages.iter().all(|language| {
            language.installed && !language.tag.trim().is_empty() && language.tag.len() <= 64
        });
        if self.implementation.is_empty()
            || !self.explicitly_on_device
            || self.remotely_backed
            || self.discovery_contacted_provider
            || self.prompted
            || self.downloaded
            || !self.network_denied_self_test
            || !language_tags_valid
            || self.languages.is_empty()
        {
            return Err("Android local speech requires installed-language, on-device, network-denied evidence".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpeechFallback {
    VoiceNote,
    Type,
    Discard,
    PhoneScene,
    Silence,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpeechStopReason {
    Release,
    PermissionLoss,
    StaleRoute,
    LifecycleExpiry,
    OversizedInput,
    EmptyInput,
    Revocation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum SpeechInput {
    InstallEvidence { evidence: LocalSpeechEvidence },
    Transcribe { language: String, pcm: Vec<u8> },
    Transcript { language: String, text: String },
    Speak { language: String, text: String },
    Complete,
    Stop { reason: SpeechStopReason },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum SpeechOutput {
    Available {
        kind: SpeechKind,
        tier: SpeechTier,
        languages: Vec<String>,
    },
    Started {
        kind: SpeechKind,
        language: String,
    },
    Transcript {
        language: String,
        text: String,
    },
    Unsupported {
        kind: SpeechKind,
        fallback: SpeechFallback,
    },
    Stopped {
        reason: SpeechStopReason,
    },
}

impl SpeechOutput {
    pub(crate) fn is_available(&self) -> bool {
        !matches!(self, Self::Unsupported { .. } | Self::Stopped { .. })
    }
}

#[derive(Default)]
pub(crate) struct AndroidSpeechState {
    tiers: BTreeMap<SpeechKind, LocalSpeechEvidence>,
    active: Option<SpeechKind>,
}

impl AndroidSpeechState {
    pub(crate) fn dispatch(&mut self, input: SpeechInput) -> Result<SpeechOutput, String> {
        match input {
            SpeechInput::InstallEvidence { mut evidence } => {
                evidence.validate()?;
                let kind = evidence.kind;
                let languages = evidence
                    .languages
                    .iter()
                    .map(|item| item.tag.clone())
                    .collect();
                self.tiers.insert(kind, evidence);
                Ok(SpeechOutput::Available {
                    kind,
                    tier: SpeechTier::OnDevice,
                    languages,
                })
            }
            SpeechInput::Transcribe { language, pcm } => {
                if pcm.is_empty() {
                    return Ok(self.stop_for(SpeechStopReason::EmptyInput));
                }
                if pcm.len() > MAX_PCM_BYTES {
                    return Ok(self.stop_for(SpeechStopReason::OversizedInput));
                }
                Ok(self.start(SpeechKind::Transcribe, language))
            }
            SpeechInput::Speak { language, text } => {
                if text.trim().is_empty() {
                    return Ok(self.stop_for(SpeechStopReason::EmptyInput));
                }
                if text.len() > MAX_SPEAK_BYTES {
                    return Ok(self.stop_for(SpeechStopReason::OversizedInput));
                }
                Ok(self.start(SpeechKind::Speak, language))
            }
            SpeechInput::Transcript { language, text } => {
                if self.active != Some(SpeechKind::Transcribe) {
                    return Err(
                        "Android transcript arrived without an active local transcriber".into(),
                    );
                }
                if text.trim().is_empty() {
                    return Ok(self.stop_for(SpeechStopReason::EmptyInput));
                }
                self.active = None;
                Ok(SpeechOutput::Transcript { language, text })
            }
            SpeechInput::Complete => Ok(self.stop_for(SpeechStopReason::Release)),
            SpeechInput::Stop { reason } => Ok(self.stop_for(reason)),
        }
    }

    fn start(&mut self, kind: SpeechKind, language: String) -> SpeechOutput {
        let supported = self.tiers.get(&kind).is_some_and(|evidence| {
            evidence
                .languages
                .iter()
                .any(|item| item.tag == language && item.installed)
        });
        if !supported {
            return SpeechOutput::Unsupported {
                kind,
                fallback: fallback(kind),
            };
        }
        self.active = Some(kind);
        SpeechOutput::Started { kind, language }
    }

    fn stop_for(&mut self, reason: SpeechStopReason) -> SpeechOutput {
        self.active = None;
        SpeechOutput::Stopped { reason }
    }

    pub(crate) fn release(&mut self, reason: SpeechStopReason) {
        self.active = None;
        let _ = reason;
    }
}

fn fallback(kind: SpeechKind) -> SpeechFallback {
    match kind {
        SpeechKind::Transcribe => SpeechFallback::VoiceNote,
        SpeechKind::Speak => SpeechFallback::Silence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(kind: SpeechKind) -> LocalSpeechEvidence {
        LocalSpeechEvidence {
            kind,
            implementation: "android-explicit-on-device".into(),
            explicitly_on_device: true,
            remotely_backed: false,
            discovery_contacted_provider: false,
            prompted: false,
            downloaded: false,
            network_denied_self_test: true,
            languages: vec![SpeechLanguage {
                tag: "sv-SE".into(),
                installed: true,
            }],
        }
    }

    #[test]
    fn every_advertised_tier_requires_network_denied_on_device_proof() {
        for kind in [SpeechKind::Transcribe, SpeechKind::Speak] {
            let mut state = AndroidSpeechState::default();
            let output = state
                .dispatch(SpeechInput::InstallEvidence {
                    evidence: evidence(kind),
                })
                .unwrap();
            assert!(matches!(
                output,
                SpeechOutput::Available {
                    tier: SpeechTier::OnDevice,
                    ..
                }
            ));

            for invalidate in [
                |item: &mut LocalSpeechEvidence| item.network_denied_self_test = false,
                |item: &mut LocalSpeechEvidence| item.explicitly_on_device = false,
                |item: &mut LocalSpeechEvidence| item.discovery_contacted_provider = true,
                |item: &mut LocalSpeechEvidence| item.downloaded = true,
                |item: &mut LocalSpeechEvidence| item.prompted = true,
            ] {
                let mut rejected = evidence(kind);
                invalidate(&mut rejected);
                assert!(
                    state
                        .dispatch(SpeechInput::InstallEvidence { evidence: rejected })
                        .is_err()
                );
            }
        }
    }

    #[test]
    fn remotely_backed_recognizer_never_satisfies_local_speech() {
        let mut remote = evidence(SpeechKind::Transcribe);
        remote.implementation = "android-system-default-recognizer".into();
        remote.remotely_backed = true;
        let mut state = AndroidSpeechState::default();
        assert!(
            state
                .dispatch(SpeechInput::InstallEvidence { evidence: remote })
                .is_err()
        );
        assert_eq!(
            state
                .dispatch(SpeechInput::Transcribe {
                    language: "sv-SE".into(),
                    pcm: vec![1, 2],
                })
                .unwrap(),
            SpeechOutput::Unsupported {
                kind: SpeechKind::Transcribe,
                fallback: SpeechFallback::VoiceNote,
            }
        );
    }

    #[test]
    fn pcm_and_plain_transcript_are_preserved_and_absence_has_fallbacks() {
        let mut state = AndroidSpeechState::default();
        state
            .dispatch(SpeechInput::InstallEvidence {
                evidence: evidence(SpeechKind::Transcribe),
            })
            .unwrap();
        assert!(matches!(
            state
                .dispatch(SpeechInput::Transcribe {
                    language: "sv-SE".into(),
                    pcm: vec![0, 1, 2, 3]
                })
                .unwrap(),
            SpeechOutput::Started {
                kind: SpeechKind::Transcribe,
                ..
            }
        ));
        assert_eq!(
            state
                .dispatch(SpeechInput::Transcript {
                    language: "sv-SE".into(),
                    text: "granskad text".into()
                })
                .unwrap(),
            SpeechOutput::Transcript {
                language: "sv-SE".into(),
                text: "granskad text".into()
            }
        );
        assert_eq!(
            state
                .dispatch(SpeechInput::Speak {
                    language: "sv-SE".into(),
                    text: "hej".into()
                })
                .unwrap(),
            SpeechOutput::Unsupported {
                kind: SpeechKind::Speak,
                fallback: SpeechFallback::Silence
            }
        );
    }

    #[test]
    fn all_stop_causes_reap_and_bounds_fail_closed() {
        let mut state = AndroidSpeechState::default();
        state
            .dispatch(SpeechInput::InstallEvidence {
                evidence: evidence(SpeechKind::Transcribe),
            })
            .unwrap();
        for reason in [
            SpeechStopReason::Release,
            SpeechStopReason::PermissionLoss,
            SpeechStopReason::StaleRoute,
            SpeechStopReason::LifecycleExpiry,
            SpeechStopReason::Revocation,
        ] {
            state
                .dispatch(SpeechInput::Transcribe {
                    language: "sv-SE".into(),
                    pcm: vec![1],
                })
                .unwrap();
            assert_eq!(
                state.dispatch(SpeechInput::Stop { reason }).unwrap(),
                SpeechOutput::Stopped { reason }
            );
        }
        assert_eq!(
            state
                .dispatch(SpeechInput::Transcribe {
                    language: "sv-SE".into(),
                    pcm: Vec::new()
                })
                .unwrap(),
            SpeechOutput::Stopped {
                reason: SpeechStopReason::EmptyInput
            }
        );
        assert_eq!(
            state
                .dispatch(SpeechInput::Transcribe {
                    language: "sv-SE".into(),
                    pcm: vec![0; MAX_PCM_BYTES + 1]
                })
                .unwrap(),
            SpeechOutput::Stopped {
                reason: SpeechStopReason::OversizedInput
            }
        );
    }
}
