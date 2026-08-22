# sim-platform-macos

The macOS capsule is the only package allowed to know Darwin frameworks,
entitlements, application-bundle layout, and native glue. Portable callers see
the existing platform and domain ports and the unchanged `sim_native_abi_v1`.

The service manifest in `src/manifest.rs` is authoritative. Permission status
is observational; only `platform/permission-request`, after its matching SIM
capability, may call a prompting native API. Bundle assembly is deterministic,
unsigned, and independent of developer signing identity.
