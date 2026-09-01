#![forbid(unsafe_code)]
//! Host-owned Halo proxy over the official SDK transport model.
//!
//! The crate owns application framing and lifecycle only. Consumers continue to
//! use the `GLASSES_8` Device/Stream and Surface contracts; the device runs no SIM
//! evaluator or routing policy.

mod halo;

pub use halo::*;
