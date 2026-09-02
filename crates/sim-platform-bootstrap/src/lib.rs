#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! The sole, bounded host rind used to admit the first platform capsule.
//!
//! Callers supply every input, including the executable location and kernel
//! seed. This crate never consults process arguments, environment variables,
//! the current directory, a registry, or a target-platform detector.

mod bootstrap;

pub use bootstrap::*;
