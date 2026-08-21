#![forbid(unsafe_code)]
//! Fictional records only; this crate does not represent a real provider.

use sim_platform_core::{
    BundleManifest, CapsuleManifest, ValidationError, parse_bundle, parse_capsule,
};

pub const FICTIONAL_CAPSULE: &str = r#"
schema = "sim.platform-capsule/v1"
provider = "platform/site/fictional"
services = ["service/fictional-clock"]
shells = ["shell/fictional"]
"#;

pub const FICTIONAL_BUNDLE: &str = r#"
schema = "sim.platform-bundle/v1"
capsule = "platform/site/fictional"
artifact = "lib/sim-platform-fictional"
artifact_content = "sha256:fictional-not-an-artifact"
entry = "sim_native_abi_v1"
shell = "shell/fictional"
"#;

/// Parse both fictional records.
///
/// # Errors
/// Returns a parse error if the committed specimen stops matching its contract.
pub fn fictional_records() -> Result<(CapsuleManifest, BundleManifest), ValidationError> {
    Ok((
        parse_capsule(FICTIONAL_CAPSULE)?,
        parse_bundle(FICTIONAL_BUNDLE)?,
    ))
}
