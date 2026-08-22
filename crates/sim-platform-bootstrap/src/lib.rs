#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! The sole, bounded host rind used to admit the first platform capsule.
//!
//! Callers supply every input, including the executable location and kernel
//! seed. This crate never consults process arguments, environment variables,
//! the current directory, a registry, or a target-platform detector.

use sha2::{Digest, Sha256};
use sim_kernel::{Cx, Lib, LibLoader};
use sim_platform_core::{
    BundleManifest, CapsuleManifest, parse_bundle, parse_capsule, validate_bundle,
};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fmt, fs,
    path::{Path, PathBuf},
};

/// Exact descriptor name admitted beside the caller-supplied executable.
pub const BUNDLE_DESCRIPTOR_NAME: &str = "sim.platform-bundle.toml";
/// Hard ceiling for the complete descriptor.
pub const MAX_DESCRIPTOR_BYTES: u64 = 16 * 1024;
/// Hard ceiling for the capsule card.
pub const MAX_CAPSULE_CARD_BYTES: u64 = 16 * 1024;
/// Hard ceiling for the one native artifact.
pub const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
/// Hard ceiling for arguments and preopened roots.
pub const MAX_ENVELOPE_ITEMS: usize = 256;
/// Hard ceiling for any individual argument or path.
pub const MAX_ENVELOPE_ITEM_BYTES: usize = 16 * 1024;

/// Owned stdio data. The host surface may replace output buffers after boot.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BootstrapStdio {
    /// Complete supplied standard input.
    pub stdin: Vec<u8>,
    /// Initial standard-output frame, normally empty.
    pub stdout: Vec<u8>,
    /// Initial standard-error frame, normally empty.
    pub stderr: Vec<u8>,
}

/// Configuration roots resolved and supplied by the embedding host.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BootstrapConfigRoots {
    /// Central user configuration root, when the host supplies one.
    pub home: Option<PathBuf>,
    /// Working configuration root.
    pub work: PathBuf,
}

/// All data crossing from the tiny host rind into the bootloader.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootstrapEnvelope {
    /// Owned process argument frames.
    pub argv: Vec<OsString>,
    /// Owned stdio frames.
    pub stdio: BootstrapStdio,
    /// Content identity from the admitted bundle descriptor.
    pub bundle_identity: String,
    /// The admitted capsule's shaped card.
    pub capsule_card: CapsuleManifest,
    /// Roots explicitly preopened by the host, with no implicit current root.
    pub preopened_roots: Vec<PathBuf>,
    /// Configuration roots explicitly resolved by the host.
    pub config_roots: BootstrapConfigRoots,
    /// Caller-owned deterministic kernel handle seed.
    pub kernel_seed: u64,
}

impl BootstrapEnvelope {
    /// Validates the bounded, owned transport shape.
    ///
    /// # Errors
    /// Refuses oversized collections, frames, arguments, paths, or identity.
    pub fn validate(&self) -> Result<(), BootstrapError> {
        if self.argv.len() > MAX_ENVELOPE_ITEMS || self.preopened_roots.len() > MAX_ENVELOPE_ITEMS {
            return Err(BootstrapError::EnvelopeBound(
                "too many argv or preopened-root items",
            ));
        }
        if self.bundle_identity.is_empty() || self.bundle_identity.len() > MAX_ENVELOPE_ITEM_BYTES {
            return Err(BootstrapError::EnvelopeBound("invalid bundle identity"));
        }
        for arg in &self.argv {
            if arg.as_encoded_bytes().len() > MAX_ENVELOPE_ITEM_BYTES {
                return Err(BootstrapError::EnvelopeBound("argument is too large"));
            }
        }
        for root in self
            .preopened_roots
            .iter()
            .chain(self.config_roots.home.iter())
            .chain(std::iter::once(&self.config_roots.work))
        {
            if root.as_os_str().as_encoded_bytes().len() > MAX_ENVELOPE_ITEM_BYTES {
                return Err(BootstrapError::EnvelopeBound("preopened root is too large"));
            }
        }
        for frame in [&self.stdio.stdin, &self.stdio.stdout, &self.stdio.stderr] {
            if u64::try_from(frame.len()).unwrap_or(u64::MAX) > MAX_ARTIFACT_BYTES {
                return Err(BootstrapError::EnvelopeBound("stdio frame is too large"));
            }
        }
        Ok(())
    }
}

/// Successful admission: one owned envelope and one already-instantiated lib.
pub struct BootstrappedCapsule {
    /// Pure input handed onward to the bootloader.
    pub envelope: BootstrapEnvelope,
    /// Capsule loaded through the existing native ABI loader.
    pub capsule: Box<dyn Lib>,
}

/// Policy supplied by the embedding host; there are no ambient defaults.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootstrapPolicy {
    /// Exact services the first capsule may claim.
    pub allowed_services: BTreeSet<String>,
}

/// Fail-closed bootstrap errors.
#[derive(Debug)]
pub enum BootstrapError {
    /// An exact required adjacent file is absent or unreadable.
    Read {
        /// Exact attempted path.
        path: PathBuf,
        /// Host error detail.
        detail: String,
    },
    /// A bounded input exceeded its declared ceiling.
    EnvelopeBound(&'static str),
    /// A descriptor named anything except one adjacent plain filename.
    InvalidAdjacentName(String),
    /// Parsed platform metadata was invalid.
    InvalidManifest(String),
    /// Artifact content did not match the descriptor identity.
    ContentMismatch {
        /// Descriptor claim.
        expected: String,
        /// Computed content identity.
        actual: String,
    },
    /// The capsule requested authority outside the supplied policy.
    OverCapable(String),
    /// Existing native ABI loading rejected the exact artifact.
    Native(String),
}

impl fmt::Display for BootstrapError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{self:?}")
    }
}
impl std::error::Error for BootstrapError {}

/// Admit exactly one adjacent descriptor, capsule card, and native artifact.
///
/// `executable` is data supplied by the caller; this function deliberately
/// never discovers it. `loader` is normally the existing native dylib loader,
/// preserving the one `sim_native_abi_v1` implementation.
///
/// # Errors
/// Refuses missing, changed, malformed, wrong-ABI, duplicate, or over-capable
/// input and never tries an alternate location or loader.
pub fn bootstrap<L: LibLoader>(
    executable: &Path,
    mut envelope: BootstrapEnvelope,
    policy: &BootstrapPolicy,
    cx: &mut Cx,
    loader: &L,
) -> Result<BootstrappedCapsule, BootstrapError> {
    envelope.validate()?;
    let adjacent = executable
        .parent()
        .ok_or_else(|| BootstrapError::InvalidAdjacentName(executable.display().to_string()))?;
    let bundle_path = adjacent.join(BUNDLE_DESCRIPTOR_NAME);
    let bundle_text = read_bounded(&bundle_path, MAX_DESCRIPTOR_BYTES)?;
    let bundle: BundleManifest = parse_bundle(
        std::str::from_utf8(&bundle_text)
            .map_err(|error| BootstrapError::InvalidManifest(error.to_string()))?,
    )
    .map_err(|error| BootstrapError::InvalidManifest(error.to_string()))?;
    require_plain_name(&bundle.capsule)?;
    require_plain_name(&bundle.artifact)?;
    let capsule_path = adjacent.join(&bundle.capsule);
    let capsule_text = read_bounded(&capsule_path, MAX_CAPSULE_CARD_BYTES)?;
    let capsule = parse_capsule(
        std::str::from_utf8(&capsule_text)
            .map_err(|error| BootstrapError::InvalidManifest(error.to_string()))?,
    )
    .map_err(|error| BootstrapError::InvalidManifest(error.to_string()))?;
    validate_bundle(&bundle, &capsule)
        .map_err(|error| BootstrapError::InvalidManifest(error.to_string()))?;
    let providers: BTreeSet<_> = std::iter::once(&capsule.provider).collect();
    if providers.len() != 1 {
        return Err(BootstrapError::InvalidManifest(
            "duplicate capsule provider".into(),
        ));
    }
    if let Some(service) = capsule
        .services
        .iter()
        .find(|service| !policy.allowed_services.contains(*service))
    {
        return Err(BootstrapError::OverCapable(service.clone()));
    }
    let artifact_path = adjacent.join(&bundle.artifact);
    let artifact = read_bounded(&artifact_path, MAX_ARTIFACT_BYTES)?;
    let actual = format!("sha256:{:x}", Sha256::digest(&artifact));
    if actual != bundle.artifact_content {
        return Err(BootstrapError::ContentMismatch {
            expected: bundle.artifact_content,
            actual,
        });
    }
    envelope.bundle_identity = actual;
    envelope.capsule_card = capsule;
    let source = sim_run_loaders::path_source(artifact_path);
    if !loader.can_load(&source) {
        return Err(BootstrapError::Native(
            "exact artifact is not accepted by the native loader".into(),
        ));
    }
    let capsule = loader
        .load(cx, source)
        .map_err(|error| BootstrapError::Native(error.to_string()))?;
    Ok(BootstrappedCapsule { envelope, capsule })
}

fn require_plain_name(name: &str) -> Result<(), BootstrapError> {
    let path = Path::new(name);
    if name.is_empty() || path.is_absolute() || path.components().count() != 1 {
        return Err(BootstrapError::InvalidAdjacentName(name.to_owned()));
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, BootstrapError> {
    let metadata = fs::metadata(path).map_err(|error| BootstrapError::Read {
        path: path.to_owned(),
        detail: error.to_string(),
    })?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(BootstrapError::EnvelopeBound(
            "adjacent input is not a bounded regular file",
        ));
    }
    fs::read(path).map_err(|error| BootstrapError::Read {
        path: path.to_owned(),
        detail: error.to_string(),
    })
}

#[cfg(test)]
mod tests;
