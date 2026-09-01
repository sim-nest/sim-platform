use super::*;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sim_platform_core::{
    ContractProvenance, EvidenceLevel, FactPort, OpenSymbol, PlatformCard, ServiceOffer,
    stable_digest,
};
use std::collections::BTreeSet;

pub const NATIVE_ABI_ENTRY: &str = "sim_native_abi_v1";
pub const CAPSULE_CARD_NAME: &str = "sim.platform-capsule.toml";
pub const BUNDLE_DESCRIPTOR_NAME: &str = "sim.platform-bundle.toml";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum MacArchitecture {
    X86_64,
    Aarch64,
}

impl MacArchitecture {
    #[must_use]
    pub const fn target(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64-apple-darwin",
            Self::Aarch64 => "aarch64-apple-darwin",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permission {
    Camera,
    Microphone,
    ScreenRecording,
    Accessibility,
}

impl Permission {
    #[must_use]
    pub const fn capability(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::Microphone => "microphone",
            Self::ScreenRecording => "screen-recording",
            Self::Accessibility => "accessibility",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionStatus {
    Granted,
    Denied,
    Undetermined,
    Restricted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lifecycle {
    Ready,
    Active,
    Suspended,
    Stopped,
}

pub trait NativeServices {
    fn permission_status(&mut self, permission: Permission) -> PermissionStatus;
    fn request_permission(&mut self, permission: Permission) -> PermissionStatus;
    /// Activate the application surface.
    ///
    /// # Errors
    /// Returns a sanitized native failure without exposing a framework handle.
    fn activate(&mut self) -> Result<(), &'static str>;
    fn cleanup(&mut self);
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapsuleError {
    MissingCapability(String),
    Suspended,
    Stopped,
    Native(&'static str),
}

pub struct MacosCapsule<N: NativeServices> {
    native: N,
    grants: BTreeSet<String>,
    lifecycle: Lifecycle,
}

impl<N: NativeServices> MacosCapsule<N> {
    #[must_use]
    pub fn new(native: N, grants: impl IntoIterator<Item = String>) -> Self {
        Self {
            native,
            grants: grants.into_iter().collect(),
            lifecycle: Lifecycle::Ready,
        }
    }
    #[must_use]
    pub const fn lifecycle(&self) -> Lifecycle {
        self.lifecycle
    }
    /// Observes authorization without ever entering a prompting API.
    ///
    /// # Errors
    /// Refuses requests while the capsule is suspended or stopped.
    pub fn permission_status(
        &mut self,
        permission: Permission,
    ) -> Result<PermissionStatus, CapsuleError> {
        self.ensure_running()?;
        Ok(self.native.permission_status(permission))
    }
    /// The sole prompt route. Capability is checked before native dispatch.
    ///
    /// # Errors
    /// Refuses a missing matching capability or an inactive capsule.
    pub fn request_permission(
        &mut self,
        permission: Permission,
    ) -> Result<PermissionStatus, CapsuleError> {
        self.ensure_running()?;
        let required = format!("platform/permission-request/{}", permission.capability());
        if !self.grants.contains(&required) {
            return Err(CapsuleError::MissingCapability(required));
        }
        Ok(self.native.request_permission(permission))
    }
    /// Activate the ordinary platform Site.
    ///
    /// # Errors
    /// Propagates a sanitized native failure or an inactive-capsule refusal.
    pub fn activate(&mut self) -> Result<(), CapsuleError> {
        self.ensure_running()?;
        self.native.activate().map_err(CapsuleError::Native)?;
        self.lifecycle = Lifecycle::Active;
        Ok(())
    }
    /// Suspend request dispatch.
    ///
    /// # Errors
    /// Refuses an already stopped capsule.
    pub fn suspend(&mut self) -> Result<(), CapsuleError> {
        self.ensure_running()?;
        self.lifecycle = Lifecycle::Suspended;
        Ok(())
    }
    /// Resume request dispatch.
    ///
    /// # Errors
    /// Refuses an already stopped capsule.
    pub fn resume(&mut self) -> Result<(), CapsuleError> {
        if self.lifecycle != Lifecycle::Suspended {
            return self.ensure_running();
        }
        self.lifecycle = Lifecycle::Ready;
        Ok(())
    }
    pub fn stop(&mut self) {
        if self.lifecycle != Lifecycle::Stopped {
            self.native.cleanup();
            self.lifecycle = Lifecycle::Stopped;
        }
    }
    fn ensure_running(&self) -> Result<(), CapsuleError> {
        match self.lifecycle {
            Lifecycle::Suspended => Err(CapsuleError::Suspended),
            Lifecycle::Stopped => Err(CapsuleError::Stopped),
            _ => Ok(()),
        }
    }
}

impl<N: NativeServices> Drop for MacosCapsule<N> {
    fn drop(&mut self) {
        self.stop();
    }
}

#[must_use]
pub fn platform_card(architecture: MacArchitecture) -> PlatformCard {
    let services = SERVICES
        .iter()
        .map(|binding| ServiceOffer {
            service: OpenSymbol(format!("platform/{}", binding.service)),
            port: FactPort::MachineLimits,
            evidence: EvidenceLevel::Attested,
        })
        .collect();
    PlatformCard {
        schema: OpenSymbol("platform/card-v1".into()),
        site: OpenSymbol("platform/site/macos".into()),
        services,
        provenance: ContractProvenance {
            contract: OpenSymbol("contract/macos-capsule-v1".into()),
            content_digest: stable_digest(architecture.target().as_bytes()),
            issuer: OpenSymbol("issuer/sim-platform".into()),
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapsuleInput {
    pub architecture: MacArchitecture,
    pub rind: Vec<u8>,
    pub capsule: Vec<u8>,
    pub resources: Vec<(String, Vec<u8>)>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct BundleFile {
    pub path: String,
    pub content_digest: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct UnsignedBundle {
    pub schema: String,
    pub target: String,
    pub identity: String,
    pub files: Vec<BundleFile>,
}

/// Assemble a canonical unsigned development bundle. No clock, host path,
/// signing identity, or filesystem enumeration contributes to the result.
#[must_use]
pub fn build_unsigned_bundle(mut input: CapsuleInput) -> UnsignedBundle {
    input.resources.sort_by(|left, right| left.0.cmp(&right.0));
    let artifact_name = format!("sim-platform-macos-{}.dylib", input.architecture.target());
    let artifact_digest = sha256(&input.capsule);
    let card = format!(
        "schema = \"sim.platform-capsule/v1\"\nprovider = \"platform/site/macos\"\nservices = [{}]\nshells = [\"shell/macos-app\"]\nloader_kinds = [\"loader/native-v1\"]\n",
        SERVICES
            .iter()
            .map(|s| format!("\"platform/{}\"", s.service))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let descriptor = format!(
        "schema = \"sim.platform-bundle/v1\"\ncapsule = \"{CAPSULE_CARD_NAME}\"\nartifact = \"{artifact_name}\"\nloader = \"loader/native-v1\"\nartifact_content = \"{artifact_digest}\"\nentry = \"{NATIVE_ABI_ENTRY}\"\nshell = \"shell/macos-app\"\n"
    );
    let mut files = vec![
        file("Contents/MacOS/sim", input.rind),
        file(format!("Contents/Resources/{artifact_name}"), input.capsule),
        file(
            format!("Contents/Resources/{CAPSULE_CARD_NAME}"),
            card.into_bytes(),
        ),
        file(
            format!("Contents/Resources/{BUNDLE_DESCRIPTOR_NAME}"),
            descriptor.into_bytes(),
        ),
        file(
            "Contents/Info.plist",
            include_bytes!("../package/Info.plist").to_vec(),
        ),
        file(
            "Contents/Resources/sim.entitlements",
            include_bytes!("../native/sim.entitlements").to_vec(),
        ),
    ];
    files.extend(
        input
            .resources
            .into_iter()
            .map(|(name, bytes)| file(format!("Contents/Resources/{name}"), bytes)),
    );
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let mut identity_source = Vec::new();
    for entry in &files {
        identity_source.extend_from_slice(entry.path.as_bytes());
        identity_source.extend_from_slice(entry.content_digest.as_bytes());
    }
    UnsignedBundle {
        schema: "sim.macos-unsigned-bundle/v1".into(),
        target: input.architecture.target().into(),
        identity: sha256(&identity_source),
        files,
    }
}

fn file(path: impl Into<String>, bytes: Vec<u8>) -> BundleFile {
    BundleFile {
        path: path.into(),
        content_digest: sha256(&bytes),
        bytes,
    }
}
fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
