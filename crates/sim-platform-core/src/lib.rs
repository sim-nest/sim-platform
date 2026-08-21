#![forbid(unsafe_code)]
//! Pure, fail-closed manifest contracts for the sole SIM platform owner.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

/// The only accepted package classifications. Missing metadata is pure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PackageRole {
    #[default]
    SimPure,
    PlatformBootstrap,
    PlatformCapsule,
    HostTool,
}

impl PackageRole {
    /// Parse explicit role metadata. `None` deliberately means `sim-pure`.
    ///
    /// # Errors
    /// Returns [`ValidationError::UnknownRole`] for any unrecognized value.
    pub fn parse(value: Option<&str>) -> Result<Self, ValidationError> {
        match value.unwrap_or("sim-pure") {
            "sim-pure" => Ok(Self::SimPure),
            "platform-bootstrap" => Ok(Self::PlatformBootstrap),
            "platform-capsule" => Ok(Self::PlatformCapsule),
            "host-tool" => Ok(Self::HostTool),
            other => Err(ValidationError::UnknownRole(other.to_owned())),
        }
    }
}

/// Context discovered from repository and package structure, not trusted input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationContext<'a> {
    pub owner_repository: &'a str,
    pub declaring_repository: &'a str,
    pub bootstrap_packages: &'a [&'a str],
    pub product_closure_roles: &'a [PackageRole],
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CapsuleManifest {
    pub schema: String,
    pub provider: String,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub shells: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    pub schema: String,
    pub capsule: String,
    pub artifact: String,
    pub artifact_content: String,
    pub entry: String,
    #[serde(default)]
    pub shell: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationError {
    Parse(String),
    WrongSchema {
        expected: &'static str,
        found: String,
    },
    CapsuleOutsideOwner,
    DuplicateProvider(String),
    UndeclaredShell(String),
    NonCanonicalBootstrap(String),
    HostToolInProductClosure,
    UnknownRole(String),
    EmptyField(&'static str),
    WrongEntry(String),
}

impl fmt::Display for ValidationError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{self:?}")
    }
}

impl std::error::Error for ValidationError {}

/// Parse a capsule manifest.
///
/// # Errors
/// Returns [`ValidationError::Parse`] when the TOML does not match the schema shape.
pub fn parse_capsule(source: &str) -> Result<CapsuleManifest, ValidationError> {
    toml::from_str(source).map_err(|error| ValidationError::Parse(error.to_string()))
}

/// Parse a bundle manifest.
///
/// # Errors
/// Returns [`ValidationError::Parse`] when the TOML does not match the schema shape.
pub fn parse_bundle(source: &str) -> Result<BundleManifest, ValidationError> {
    toml::from_str(source).map_err(|error| ValidationError::Parse(error.to_string()))
}

/// Validate discovered ownership plus a complete set of capsule records.
///
/// # Errors
/// Returns the first ownership, role, schema, or provider-identity violation.
pub fn validate_capsules(
    context: &ValidationContext<'_>,
    capsules: &[CapsuleManifest],
) -> Result<(), ValidationError> {
    if context.declaring_repository != context.owner_repository {
        return Err(ValidationError::CapsuleOutsideOwner);
    }
    if let Some(name) = context
        .bootstrap_packages
        .iter()
        .find(|name| **name != "sim-platform-bootstrap")
    {
        return Err(ValidationError::NonCanonicalBootstrap((*name).to_owned()));
    }
    if context
        .product_closure_roles
        .contains(&PackageRole::HostTool)
    {
        return Err(ValidationError::HostToolInProductClosure);
    }
    let mut providers = BTreeSet::new();
    for capsule in capsules {
        require_schema(&capsule.schema, "sim.platform-capsule/v1")?;
        require_nonempty(&capsule.provider, "provider")?;
        if !providers.insert(capsule.provider.clone()) {
            return Err(ValidationError::DuplicateProvider(capsule.provider.clone()));
        }
    }
    Ok(())
}

/// Validate a bundle against its selected capsule.
///
/// # Errors
/// Returns the first schema, content, entrypoint, or shell declaration violation.
pub fn validate_bundle(
    bundle: &BundleManifest,
    capsule: &CapsuleManifest,
) -> Result<(), ValidationError> {
    require_schema(&bundle.schema, "sim.platform-bundle/v1")?;
    for (value, field) in [
        (&bundle.capsule, "capsule"),
        (&bundle.artifact, "artifact"),
        (&bundle.artifact_content, "artifact_content"),
    ] {
        require_nonempty(value, field)?;
    }
    if bundle.entry != "sim_native_abi_v1" {
        return Err(ValidationError::WrongEntry(bundle.entry.clone()));
    }
    if let Some(shell) = &bundle.shell
        && !capsule.shells.contains(shell)
    {
        return Err(ValidationError::UndeclaredShell(shell.clone()));
    }
    Ok(())
}

fn require_schema(value: &str, expected: &'static str) -> Result<(), ValidationError> {
    if value == expected {
        Ok(())
    } else {
        Err(ValidationError::WrongSchema {
            expected,
            found: value.to_owned(),
        })
    }
}

fn require_nonempty(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if value.trim().is_empty() {
        Err(ValidationError::EmptyField(field))
    } else {
        Ok(())
    }
}

/// Stable, open identity for platform records and services.
///
/// The platform owner intentionally does not close this namespace into an enum:
/// providers may introduce symbols without changing this crate.
#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(transparent)]
pub struct OpenSymbol(pub String);

impl OpenSymbol {
    /// Constructs a non-empty slash-qualified symbol.
    ///
    /// # Errors
    /// Returns [`PlatformRecordError::InvalidSymbol`] for a malformed identity.
    pub fn new(value: impl Into<String>) -> Result<Self, PlatformRecordError> {
        let value = value.into();
        let valid = !value.trim().is_empty()
            && value.contains('/')
            && !value.starts_with('/')
            && !value.ends_with('/')
            && !value.chars().any(char::is_whitespace);
        valid
            .then_some(Self(value.clone()))
            .ok_or(PlatformRecordError::InvalidSymbol(value))
    }
}

/// Cross-domain facts a platform may expose. This is deliberately the complete
/// list: process, identity, environment and host-path access are not services.
#[derive(Clone, Copy, Debug, Deserialize, serde::Serialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum FactPort {
    WallClock,
    MonotonicClock,
    Timer,
    Entropy,
    Locale,
    Timezone,
    LifecyclePressure,
    MachineLimits,
}

/// Evidence strength attached to a service claim.
#[derive(Clone, Copy, Debug, Deserialize, serde::Serialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceLevel {
    Declared,
    Modeled,
    Measured,
    Attested,
}

/// One service offered by a platform card.
#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct ServiceOffer {
    pub service: OpenSymbol,
    pub port: FactPort,
    pub evidence: EvidenceLevel,
}

/// Shaped data describing an available platform without embedding behavior.
#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct PlatformCard {
    pub schema: OpenSymbol,
    pub site: OpenSymbol,
    pub services: Vec<ServiceOffer>,
    pub provenance: ContractProvenance,
}

/// One requested service, with ordered, explicit substitutes.
#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct Requirement {
    pub service: OpenSymbol,
    #[serde(default)]
    pub substitutes: Vec<OpenSymbol>,
    pub optional: bool,
    pub minimum_evidence: EvidenceLevel,
}

/// Atomic resolver input.
#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct PlatformRequest {
    pub request: OpenSymbol,
    pub requirements: Vec<Requirement>,
}

/// A requested identity and the concrete offer selected for it.
#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct ServiceBinding {
    pub requested: OpenSymbol,
    pub bound: OpenSymbol,
    pub port: FactPort,
    pub evidence: EvidenceLevel,
}

/// Complete result of an atomic resolution.
#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct BoundServices {
    pub request: OpenSymbol,
    pub site: OpenSymbol,
    pub bindings: Vec<ServiceBinding>,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RefusalKind {
    Unsupported,
    Denied,
    Unavailable,
    Suspended,
    Invalid,
    BudgetExhausted,
    Cancelled,
    ProviderFault,
}

/// Fail-closed resolver refusal. No partial bindings accompany it.
#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct ResolutionRefusal {
    pub request: OpenSymbol,
    pub service: OpenSymbol,
    pub kind: RefusalKind,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct ResolutionReceipt {
    pub id: OpenSymbol,
    pub request: OpenSymbol,
    pub site: OpenSymbol,
    pub bindings: Vec<ServiceBinding>,
    pub card_digest: String,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Lifecycle {
    Created,
    Ready,
    Pressured,
    Suspended,
    Stopped,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct Activation {
    pub id: OpenSymbol,
    pub site: OpenSymbol,
    pub lifecycle: Lifecycle,
    pub services: BoundServices,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct ContractProvenance {
    pub contract: OpenSymbol,
    pub content_digest: String,
    pub issuer: OpenSymbol,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct ExecutionEvidence {
    pub execution: OpenSymbol,
    pub activation: OpenSymbol,
    pub request_digest: String,
    pub result_digest: String,
    pub ledger_ref: OpenSymbol,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlatformRecordError {
    InvalidSymbol(String),
    DuplicateOffer(OpenSymbol),
}

impl fmt::Display for PlatformRecordError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{self:?}")
    }
}
impl std::error::Error for PlatformRecordError {}

/// Pure `platform/require` resolver. Selection is deterministic (requested
/// identity first, then substitutes in caller order) and atomic.
///
/// # Errors
/// Returns a refusal without partial bindings when a required service is
/// unsupported or does not satisfy its minimum evidence level.
///
/// # Panics
/// Panics only if serde cannot serialize the closed, data-only [`PlatformCard`]
/// representation, which its implementation makes infallible.
pub fn platform_require(
    card: &PlatformCard,
    request: &PlatformRequest,
) -> Result<(BoundServices, ResolutionReceipt), ResolutionRefusal> {
    let mut offers = BTreeMap::new();
    for offer in &card.services {
        offers.insert(&offer.service, offer);
    }
    let mut bindings = Vec::new();
    for requirement in &request.requirements {
        let candidates =
            std::iter::once(&requirement.service).chain(requirement.substitutes.iter());
        let mut weak = false;
        let selected = candidates
            .filter_map(|id| offers.get(id).copied())
            .find(|offer| {
                let enough = offer.evidence >= requirement.minimum_evidence;
                weak |= !enough;
                enough
            });
        match selected {
            Some(offer) => bindings.push(ServiceBinding {
                requested: requirement.service.clone(),
                bound: offer.service.clone(),
                port: offer.port,
                evidence: offer.evidence,
            }),
            None if requirement.optional => {}
            None => {
                return Err(ResolutionRefusal {
                    request: request.request.clone(),
                    service: requirement.service.clone(),
                    kind: RefusalKind::Unsupported,
                    detail: if weak {
                        "offers do not meet minimum evidence"
                    } else {
                        "service is unbound"
                    }
                    .into(),
                });
            }
        }
    }
    let bound = BoundServices {
        request: request.request.clone(),
        site: card.site.clone(),
        bindings,
    };
    let card_bytes = serde_json::to_vec(card).expect("platform records serialize");
    let digest = stable_digest(&card_bytes);
    let receipt = ResolutionReceipt {
        id: OpenSymbol(format!("receipt/{digest}")),
        request: request.request.clone(),
        site: card.site.clone(),
        bindings: bound.bindings.clone(),
        card_digest: digest,
    };
    Ok((bound, receipt))
}

/// Stable host-independent digest used for modeled receipts and evidence.
#[must_use]
pub fn stable_digest(bytes: &[u8]) -> String {
    // FNV-1a is used as an identity checksum, not for security.
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPSULE: &str = r#"
schema = "sim.platform-capsule/v1"
provider = "platform/site/fictional"
services = ["service/fictional-clock"]
shells = ["shell/fictional"]
"#;
    const BUNDLE: &str = r#"
schema = "sim.platform-bundle/v1"
capsule = "platform/site/fictional"
artifact = "lib/sim-platform-fictional"
artifact_content = "sha256:fictional"
entry = "sim_native_abi_v1"
shell = "shell/fictional"
"#;

    fn context<'a>() -> ValidationContext<'a> {
        ValidationContext {
            owner_repository: "sim-platform",
            declaring_repository: "sim-platform",
            bootstrap_packages: &["sim-platform-bootstrap"],
            product_closure_roles: &[PackageRole::SimPure, PackageRole::PlatformBootstrap],
        }
    }

    #[test]
    fn fictional_records_parse_and_validate() {
        let capsule = parse_capsule(CAPSULE).unwrap();
        validate_capsules(&context(), std::slice::from_ref(&capsule)).unwrap();
        validate_bundle(&parse_bundle(BUNDLE).unwrap(), &capsule).unwrap();
    }

    #[test]
    fn absent_role_is_pure_and_all_roles_are_closed() {
        assert_eq!(PackageRole::parse(None), Ok(PackageRole::SimPure));
        for (name, role) in [
            ("sim-pure", PackageRole::SimPure),
            ("platform-bootstrap", PackageRole::PlatformBootstrap),
            ("platform-capsule", PackageRole::PlatformCapsule),
            ("host-tool", PackageRole::HostTool),
        ] {
            assert_eq!(PackageRole::parse(Some(name)), Ok(role));
        }
        assert!(matches!(
            PackageRole::parse(Some("magic")),
            Err(ValidationError::UnknownRole(_))
        ));
    }

    #[test]
    fn rejects_capsule_outside_owner() {
        let mut ctx = context();
        ctx.declaring_repository = "sim-runtime";
        assert_eq!(
            validate_capsules(&ctx, &[parse_capsule(CAPSULE).unwrap()]),
            Err(ValidationError::CapsuleOutsideOwner)
        );
    }

    #[test]
    fn rejects_duplicate_provider_ids() {
        let capsule = parse_capsule(CAPSULE).unwrap();
        assert!(matches!(
            validate_capsules(&context(), &[capsule.clone(), capsule]),
            Err(ValidationError::DuplicateProvider(_))
        ));
    }

    #[test]
    fn rejects_undeclared_shell() {
        let capsule = parse_capsule(CAPSULE).unwrap();
        let mut bundle = parse_bundle(BUNDLE).unwrap();
        bundle.shell = Some("shell/other".into());
        assert!(matches!(
            validate_bundle(&bundle, &capsule),
            Err(ValidationError::UndeclaredShell(_))
        ));
    }

    #[test]
    fn rejects_noncanonical_bootstrap_and_host_tool_closure() {
        let capsule = parse_capsule(CAPSULE).unwrap();
        let mut ctx = context();
        ctx.bootstrap_packages = &["other-bootstrap"];
        assert!(matches!(
            validate_capsules(&ctx, std::slice::from_ref(&capsule)),
            Err(ValidationError::NonCanonicalBootstrap(_))
        ));
        let mut ctx = context();
        ctx.product_closure_roles = &[PackageRole::HostTool];
        assert_eq!(
            validate_capsules(&ctx, &[capsule]),
            Err(ValidationError::HostToolInProductClosure)
        );
    }
}
