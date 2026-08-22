//! Dynamic SDK discovery and library loading.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use libloading::{Library, Symbol};

/// Environment variable used for an explicit SDK library path.
pub const VITURE_SDK_PATH_ENV: &str = "VITURE_SDK_PATH";

/// VITURE's USB vendor id as documented by the SDK.
pub const VITURE_USB_VID: &str = "0x35CA";

const VITURE_USB_VID_LOWER: &str = "35ca";

/// Result type used by this crate.
pub type VitureResult<T> = Result<T, VitureError>;

/// Error returned by VITURE SDK discovery and calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VitureError {
    /// No SDK was configured or loadable on this host.
    Unsupported,
    /// A dynamic library failed to load.
    Load {
        /// Path or linker name that was attempted.
        target: String,
        /// Loader diagnostic.
        message: String,
    },
    /// A required SDK symbol was absent.
    Symbol {
        /// Symbol name requested from the dynamic library.
        name: &'static str,
        /// Loader diagnostic.
        message: String,
    },
    /// The SDK returned a null provider handle.
    NullHandle,
    /// The SDK returned a non-zero status code.
    Sdk(i32),
}

impl fmt::Display for VitureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("VITURE SDK is unsupported on this host"),
            Self::Load { target, message } => {
                write!(f, "failed to load VITURE SDK {target}: {message}")
            }
            Self::Symbol { name, message } => {
                write!(f, "missing VITURE SDK symbol {name}: {message}")
            }
            Self::NullHandle => f.write_str("VITURE SDK returned a null provider handle"),
            Self::Sdk(code) => write!(f, "VITURE SDK returned status {code}"),
        }
    }
}

impl std::error::Error for VitureError {}

/// Successful status returned by a VITURE SDK call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VitureStatus {
    code: i32,
}

impl VitureStatus {
    /// Converts a raw SDK status code into a safe status value.
    pub fn from_code(code: i32) -> VitureResult<Self> {
        if code == 0 {
            Ok(Self { code })
        } else {
            Err(VitureError::Sdk(code))
        }
    }

    /// Returns the raw successful status code.
    pub const fn code(self) -> i32 {
        self.code
    }
}

/// Source that produced a discovery candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdkDiscoverySource {
    /// Explicit path supplied by configuration or environment.
    ConfiguredPath,
    /// Library name passed to the platform dynamic linker.
    RuntimeLinker,
    /// Linux sysfs USB device whose vendor id matches VITURE.
    LinuxSysfsVid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SdkLoadTarget {
    Path(PathBuf),
    LinkerName(OsString),
}

/// One SDK discovery candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdkCandidate {
    source: SdkDiscoverySource,
    target: SdkLoadTarget,
}

impl SdkCandidate {
    fn configured(path: PathBuf) -> Self {
        Self {
            source: SdkDiscoverySource::ConfiguredPath,
            target: SdkLoadTarget::Path(path),
        }
    }

    fn runtime(name: OsString) -> Self {
        Self {
            source: SdkDiscoverySource::RuntimeLinker,
            target: SdkLoadTarget::LinkerName(name),
        }
    }

    fn linux_sysfs_vid(path: PathBuf) -> Self {
        Self {
            source: SdkDiscoverySource::LinuxSysfsVid,
            target: SdkLoadTarget::Path(path),
        }
    }

    /// Returns where this candidate came from.
    pub const fn source(&self) -> SdkDiscoverySource {
        self.source
    }

    /// Returns the explicit library path when this candidate has one.
    pub fn library_path(&self) -> Option<&Path> {
        match &self.target {
            SdkLoadTarget::Path(path) if self.source != SdkDiscoverySource::LinuxSysfsVid => {
                Some(path)
            }
            _ => None,
        }
    }

    /// Returns the runtime linker name when this candidate has one.
    pub fn linker_name(&self) -> Option<&OsStr> {
        match &self.target {
            SdkLoadTarget::LinkerName(name) => Some(name.as_os_str()),
            _ => None,
        }
    }

    /// Returns the matching Linux sysfs device path when this candidate has one.
    pub fn sysfs_device(&self) -> Option<&Path> {
        match &self.target {
            SdkLoadTarget::Path(path) if self.source == SdkDiscoverySource::LinuxSysfsVid => {
                Some(path)
            }
            _ => None,
        }
    }
}

/// SDK discovery configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VitureSdkDiscovery {
    configured_path: Option<PathBuf>,
    runtime_names: Vec<OsString>,
    sysfs_root: PathBuf,
}

impl Default for VitureSdkDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

impl VitureSdkDiscovery {
    /// Builds the default discovery plan.
    pub fn new() -> Self {
        Self {
            configured_path: std::env::var_os(VITURE_SDK_PATH_ENV).map(PathBuf::from),
            runtime_names: vec![
                OsString::from("libviture_sdk.so"),
                OsString::from("libviture_glasses_sdk.so"),
            ],
            sysfs_root: PathBuf::from("/sys/bus/usb/devices"),
        }
    }

    /// Sets an explicit SDK library path.
    pub fn with_configured_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.configured_path = Some(path.into());
        self
    }

    /// Clears runtime-linker library-name candidates.
    pub fn without_runtime_names(mut self) -> Self {
        self.runtime_names.clear();
        self
    }

    /// Adds a runtime-linker library-name candidate.
    pub fn with_runtime_name(mut self, name: impl Into<OsString>) -> Self {
        self.runtime_names.push(name.into());
        self
    }

    /// Sets the Linux sysfs root used for USB VID scans.
    pub fn with_sysfs_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.sysfs_root = path.into();
        self
    }

    /// Returns configured path, runtime-linker, and sysfs VID candidates.
    pub fn candidates(&self) -> Vec<SdkCandidate> {
        let mut candidates = Vec::new();
        if let Some(path) = &self.configured_path {
            candidates.push(SdkCandidate::configured(path.clone()));
        }
        candidates.extend(
            self.runtime_names
                .iter()
                .cloned()
                .map(SdkCandidate::runtime),
        );
        candidates.extend(self.sysfs_vid_candidates());
        candidates
    }

    fn sysfs_vid_candidates(&self) -> Vec<SdkCandidate> {
        let Ok(entries) = fs::read_dir(&self.sysfs_root) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                let vendor = fs::read_to_string(path.join("idVendor")).ok()?;
                (vendor.trim().eq_ignore_ascii_case(VITURE_USB_VID_LOWER))
                    .then(|| SdkCandidate::linux_sysfs_vid(path))
            })
            .collect()
    }
}

/// Loaded VITURE SDK library or CI-safe unsupported stub.
#[derive(Clone)]
pub struct VitureLib {
    inner: VitureLibInner,
}

#[derive(Clone)]
enum VitureLibInner {
    Dynamic(Arc<Library>),
    Stub,
}

impl fmt::Debug for VitureLib {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner {
            VitureLibInner::Dynamic(_) => f.write_str("VitureLib::Dynamic(..)"),
            VitureLibInner::Stub => f.write_str("VitureLib::Stub"),
        }
    }
}

impl VitureLib {
    /// Builds the hardware-free unsupported stub.
    pub fn stub() -> Self {
        Self {
            inner: VitureLibInner::Stub,
        }
    }

    /// Loads a dynamic SDK library from an explicit path.
    pub fn load_path(path: impl AsRef<Path>) -> VitureResult<Self> {
        let path = path.as_ref();
        Self::load_os_str(path.as_os_str(), path.display().to_string())
    }

    /// Loads a dynamic SDK library through the platform runtime linker.
    pub fn load_linker_name(name: impl AsRef<OsStr>) -> VitureResult<Self> {
        let name = name.as_ref();
        Self::load_os_str(name, name.to_string_lossy().into_owned())
    }

    /// Attempts configured and runtime-linker candidates; returns a stub when no
    /// SDK can be loaded and no explicit configured path failed.
    pub fn discover(discovery: &VitureSdkDiscovery) -> VitureResult<Self> {
        let mut configured_error = None;
        for candidate in discovery.candidates() {
            match candidate.target {
                SdkLoadTarget::Path(path)
                    if candidate.source == SdkDiscoverySource::ConfiguredPath =>
                {
                    match Self::load_path(&path) {
                        Ok(lib) => return Ok(lib),
                        Err(err) => configured_error = Some(err),
                    }
                }
                SdkLoadTarget::LinkerName(name) => {
                    if let Ok(lib) = Self::load_linker_name(&name) {
                        return Ok(lib);
                    }
                }
                SdkLoadTarget::Path(_) => {}
            }
        }
        if let Some(err) = configured_error {
            Err(err)
        } else {
            Ok(Self::stub())
        }
    }

    pub(crate) fn dynamic_library(&self) -> VitureResult<&Arc<Library>> {
        match &self.inner {
            VitureLibInner::Dynamic(library) => Ok(library),
            VitureLibInner::Stub => Err(VitureError::Unsupported),
        }
    }

    pub(crate) fn symbol<'library, T>(
        library: &'library Library,
        name: &'static str,
        bytes: &'static [u8],
    ) -> VitureResult<Symbol<'library, T>> {
        // SAFETY: the caller supplies the expected SDK function pointer type for
        // this symbol. All calls are wrapped by safe methods that validate status
        // codes and never expose raw pointers.
        unsafe { library.get(bytes) }.map_err(|err| VitureError::Symbol {
            name,
            message: err.to_string(),
        })
    }

    fn load_os_str(name: &OsStr, label: String) -> VitureResult<Self> {
        // SAFETY: loading a dynamic library is unsafe because constructors may
        // run and symbol ABIs must match later calls. This crate is the isolated
        // boundary that contains that risk and exposes only safe wrappers.
        let library = unsafe { Library::new(name) }.map_err(|err| VitureError::Load {
            target: label,
            message: err.to_string(),
        })?;
        Ok(Self {
            inner: VitureLibInner::Dynamic(Arc::new(library)),
        })
    }
}
