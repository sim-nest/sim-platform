use serde::{Deserialize, Serialize};
use sim_platform_core::{
    ContractProvenance, EvidenceLevel, FactPort, OpenSymbol, PlatformCard, ServiceOffer,
    stable_digest,
};
use std::collections::{BTreeMap, BTreeSet};
use wasmparser::{Parser, Payload};

/// Content reference supplied by a bundle. There is no path-valued variant.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PreopenRef {
    Table { table: String, key: String },
    Dir { table: String, key: String },
}

/// Existing service reached by one admitted WASI import.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CanonicalService {
    PreopenedDirectories,
    WallClock,
    MonotonicClock,
    Entropy,
    Sockets,
    Lifecycle,
}

impl CanonicalService {
    #[must_use]
    pub const fn identity(self) -> &'static str {
        match self {
            Self::PreopenedDirectories => "storage/table-dir",
            Self::WallClock => "platform/wall-clock",
            Self::MonotonicClock => "platform/monotonic",
            Self::Entropy => "platform/entropy",
            Self::Sockets => "transport/socket",
            Self::Lifecycle => "platform/lifecycle",
        }
    }

    #[must_use]
    pub const fn capability(self) -> &'static str {
        match self {
            Self::PreopenedDirectories => "storage/preopen",
            Self::WallClock => "platform/wall-clock",
            Self::MonotonicClock => "platform/monotonic",
            Self::Entropy => "platform/entropy",
            Self::Sockets => "transport/socket",
            Self::Lifecycle => "platform/lifecycle",
        }
    }
}

/// One exact artifact import beside the canonical SIM service it realizes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImportDeclaration {
    pub module: &'static str,
    pub name: &'static str,
    pub service: CanonicalService,
}

/// Closed component profile. Extending the WASI surface requires an explicit
/// source change and therefore cannot happen through host feature discovery.
pub const ALLOWED_IMPORTS: &[ImportDeclaration] = &[
    ImportDeclaration {
        module: "wasi:filesystem/preopens@0.2.0",
        name: "get-directories",
        service: CanonicalService::PreopenedDirectories,
    },
    ImportDeclaration {
        module: "wasi:clocks/wall-clock@0.2.0",
        name: "now",
        service: CanonicalService::WallClock,
    },
    ImportDeclaration {
        module: "wasi:clocks/monotonic-clock@0.2.0",
        name: "now",
        service: CanonicalService::MonotonicClock,
    },
    ImportDeclaration {
        module: "wasi:random/random@0.2.0",
        name: "get-random-bytes",
        service: CanonicalService::Entropy,
    },
    ImportDeclaration {
        module: "wasi:sockets/tcp@0.2.0",
        name: "start-connect",
        service: CanonicalService::Sockets,
    },
    ImportDeclaration {
        module: "wasi:cli/exit@0.2.0",
        name: "exit",
        service: CanonicalService::Lifecycle,
    },
];

/// The entire discoverable authority of one activation.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WasiProfile {
    pub preopens: BTreeMap<String, PreopenRef>,
    pub capabilities: BTreeSet<String>,
    pub wall_clock_ticks: u64,
    pub monotonic_ticks: u64,
    pub entropy_seed: [u8; 32],
    pub socket_endpoints: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WasiRefusal {
    MalformedArtifact(String),
    UndeclaredImport { module: String, name: String },
    CapabilityDenied { capability: &'static str },
    UnsupportedService { service: String },
    UnknownPreopen { name: String },
    UnknownSocket { endpoint: String },
}

/// Inspect the complete core Wasm import section before instantiation.
///
/// # Errors
/// Refuses malformed artifacts and the first import absent from
/// [`ALLOWED_IMPORTS`].
pub fn inspect_artifact(bytes: &[u8]) -> Result<Vec<ImportDeclaration>, WasiRefusal> {
    let mut admitted = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|error| WasiRefusal::MalformedArtifact(error.to_string()))?;
        if let Payload::ImportSection(section) = payload {
            for import in section {
                let import =
                    import.map_err(|error| WasiRefusal::MalformedArtifact(error.to_string()))?;
                let declaration = ALLOWED_IMPORTS
                    .iter()
                    .find(|item| item.module == import.module && item.name == import.name)
                    .copied()
                    .ok_or_else(|| WasiRefusal::UndeclaredImport {
                        module: import.module.to_owned(),
                        name: import.name.to_owned(),
                    })?;
                admitted.push(declaration);
            }
        }
    }
    Ok(admitted)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WasiCapsule {
    profile: WasiProfile,
}

impl WasiCapsule {
    #[must_use]
    pub fn new(profile: WasiProfile) -> Self {
        Self { profile }
    }

    /// Resolve a named preopen without consulting cwd, environment, or host paths.
    ///
    /// # Errors
    /// Refuses a missing preopen capability or a name absent from the bundle.
    pub fn preopen(&self, name: &str) -> Result<&PreopenRef, WasiRefusal> {
        self.require(CanonicalService::PreopenedDirectories)?;
        self.profile
            .preopens
            .get(name)
            .ok_or_else(|| WasiRefusal::UnknownPreopen {
                name: name.to_owned(),
            })
    }

    /// Resolve a declared endpoint; DNS and interface discovery are not exposed.
    ///
    /// # Errors
    /// Refuses a missing socket capability or an undeclared endpoint.
    pub fn connect(&self, endpoint: &str) -> Result<(), WasiRefusal> {
        self.require(CanonicalService::Sockets)?;
        self.profile
            .socket_endpoints
            .contains(endpoint)
            .then_some(())
            .ok_or_else(|| WasiRefusal::UnknownSocket {
                endpoint: endpoint.to_owned(),
            })
    }

    /// Capability-check a canonical service before its host adapter is invoked.
    ///
    /// # Errors
    /// Refuses when the bundle does not carry the service's exact capability.
    pub fn require(&self, service: CanonicalService) -> Result<(), WasiRefusal> {
        let capability = service.capability();
        self.profile
            .capabilities
            .contains(capability)
            .then_some(())
            .ok_or(WasiRefusal::CapabilityDenied { capability })
    }

    /// Typed refusal for services deliberately absent from the WASI Card.
    pub fn unsupported(&self, service: impl Into<String>) -> WasiRefusal {
        WasiRefusal::UnsupportedService {
            service: service.into(),
        }
    }

    /// Stable model result depending only on the explicit profile and request.
    ///
    /// # Panics
    /// Panics only if serialization of the closed, data-only profile fails.
    #[must_use]
    pub fn model_digest(&self, request: &[u8]) -> String {
        let mut bytes = serde_json::to_vec(&self.profile).expect("closed profile serializes");
        bytes.extend_from_slice(request);
        stable_digest(&bytes)
    }
}

/// Card contains only the six canonical services. Process creation, native
/// dynamic loading, UI, notifications, and devices are intentionally absent.
///
/// # Panics
/// Panics only if serialization of the closed, data-only profile fails.
#[must_use]
pub fn platform_card(profile: &WasiProfile) -> PlatformCard {
    let services = ALLOWED_IMPORTS
        .iter()
        .map(|declaration| {
            let port = match declaration.service {
                CanonicalService::WallClock => FactPort::WallClock,
                CanonicalService::MonotonicClock => FactPort::MonotonicClock,
                CanonicalService::Entropy => FactPort::Entropy,
                CanonicalService::Lifecycle => FactPort::LifecyclePressure,
                CanonicalService::PreopenedDirectories | CanonicalService::Sockets => {
                    FactPort::MachineLimits
                }
            };
            ServiceOffer {
                service: OpenSymbol(declaration.service.identity().into()),
                port,
                evidence: EvidenceLevel::Modeled,
            }
        })
        .collect();
    let profile_bytes = serde_json::to_vec(profile).expect("closed profile serializes");
    PlatformCard {
        schema: OpenSymbol("platform/card-v1".into()),
        site: OpenSymbol("platform/site/wasi".into()),
        services,
        provenance: ContractProvenance {
            contract: OpenSymbol("contract/wasi-component-v1".into()),
            content_digest: stable_digest(&profile_bytes),
            issuer: OpenSymbol("issuer/sim-platform".into()),
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceGrade {
    InMemory,
    RealRuntime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameEvidence {
    pub grade: EvidenceGrade,
    pub cases: usize,
    pub digest: String,
}

/// Run ABI value frames through the in-memory codec/runtime boundary.
///
/// # Errors
/// Returns a codec error when a frame does not round-trip.
pub fn run_in_memory_frame_corpus(
    values: &[sim_wasm_abi::AbiValue],
) -> Result<FrameEvidence, String> {
    let mut evidence = Vec::new();
    for value in values {
        let encoded = sim_wasm_abi::encode_value_frame(value).map_err(|error| error.to_string())?;
        let decoded =
            sim_wasm_abi::decode_value_frame(&encoded).map_err(|error| error.to_string())?;
        if &decoded != value {
            return Err("ABI value frame changed during round-trip".into());
        }
        evidence.extend_from_slice(encoded.bytes());
    }
    Ok(FrameEvidence {
        grade: EvidenceGrade::InMemory,
        cases: values.len(),
        digest: stable_digest(&evidence),
    })
}

/// Pluggable hosted runner keeps a real-runtime result distinct from modeled proof.
pub trait RealWasiRuntime {
    /// Execute the corpus and return a content receipt from the hosted runtime.
    ///
    /// # Errors
    /// Returns the hosted runtime's typed execution or frame-parity failure.
    fn run_frame_corpus(&self, values: &[sim_wasm_abi::AbiValue]) -> Result<Vec<u8>, String>;
}

/// Execute the corpus through an injected real WASI runtime.
///
/// # Errors
/// Returns the hosted runtime's execution or frame-parity failure.
pub fn run_real_frame_corpus(
    runtime: &dyn RealWasiRuntime,
    values: &[sim_wasm_abi::AbiValue],
) -> Result<FrameEvidence, String> {
    let receipt = runtime.run_frame_corpus(values)?;
    Ok(FrameEvidence {
        grade: EvidenceGrade::RealRuntime,
        cases: values.len(),
        digest: stable_digest(&receipt),
    })
}

pub const CAPSULE_MANIFEST: &str = include_str!("../component/wasi-capsule.toml");
