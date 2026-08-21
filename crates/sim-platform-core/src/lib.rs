#![forbid(unsafe_code)]
//! Pure, fail-closed manifest contracts for the sole SIM platform owner.

use serde::Deserialize;
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
