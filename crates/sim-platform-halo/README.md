# sim-platform-halo

This capsule hosts the official Brilliant Labs Halo SDK connection in the
existing Android, iOS, or browser capsule. It defines one bounded application
codec shared by those transports, normalizes input for the delivered
`GLASSES_8` Device/Stream profile, and accepts output from `Surface`.

The Lua program under `shell/halo` is deliberately small. It forwards lifecycle,
button, and sensor events and applies bounded display operations. It contains no
SIM runtime, evaluator, policy, persistence, or glasses-specific application API.

The committed evidence is model and cross-build evidence only. There is no
registered Halo resource, so this crate makes no physical-device claim.
