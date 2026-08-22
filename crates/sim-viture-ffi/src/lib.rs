//! Unsafe-isolated dynamic loader for VITURE glasses SDK entry points.
//!
//! The crate owns dynamic discovery and vendor-symbol calls. Its public surface
//! exposes safe handles, status values, and errors so the stream-host provider
//! does not carry raw SDK pointers.

#![deny(missing_docs)]

pub mod carina;
pub mod dynload;
pub mod legacy;
pub mod physical;
pub mod stub;

pub use carina::{CarinaPose, VitureHandle};
pub use dynload::{
    SdkCandidate, SdkDiscoverySource, VITURE_SDK_PATH_ENV, VITURE_USB_VID, VitureError, VitureLib,
    VitureResult, VitureSdkDiscovery, VitureStatus,
};
pub use legacy::LegacyImuRate;
pub use physical::{
    DevicePhysicalPort, DevicePhysicalSession, ModeledViturePort, PhysicalDeviceCommand,
    PhysicalDeviceSample, PhysicalTransportStats, ViturePhysicalPort,
};
pub use stub::unsupported_viture_lib;

#[cfg(test)]
mod tests;
