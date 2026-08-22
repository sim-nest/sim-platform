#![forbid(unsafe_code)]
//! Headless Ubuntu Raspberry Pi capsule with profile-owned native facts.

use serde::{Deserialize, Serialize};
use sim_platform_core::{
    ContractProvenance, EvidenceLevel, FactPort, OpenSymbol, PlatformCard, ServiceOffer,
    UbuntuArchitecture, UbuntuProfile, UbuntuProfileKind, stable_digest,
};
use std::collections::BTreeMap;

/// The complete domain-facing service vocabulary of this capsule.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum PiService {
    Gpio,
    Serial,
    I2c,
    Spi,
    Audio,
    Network,
    Storage,
    Process,
    Compute,
}

impl PiService {
    pub const ALL: [Self; 9] = [
        Self::Gpio,
        Self::Serial,
        Self::I2c,
        Self::Spi,
        Self::Audio,
        Self::Network,
        Self::Storage,
        Self::Process,
        Self::Compute,
    ];

    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Gpio => "device/gpio",
            Self::Serial => "device/serial",
            Self::I2c => "device/i2c",
            Self::Spi => "device/spi",
            Self::Audio => "audio/device",
            Self::Network => "network/socket",
            Self::Storage => "storage/host-dir",
            Self::Process => "runtime/process",
            Self::Compute => "compute/cpu",
        }
    }
}

/// A concrete binding held at the provider boundary, never by a domain consumer.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "value")]
pub enum Binding {
    DevicePath(String),
    Api(String),
}

/// Permission needed before the capsule can open a binding.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct PermissionRule {
    pub group: String,
    pub read: bool,
    pub write: bool,
}

/// All Raspberry Pi-specific facts. Adding a board never changes a consumer.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct UbuntuRpiProfile {
    pub ubuntu: UbuntuProfile,
    pub board_revision: String,
    pub bindings: BTreeMap<PiService, Binding>,
    pub permissions: BTreeMap<PiService, PermissionRule>,
}

impl UbuntuRpiProfile {
    /// Derive only the common headless Ubuntu identity; services are explicit.
    #[must_use]
    pub fn from_ubuntu_headless(board_revision: impl Into<String>) -> Self {
        Self {
            ubuntu: UbuntuProfile {
                architecture: UbuntuArchitecture::Aarch64,
                kind: UbuntuProfileKind::Headless,
            },
            board_revision: board_revision.into(),
            bindings: BTreeMap::new(),
            permissions: BTreeMap::new(),
        }
    }

    /// Resolve a declared domain service without probing or fallback.
    pub fn require(&self, service: PiService) -> Result<&Binding, PiRefusal> {
        let binding = self
            .bindings
            .get(&service)
            .ok_or(PiRefusal::Unsupported { service })?;
        if !self.permissions.contains_key(&service) {
            return Err(PiRefusal::PermissionProfileMissing { service });
        }
        Ok(binding)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PiRefusal {
    Unsupported { service: PiService },
    PermissionProfileMissing { service: PiService },
}

/// Registration output pairs the immutable profile and its exact observed Card.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredPiCard {
    pub profile: UbuntuRpiProfile,
    pub card: PlatformCard,
}

#[must_use]
pub fn register(profile: UbuntuRpiProfile) -> RegisteredPiCard {
    let services = PiService::ALL
        .into_iter()
        .filter(|service| profile.bindings.contains_key(service))
        .map(|service| ServiceOffer {
            service: OpenSymbol(service.symbol().into()),
            port: FactPort::MachineLimits,
            evidence: EvidenceLevel::Declared,
        })
        .collect();
    let bytes = serde_json::to_vec(&profile).expect("closed Pi profile serializes");
    RegisteredPiCard {
        card: PlatformCard {
            schema: OpenSymbol("platform/card-v1".into()),
            site: OpenSymbol("platform/site/ubuntu-rpi-headless".into()),
            services,
            provenance: ContractProvenance {
                contract: OpenSymbol("contract/ubuntu-rpi-v1".into()),
                content_digest: stable_digest(&bytes),
                issuer: OpenSymbol("issuer/sim-platform".into()),
            },
        },
        profile,
    }
}

/// Deterministic hostile profile: only bindings explicitly inserted exist.
#[must_use]
pub fn hostile_model_profile() -> UbuntuRpiProfile {
    let mut profile = UbuntuRpiProfile::from_ubuntu_headless("model-hostile");
    profile
        .bindings
        .insert(PiService::Gpio, Binding::Api("model/gpio-v1".into()));
    profile.permissions.insert(
        PiService::Gpio,
        PermissionRule {
            group: "modeled".into(),
            read: true,
            write: true,
        },
    );
    profile
}

/// Raspberry Pi 5 profile for Ubuntu's documented kernel interfaces.
///
/// No path is opened here. Activation code must verify the recorded group
/// permission before constructing the domain-owned port implementation.
#[must_use]
pub fn raspberry_pi_5_profile() -> UbuntuRpiProfile {
    let mut profile = UbuntuRpiProfile::from_ubuntu_headless("d04170");
    for (service, binding, group, write) in [
        (
            PiService::Gpio,
            Binding::DevicePath("/dev/gpiochip4".into()),
            "gpio",
            true,
        ),
        (
            PiService::Serial,
            Binding::DevicePath("/dev/serial0".into()),
            "dialout",
            true,
        ),
        (
            PiService::I2c,
            Binding::DevicePath("/dev/i2c-1".into()),
            "i2c",
            true,
        ),
        (
            PiService::Spi,
            Binding::DevicePath("/dev/spidev0.0".into()),
            "spi",
            true,
        ),
        (
            PiService::Audio,
            Binding::Api("alsa/default".into()),
            "audio",
            true,
        ),
        (
            PiService::Network,
            Binding::Api("linux/socket-v1".into()),
            "sim",
            true,
        ),
        (
            PiService::Storage,
            Binding::Api("linux/preopened-dir-v1".into()),
            "sim",
            true,
        ),
        (
            PiService::Process,
            Binding::Api("linux/process-v1".into()),
            "sim",
            true,
        ),
        (
            PiService::Compute,
            Binding::Api("linux/aarch64-cpu-v1".into()),
            "sim",
            false,
        ),
    ] {
        profile.bindings.insert(service, binding);
        profile.permissions.insert(
            service,
            PermissionRule {
                group: group.into(),
                read: true,
                write,
            },
        );
    }
    profile
}

pub const CAPSULE_MANIFEST: &str = r#"
schema = "sim.platform-capsule/v1"
provider = "platform/site/ubuntu-rpi-headless"
services = ["device/gpio"]
shells = []
loader_kinds = ["loader/native-v1"]
"#;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum BuildEvidence {
    CrossBuilt,
    Physical,
}

/// Offline evidence for declared Rust targets. Physical is forbidden without a host receipt.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct TargetAttestation {
    pub targets: Vec<String>,
    pub evidence: BuildEvidence,
    pub registered_host: Option<String>,
}

impl TargetAttestation {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.targets.is_empty() {
            return Err("no declared target was built");
        }
        if self.evidence == BuildEvidence::Physical && self.registered_host.is_none() {
            return Err("physical evidence requires a registered host");
        }
        Ok(())
    }
}
