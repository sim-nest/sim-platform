# sim-viture-ffi

`sim-viture-ffi` is the unsafe-isolated VITURE SDK loading boundary for
the `sim-platform` capsule. The crate loads a local SDK dynamically, exposes a
safe `DevicePhysicalPort`, and returns a hardware-free unsupported result when
no SDK is available. Stream providers consume that port and never discover,
load, or probe vendor hardware themselves.

The stream-host workspace remains `unsafe_code = "forbid"`; this platform
package is the isolated exception that owns the dynamic-link boundary.
