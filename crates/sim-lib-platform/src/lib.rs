#![forbid(unsafe_code)]
//! Pure API facade. Host bindings arrive only through validated future capsules.

pub use sim_platform_core::{
    BundleManifest, CapsuleManifest, PackageRole, ValidationContext, ValidationError, parse_bundle,
    parse_capsule, validate_bundle, validate_capsules,
};
