// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![forbid(unsafe_code)]
//! Bounded Linux mechanics. Native paths and portal calls cannot escape this package.

mod cgroup_limits;
mod transport;
pub use cgroup_limits::{
    CgroupV2Limits, MAXIMUM_PINNED_BOOT_TREE_BYTES, MAXIMUM_PINNED_BOOT_TREE_ENTRIES,
};
pub use transport::{LinuxDnsPort, LinuxIpcPort, LinuxSocketPort, bind_transport_services};

mod linux;

pub use linux::*;
