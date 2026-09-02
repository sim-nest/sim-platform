#![forbid(unsafe_code)]
//! Windows capsule policy, bounded native membrane, and package description.

mod manifest;
mod path;
pub use manifest::{SERVICES, ServiceBinding, generated_service_set};
pub use path::{PathError, WindowsPath};

mod windows;

pub use windows::*;
