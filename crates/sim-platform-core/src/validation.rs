use serde::Deserialize;
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
