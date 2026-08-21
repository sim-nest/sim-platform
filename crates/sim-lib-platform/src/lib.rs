#![forbid(unsafe_code)]
//! The closed, provider-neutral platform API.

mod runtime;

pub use runtime::{
    ACTIVE_PLATFORM_SITE, PlatformFunction, PlatformFunctionKind, PlatformLib,
    install_platform_lib, platform_function_symbols,
};

pub use sim_platform_core::{
    BundleManifest, CapsuleManifest, PackageRole, ValidationContext, ValidationError, parse_bundle,
    parse_capsule, validate_bundle, validate_capsules,
};
