use super::{SERVICES, WindowsPath, generated_service_set};

use serde::{Deserialize, Serialize};
use sim_platform_core::{
    ContractProvenance, EvidenceLevel, FactPort, OpenSymbol, PlatformCard, ServiceOffer,
    stable_digest,
};
use std::collections::{BTreeMap, BTreeSet};

pub const NATIVE_ABI_ENTRY: &str = "sim_native_abi_v1";
pub const TARGET: &str = "x86_64-pc-windows-msvc";
pub const CAPSULE_CARD_NAME: &str = "sim.platform-capsule.toml";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowsProfile {
    pub preopened_dirs: BTreeMap<String, Vec<u16>>,
    pub grants: BTreeSet<String>,
    pub max_processes: u32,
    pub max_output_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lifecycle {
    Ready,
    Active,
    Suspended,
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeRequest {
    Launch,
    Load,
    Connect,
    Activate,
    Clipboard,
    Notify,
    Audio,
    Midi,
    Compute,
    PermissionStatus,
    PermissionRequest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapsuleError {
    MissingCapability(String),
    UnknownPreopen(String),
    InvalidBudget,
    Suspended,
    Stopped,
    Native(&'static str),
}

pub trait NativeServices {
    /// Dispatch one already-authorized request through private Win32/WinRT mechanics.
    /// # Errors
    /// Returns a sanitized failure and never exposes a native handle.
    fn dispatch(&mut self, request: NativeRequest) -> Result<(), &'static str>;
    fn cancel_process_tree(&mut self);
    fn cleanup(&mut self);
}

pub struct WindowsCapsule<N: NativeServices> {
    native: N,
    profile: WindowsProfile,
    lifecycle: Lifecycle,
}

impl<N: NativeServices> WindowsCapsule<N> {
    /// Construct a capsule from explicit mounts, authority, and budgets.
    /// # Errors
    /// Refuses unbounded process or output profiles.
    pub fn new(native: N, profile: WindowsProfile) -> Result<Self, CapsuleError> {
        if profile.max_processes == 0
            || profile.max_processes > 64
            || profile.max_output_bytes == 0
            || profile.max_output_bytes > 16 * 1024 * 1024
        {
            return Err(CapsuleError::InvalidBudget);
        }
        Ok(Self {
            native,
            profile,
            lifecycle: Lifecycle::Ready,
        })
    }
    #[must_use]
    pub const fn lifecycle(&self) -> Lifecycle {
        self.lifecycle
    }
    /// Resolve a named preopen and normalize it only at the Table/Dir boundary.
    /// # Errors
    /// Refuses unknown roots and malformed relative paths.
    pub fn resolve_preopen(
        &self,
        name: &str,
        relative: &[u16],
    ) -> Result<WindowsPath, CapsuleError> {
        if !self.profile.preopened_dirs.contains_key(name) {
            return Err(CapsuleError::UnknownPreopen(name.into()));
        }
        WindowsPath::from_table_units(relative)
            .map_err(|_| CapsuleError::Native("invalid table path"))
    }
    /// Dispatch a capability-gated service request.
    /// # Errors
    /// Refuses missing authority or inactive lifecycle before native dispatch.
    pub fn request(&mut self, service: &str, request: NativeRequest) -> Result<(), CapsuleError> {
        self.ensure_running()?;
        let required = format!("platform/{service}");
        if !self.profile.grants.contains(&required) {
            return Err(CapsuleError::MissingCapability(required));
        }
        self.native.dispatch(request).map_err(CapsuleError::Native)
    }
    /// Enter active lifecycle through the ordinary activation service.
    /// # Errors
    /// Applies the same capability gate as every other request.
    pub fn activate(&mut self) -> Result<(), CapsuleError> {
        self.request("activation", NativeRequest::Activate)?;
        self.lifecycle = Lifecycle::Active;
        Ok(())
    }
    /// Cancel the entire bounded job-object process tree.
    /// # Errors
    /// Refuses inactive lifecycle.
    pub fn cancel_process_tree(&mut self) -> Result<(), CapsuleError> {
        self.ensure_running()?;
        self.native.cancel_process_tree();
        Ok(())
    }
    /// Suspends an active capsule without discarding its bounded state.
    ///
    /// # Errors
    ///
    /// Returns [`CapsuleError::Stopped`] after terminal shutdown.
    pub fn suspend(&mut self) -> Result<(), CapsuleError> {
        self.ensure_running()?;
        self.lifecycle = Lifecycle::Suspended;
        Ok(())
    }
    /// Resumes a suspended capsule into its ready lifecycle.
    ///
    /// # Errors
    ///
    /// Returns [`CapsuleError::Stopped`] after terminal shutdown.
    pub fn resume(&mut self) -> Result<(), CapsuleError> {
        if self.lifecycle == Lifecycle::Suspended {
            self.lifecycle = Lifecycle::Ready;
            Ok(())
        } else {
            self.ensure_running()
        }
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
impl<N: NativeServices> Drop for WindowsCapsule<N> {
    fn drop(&mut self) {
        self.stop();
    }
}

#[must_use]
pub fn platform_card() -> PlatformCard {
    PlatformCard {
        schema: OpenSymbol("platform/card-v1".into()),
        site: OpenSymbol("platform/site/windows".into()),
        services: SERVICES
            .iter()
            .map(|binding| ServiceOffer {
                service: OpenSymbol(format!("platform/{}", binding.service)),
                port: FactPort::MachineLimits,
                evidence: EvidenceLevel::Attested,
            })
            .collect(),
        provenance: ContractProvenance {
            contract: OpenSymbol("contract/windows-capsule-v1".into()),
            content_digest: stable_digest(generated_service_set().as_bytes()),
            issuer: OpenSymbol("issuer/sim-platform".into()),
        },
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct PackageFile {
    pub path: String,
    pub content_digest: String,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct UnsignedPackage {
    pub schema: String,
    pub target: String,
    pub identity: String,
    pub files: Vec<PackageFile>,
}

/// Assemble the deterministic package beside its Card, permissions, shell, and rind.
#[must_use]
pub fn build_unsigned_package(rind: Vec<u8>, capsule: Vec<u8>) -> UnsignedPackage {
    let artifact = "sim-platform-windows.dll";
    let card = format!(
        "schema = \"sim.platform-capsule/v1\"\nprovider = \"platform/site/windows\"\nservices = [{}]\nshells = [\"shell/windows-app\"]\nloader_kinds = [\"loader/native-v1\"]\n",
        SERVICES
            .iter()
            .map(|s| format!("\"platform/{}\"", s.service))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let descriptor = format!(
        "schema = \"sim.platform-bundle/v1\"\ncapsule = \"{CAPSULE_CARD_NAME}\"\nartifact = \"{artifact}\"\nloader = \"loader/native-v1\"\nartifact_content = \"{}\"\nentry = \"{NATIVE_ABI_ENTRY}\"\nshell = \"shell/windows-app\"\n",
        stable_digest(&capsule)
    );
    let mut files = vec![
        file("sim.exe", rind),
        file(artifact, capsule),
        file(CAPSULE_CARD_NAME, card.into_bytes()),
        file("sim.platform-bundle.toml", descriptor.into_bytes()),
        file(
            "AppxManifest.xml",
            include_bytes!("../package/AppxManifest.xml").to_vec(),
        ),
    ];
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let identity = stable_digest(
        &files
            .iter()
            .flat_map(|f| f.content_digest.bytes())
            .collect::<Vec<_>>(),
    );
    UnsignedPackage {
        schema: "sim.platform-package/v1".into(),
        target: TARGET.into(),
        identity,
        files,
    }
}
fn file(path: impl Into<String>, bytes: Vec<u8>) -> PackageFile {
    PackageFile {
        path: path.into(),
        content_digest: stable_digest(&bytes),
        bytes,
    }
}
