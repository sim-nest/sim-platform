#![forbid(unsafe_code)]
//! Pure, fail-closed manifest contracts for the sole SIM platform owner.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

/// Shared Ubuntu profile facts composed by concrete PC and Pi capsules.
#[derive(Clone, Copy, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum UbuntuArchitecture {
    X86_64,
    Aarch64,
}

#[derive(Clone, Copy, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum UbuntuProfileKind {
    Desktop,
    Headless,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct UbuntuProfile {
    pub architecture: UbuntuArchitecture,
    pub kind: UbuntuProfileKind,
}

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
    /// Exact loader kinds implemented by this capsule.
    #[serde(default)]
    pub loader_kinds: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    pub schema: String,
    pub capsule: String,
    pub artifact: String,
    /// Exact loader kind used to realize `artifact`.
    pub loader: String,
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
    UnsupportedLoader(String),
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
        (&bundle.loader, "loader"),
    ] {
        require_nonempty(value, field)?;
    }
    if bundle.entry != "sim_native_abi_v1" {
        return Err(ValidationError::WrongEntry(bundle.entry.clone()));
    }
    if !capsule.loader_kinds.contains(&bundle.loader) {
        return Err(ValidationError::UnsupportedLoader(bundle.loader.clone()));
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

/// Builder for an ordered, provider-neutral platform requirement.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RequirementBuilder {
    requirements: Vec<Requirement>,
}

impl RequirementBuilder {
    /// Starts an empty requirement set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one required service.
    ///
    /// # Errors
    /// Returns [`PlatformRecordError::InvalidSymbol`] for an empty service identity.
    pub fn require(mut self, service: impl Into<String>) -> Result<Self, PlatformRecordError> {
        self.requirements.push(Requirement {
            service: OpenSymbol::new(service)?,
            optional: false,
            substitutes: Vec::new(),
            minimum_evidence: EvidenceLevel::Modeled,
        });
        Ok(self)
    }

    /// Appends one optional service with ordered substitutes.
    ///
    /// # Errors
    /// Returns [`PlatformRecordError::InvalidSymbol`] for an empty service or substitute.
    pub fn optional<I, S>(
        mut self,
        service: impl Into<String>,
        substitutes: I,
    ) -> Result<Self, PlatformRecordError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.requirements.push(Requirement {
            service: OpenSymbol::new(service)?,
            optional: true,
            substitutes: substitutes
                .into_iter()
                .map(Into::into)
                .map(OpenSymbol::new)
                .collect::<Result<_, _>>()?,
            minimum_evidence: EvidenceLevel::Modeled,
        });
        Ok(self)
    }

    /// Finishes the ordered requirement list.
    #[must_use]
    pub fn build(self) -> Vec<Requirement> {
        self.requirements
    }
}

/// Contract implemented by capsule authors without exposing concrete capsules.
pub trait PlatformProviderAuthor {
    /// Returns shaped discovery data for the provider.
    fn card(&self) -> PlatformCard;
    /// Returns the exact services the capsule binds for this card.
    fn bound_services(&self) -> BoundServices;
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

/// Content-addressed application or library admitted to a runtime bundle.
#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, Ord, PartialEq, PartialOrd)]
pub struct BundleContent {
    pub id: OpenSymbol,
    pub content_digest: String,
    /// Capabilities this content is permitted to exercise.
    #[serde(default)]
    pub capabilities: Vec<OpenSymbol>,
}

/// Evidence retained beside a capsule Card. Contract provenance and execution
/// evidence deliberately remain distinct so generated support matrices cannot
/// turn a declaration into a tested claim.
#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct CapsuleAttestation {
    pub capsule: OpenSymbol,
    pub artifact_digest: String,
    pub card_digest: String,
    pub evidence: ExecutionEvidence,
}

/// The sole capsule artifact emitted into a composed bundle.
#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct CapsuleArtifact {
    pub capsule: OpenSymbol,
    pub content_digest: String,
}

/// Ordered, content-addressed plan consumed identically by every boot entry.
#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct LibraryLoadPlan {
    pub application: BundleContent,
    pub libraries: Vec<BundleContent>,
}

/// Provider-neutral boot data. It contains no target-selection mechanism.
#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct PureBootEnvelope {
    pub schema: OpenSymbol,
    pub capsule: OpenSymbol,
    pub bootstrap: OpenSymbol,
    pub load_plan: LibraryLoadPlan,
}

/// Complete runtime bundle: exactly one descriptor and one capsule artifact.
#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct ComposedBundle {
    pub bootstrap: PureBootEnvelope,
    pub capsule_artifact: CapsuleArtifact,
    pub card: PlatformCard,
    pub attestation: CapsuleAttestation,
}

/// Pure build-tool input. A build tool selects `capsule`; runtime data merely
/// records that explicit choice and the exact content closure.
pub struct BundleComposition<'a> {
    pub capsule: &'a OpenSymbol,
    pub application: BundleContent,
    pub libraries: Vec<BundleContent>,
    pub cards: &'a [PlatformCard],
    pub attestations: &'a [CapsuleAttestation],
    pub declared_artifacts: &'a BTreeMap<OpenSymbol, String>,
    pub allowed_capabilities: &'a BTreeSet<OpenSymbol>,
    pub required_services: &'a [OpenSymbol],
}

/// Typed, fail-closed bundle-composition refusal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BundleRefusal {
    MissingCapsule(OpenSymbol),
    DuplicateProvider(OpenSymbol),
    MissingAttestation(OpenSymbol),
    DuplicateAttestation(OpenSymbol),
    UndeclaredArtifact(OpenSymbol),
    EvidenceContentMismatch(OpenSymbol),
    CapabilityEscalation {
        content: OpenSymbol,
        capability: OpenSymbol,
    },
    MissingRequiredService(OpenSymbol),
    DuplicateContent(OpenSymbol),
}

impl fmt::Display for BundleRefusal {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{self:?}")
    }
}
impl std::error::Error for BundleRefusal {}

/// Composes one selected capsule with an explicit application and library
/// closure. No substitute capsule is considered when the selected Card is
/// absent or invalid.
pub fn compose_bundle(input: BundleComposition<'_>) -> Result<ComposedBundle, BundleRefusal> {
    let matching_cards = input
        .cards
        .iter()
        .filter(|card| card.site == *input.capsule)
        .collect::<Vec<_>>();
    let card = match matching_cards.as_slice() {
        [] => return Err(BundleRefusal::MissingCapsule(input.capsule.clone())),
        [card] => *card,
        _ => return Err(BundleRefusal::DuplicateProvider(input.capsule.clone())),
    };
    let matching_attestations = input
        .attestations
        .iter()
        .filter(|item| item.capsule == *input.capsule)
        .collect::<Vec<_>>();
    let attestation = match matching_attestations.as_slice() {
        [] => return Err(BundleRefusal::MissingAttestation(input.capsule.clone())),
        [attestation] => *attestation,
        _ => return Err(BundleRefusal::DuplicateAttestation(input.capsule.clone())),
    };
    let Some(artifact_digest) = input.declared_artifacts.get(input.capsule) else {
        return Err(BundleRefusal::UndeclaredArtifact(input.capsule.clone()));
    };
    let card_digest = stable_digest(&serde_json::to_vec(card).expect("platform Card serializes"));
    if attestation.artifact_digest != *artifact_digest || attestation.card_digest != card_digest {
        return Err(BundleRefusal::EvidenceContentMismatch(
            input.capsule.clone(),
        ));
    }
    for service in input.required_services {
        if !card.services.iter().any(|offer| offer.service == *service) {
            return Err(BundleRefusal::MissingRequiredService(service.clone()));
        }
    }
    let mut content_ids = BTreeSet::new();
    for content in std::iter::once(&input.application).chain(input.libraries.iter()) {
        if !content_ids.insert(content.id.clone()) {
            return Err(BundleRefusal::DuplicateContent(content.id.clone()));
        }
        if let Some(capability) = content
            .capabilities
            .iter()
            .find(|capability| !input.allowed_capabilities.contains(*capability))
        {
            return Err(BundleRefusal::CapabilityEscalation {
                content: content.id.clone(),
                capability: capability.clone(),
            });
        }
    }
    let bootstrap = PureBootEnvelope {
        schema: OpenSymbol("boot/envelope/v1".into()),
        capsule: input.capsule.clone(),
        bootstrap: OpenSymbol("bootstrap/sim-native-abi-v1".into()),
        load_plan: LibraryLoadPlan {
            application: input.application,
            libraries: input.libraries,
        },
    };
    Ok(ComposedBundle {
        bootstrap,
        capsule_artifact: CapsuleArtifact {
            capsule: input.capsule.clone(),
            content_digest: artifact_digest.clone(),
        },
        card: card.clone(),
        attestation: attestation.clone(),
    })
}

/// One generated support-matrix row derived only from a retained Card and its
/// matching attestation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformSupportRow {
    pub capsule: OpenSymbol,
    pub service: OpenSymbol,
    pub contract_provenance: String,
    pub execution_evidence: String,
}

/// Generates support rows without consulting target names or hand-maintained
/// support lists.
pub fn platform_support_matrix(bundles: &[ComposedBundle]) -> Vec<PlatformSupportRow> {
    let mut rows = bundles
        .iter()
        .flat_map(|bundle| {
            bundle.card.services.iter().map(|offer| PlatformSupportRow {
                capsule: bundle.card.site.clone(),
                service: offer.service.clone(),
                contract_provenance: bundle.card.provenance.content_digest.clone(),
                execution_evidence: bundle.attestation.evidence.result_digest.clone(),
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        (&left.capsule, &left.service).cmp(&(&right.capsule, &right.service))
    });
    rows
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
loader_kinds = ["loader/static-v1"]
"#;
    const BUNDLE: &str = r#"
schema = "sim.platform-bundle/v1"
capsule = "platform/site/fictional"
artifact = "lib/sim-platform-fictional"
loader = "loader/static-v1"
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
