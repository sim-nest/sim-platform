# sim-platform-ubuntu-pc

The reference native capsule. It publishes distinct desktop and headless Cards
for Ubuntu on x86_64 and aarch64; architecture remains descriptive Card data.

`LocalCheckAdapter` is the qualified local implementation-check boundary. Boot
configuration supplies exact `CommandSpec` allowlist entries, opaque native
resource mappings, `UbuntuProcess`, and registered `BwrapLauncher` instances.
Packet tooling submits only a `LocalCheckRequest` containing the installed
command identity. The adapter wraps execution in the durable operation
lifecycle, clears ambient environment, keeps networking absent in bwrap,
accounts for writable roots, removes owned scratch contents, and reconciles the
declared output contract through a distinct observer.
