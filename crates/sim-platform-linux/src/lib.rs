#![forbid(unsafe_code)]
//! Bounded Linux mechanics. Native paths and portal calls cannot escape this package.

mod transport;
pub use transport::{LinuxDnsPort, LinuxIpcPort, LinuxSocketPort, bind_transport_services};

mod linux;

pub use linux::*;
