#![forbid(unsafe_code)]
//! The closed, provider-neutral platform API.

mod runtime;

pub use runtime::{
    ACTIVE_PLATFORM_SITE, PlatformFunction, PlatformFunctionKind, PlatformLib,
    install_platform_lib, platform_function_symbols,
};

pub use sim_platform_core::{
    Activation, BoundServices, BundleComposition, BundleContent, BundleManifest, BundleRefusal,
    CapsuleArtifact, CapsuleAttestation, CapsuleManifest, ComposedBundle, ContractProvenance,
    ExecutionEvidence, FactPort, LibraryLoadPlan, Lifecycle, OpenSymbol, PackageRole, PlatformCard,
    PlatformProviderAuthor, PlatformRecordError, PlatformRequest, PlatformSupportRow,
    PureBootEnvelope, RefusalKind, Requirement, RequirementBuilder, ResolutionReceipt,
    ResolutionRefusal, ServiceBinding, ServiceOffer, ValidationContext, ValidationError,
    compose_bundle, parse_bundle, parse_capsule, platform_require, platform_support_matrix,
    validate_bundle, validate_capsules,
};
