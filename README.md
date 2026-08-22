# sim-platform

`sim-platform` is the sole owner of SIM platform capsules and their bundle
descriptors. It begins with pure manifest contracts and fictional model data;
repository membership alone grants no host-binding authority.

The workspace contains the pure contract (`sim-platform-core`), deterministic
fictional examples (`sim-platform-model`), the loadable pure API
(`sim-lib-platform`), and the uniquely classified bootstrap rind
(`sim-platform-bootstrap`), and the relocated `sim-table-fs` package. Filesystem
Table/Dir policy uses the storage-owned `HostDirPort`; the package provides one
deterministic in-memory model and one root-confined Ubuntu realization behind
that shared boundary.

Physical device transport follows the same rule. `sim-viture-ffi` is the
Ubuntu VITURE capsule package: it owns SDK discovery, dynamic loading, USB
device-path probes, native handles, lifecycle, and a deterministic modeled
`DevicePhysicalPort`. Stream-host crates consume that port and retain all
device profiles and stream semantics.

See `docs/humans/README.md`, `docs/agents/README.md`, `docs/generated/README.md`,
and `docs/rustdoc/README.md` for the four documentation lanes.
