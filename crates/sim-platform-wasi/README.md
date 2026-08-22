# sim-platform-wasi

Least-authority WASI component capsule. Artifact admission checks every import
against a closed declaration and maps it to an existing SIM port. Activation
receives named Table/Dir preopens, clocks, entropy, socket endpoints, lifecycle,
and capabilities solely through a bundle profile. No ambient host discovery is
available through the API.

The frame conformance API records modeled in-memory and real-runtime execution
as distinct evidence grades; an unavailable hosted runtime cannot be mistaken
for physical proof.
