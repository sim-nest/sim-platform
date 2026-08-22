#![forbid(unsafe_code)]
//! Ubuntu PC capsule registration and sanitized evidence contracts.
use serde::{Deserialize, Serialize};
use sim_platform_core::{
    ContractProvenance, EvidenceLevel, FactPort, OpenSymbol, PlatformCard, ServiceOffer,
    stable_digest,
};
pub use sim_platform_core::{
    UbuntuArchitecture as Architecture, UbuntuProfile as UbuntuPcProfile,
    UbuntuProfileKind as ProfileKind,
};
use std::{ffi::OsString, path::PathBuf};

pub use sim_platform_linux as linux;
mod compute;
mod loader;
pub use compute::UbuntuComputeProbe;
pub use loader::UbuntuLoaderPort;
mod process;
pub use process::UbuntuProcess;

/// Owned process-entry facts captured by the Ubuntu capsule.
///
/// Portable bootloaders consume this value and never inspect argv, the current
/// directory, or environment variables themselves.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UbuntuProcessEnvelope {
    /// Complete process argument vector.
    pub argv: Vec<OsString>,
    /// Explicit working root supplied to configuration and cache policy.
    pub work_root: PathBuf,
    /// Explicit cache root override, when supplied by the host.
    pub cache_root: Option<PathBuf>,
    /// Explicit registry artifact endpoint, when supplied by the host.
    pub registry_endpoint: Option<String>,
    /// Whether the host admits unauthenticated non-loopback registry access.
    pub allow_insecure_registry: bool,
}

impl UbuntuProcessEnvelope {
    /// Captures the bounded process facts owned by this concrete capsule.
    ///
    /// # Errors
    /// Returns an error when the host does not expose a current working root.
    pub fn capture() -> std::io::Result<Self> {
        Ok(Self {
            argv: std::env::args_os().collect(),
            work_root: std::env::current_dir()?,
            cache_root: std::env::var_os("SIM_CLI_CACHE_DIR").map(PathBuf::from),
            registry_endpoint: std::env::var_os("SIM_GIT_REGISTRY_ENDPOINT")
                .map(|value| value.to_string_lossy().into_owned()),
            allow_insecure_registry: std::env::var_os("SIM_GIT_REGISTRY_ALLOW_INSECURE")
                .is_some_and(|value| !value.is_empty()),
        })
    }
}
pub const BUNDLE_DESCRIPTOR: &str = "sim.platform-bundle.toml";
/// Ubuntu PC distribution bundle. The one row admits only this capsule.
pub const UBUNTU_BUNDLE_MANIFEST: &str = r#"
schema = "sim.platform-bundle/v1"
capsule = "platform/site/ubuntu-pc"
artifact = "sim-platform-ubuntu-pc.so"
loader = "loader/native-v1"
artifact_content = "sha256:ubuntu-pc-release-content"
entry = "sim_native_abi_v1"
"#;

/// Ubuntu PC capsule card paired with [`UBUNTU_BUNDLE_MANIFEST`].
pub const UBUNTU_CAPSULE_MANIFEST: &str = r#"
schema = "sim.platform-capsule/v1"
provider = "platform/site/ubuntu-pc"
services = ["platform/monotonic", "platform/wall-clock", "platform/entropy"]
shells = []
loader_kinds = ["loader/native-v1", "loader/wasm-v1", "loader/source-v1", "loader/static-v1"]
"#;
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct RegisteredCard {
    pub profile: UbuntuPcProfile,
    pub card: PlatformCard,
}

const FACTS: &[(&str, FactPort)] = &[
    ("wall-clock", FactPort::WallClock),
    ("monotonic-clock", FactPort::MonotonicClock),
    ("timer", FactPort::Timer),
    ("entropy", FactPort::Entropy),
    ("locale", FactPort::Locale),
    ("timezone", FactPort::Timezone),
    ("pressure", FactPort::LifecyclePressure),
    ("limits", FactPort::MachineLimits),
];
const DESKTOP: &[&str] = &[
    "open",
    "share",
    "notify",
    "clipboard",
    "permission",
    "keep-awake",
    "activation",
];
/// Construct the exact Card for a supported Ubuntu profile.
///
/// # Panics
/// Panics only if serialization of the closed profile enum fails.
#[must_use]
pub fn register(profile: UbuntuPcProfile) -> RegisteredCard {
    let kind = profile.kind;
    let mut services = FACTS
        .iter()
        .map(|(name, port)| ServiceOffer {
            service: OpenSymbol(format!("platform/{name}")),
            port: *port,
            evidence: EvidenceLevel::Attested,
        })
        .collect::<Vec<_>>();
    services.extend(
        ["xdg-config", "xdg-cache", "xdg-data", "xdg-state", "temp"]
            .into_iter()
            .map(|name| ServiceOffer {
                service: OpenSymbol(format!("platform/mount/{name}")),
                port: FactPort::MachineLimits,
                evidence: EvidenceLevel::Attested,
            }),
    );
    if kind == ProfileKind::Desktop {
        services.extend(DESKTOP.iter().map(|name| ServiceOffer {
            service: OpenSymbol(format!("platform/{name}")),
            port: FactPort::LifecyclePressure,
            evidence: EvidenceLevel::Attested,
        }));
    }
    let profile_bytes = serde_json::to_vec(&profile).expect("closed profile serializes");
    RegisteredCard {
        profile,
        card: PlatformCard {
            schema: OpenSymbol("platform/card-v1".into()),
            site: OpenSymbol(
                match kind {
                    ProfileKind::Desktop => "platform/site/ubuntu-pc-desktop",
                    ProfileKind::Headless => "platform/site/ubuntu-pc-headless",
                }
                .into(),
            ),
            services,
            provenance: ContractProvenance {
                contract: OpenSymbol("contract/ubuntu-pc-v1".into()),
                content_digest: stable_digest(&profile_bytes),
                issuer: OpenSymbol("issuer/sim-platform".into()),
            },
        },
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct PhysicalAttestation {
    pub schema: String,
    pub provider: String,
    pub registered_capability: String,
    pub source_content: String,
    pub artifact_content: String,
    pub card_content: String,
    pub result_content: String,
    pub source: String,
    pub artifact: String,
    pub card: String,
    pub result: String,
    pub checks: Vec<String>,
}
impl PhysicalAttestation {
    /// Verify the sanitized, content-bound offline evidence envelope.
    ///
    /// # Errors
    /// Refuses wrong identity, missing content identities, or leaking checks.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != "sim.platform-physical-attestation/v1"
            || self.provider != "ubuntu-pc"
            || self.registered_capability != "linux-x86_64"
        {
            return Err("wrong attestation identity");
        }
        for (claimed, payload) in [
            (&self.source_content, &self.source),
            (&self.artifact_content, &self.artifact),
            (&self.card_content, &self.card),
            (&self.result_content, &self.result),
        ] {
            if *claimed != stable_digest(payload.as_bytes()) {
                return Err("content identity mismatch");
            }
        }
        if self.checks.len() < 4
            || self
                .checks
                .iter()
                .any(|v| v.contains('/') || v.contains('@'))
        {
            return Err("unsanitized or incomplete evidence");
        }
        Ok(())
    }
}
