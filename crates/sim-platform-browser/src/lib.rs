//! Browser/Wasm platform capsule.
//!
//! The capsule owns browser handle realization and lifecycle events. It does
//! not contain a DOM, canvas, WebGPU, render-tree, process, native-loader, or
//! ambient-filesystem abstraction; presentation remains in `sim-web`.

#![forbid(unsafe_code)]

#[cfg(target_arch = "wasm32")]
mod wasm;

mod browser;

pub use browser::*;
