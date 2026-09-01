#![forbid(unsafe_code)]
//! macOS capsule policy, packaging, and testable native membrane.

mod manifest;
pub use manifest::{PromptPolicy, SERVICES, ServiceBinding};

mod macos;

pub use macos::*;
