#![deny(unsafe_code)]
//! iOS AOT capsule using the unchanged SIM native byte-frame ABI.

mod ffi;

pub use ffi::{StaticAbiCapsule, sim_native_abi_v1};

mod ios;

pub use ios::*;
