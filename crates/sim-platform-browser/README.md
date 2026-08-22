# sim-platform-browser

This capsule realizes capability-scoped browser APIs for a SIM Wasm library.
Its Card is derived from APIs detected by the shell and deliberately omits
processes, native loading, ambient filesystem access, and presentation.
`sim-web` remains the sole owner of DOM/Canvas/WebGPU-facing Surface behavior.

One named-call export accepts canonical SIM binary frames. The JavaScript shell
only detects APIs, copies bytes, performs the requested host call, and returns
bytes; operation selection and bounds remain in Rust.
