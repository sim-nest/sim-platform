#![forbid(unsafe_code)]
//! Deterministic, host-free platform model and the standard platform site.

mod process;
pub use process::{ModelProcess, ModelProcessOutcome};

mod model;

pub use model::*;
