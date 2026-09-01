#![deny(unsafe_code)]
//! Android AOT capsule using the unchanged SIM native byte-frame ABI.

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
mod android;

pub use android::*;
