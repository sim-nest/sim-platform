#![forbid(unsafe_code)]
//! The closed, provider-neutral platform API.

mod runtime;

pub use runtime::{
    ACTIVE_PLATFORM_SITE, PlatformFunction, PlatformFunctionKind, PlatformLib,
    install_platform_lib, platform_function_symbols,
};

pub use sim_platform_core::{
    Activation, BoundServices, BundleManifest, CapsuleManifest, ContractProvenance,
    ExecutionEvidence, FactPort, Lifecycle, OpenSymbol, PackageRole, PlatformCard,
    PlatformProviderAuthor, PlatformRecordError, PlatformRequest, RefusalKind, Requirement,
    RequirementBuilder, ResolutionReceipt, ResolutionRefusal, ServiceBinding, ServiceOffer,
    ValidationContext, ValidationError, parse_bundle, parse_capsule, platform_require,
    validate_bundle, validate_capsules,
};
