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

See `docs/humans/README.md`, `docs/agents/README.md`, `docs/generated/README.md`,
and `docs/rustdoc/README.md` for the four documentation lanes.
