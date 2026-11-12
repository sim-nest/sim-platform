// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Finite cgroup v2 accounting values and the protected-tree ceiling.
//!
//! These types record a selected limit. They do not open a cgroup, write a
//! controller file, or start a process.

use std::io;

/// Hard maximum of directories plus files in one pinned read-only tree.
pub const MAXIMUM_PINNED_BOOT_TREE_ENTRIES: usize = 4096;

/// Hard maximum of logical file bytes in one pinned read-only tree.
pub const MAXIMUM_PINNED_BOOT_TREE_BYTES: usize = 256 * 1024 * 1024;

/// 4 KiB base page of the 64-bit memory controller.
const BASE_PAGE_BYTES: u64 = 4096;

/// `LONG_MAX` as a byte count. `PAGE_COUNTER_MAX` is this value divided by
/// the base page, and a counter equal to `PAGE_COUNTER_MAX` is unlimited.
const SIGNED_LONG_MAX: u64 = (1_u64 << 63) - 1;

/// Largest memory or swap byte count that remains a finite page limit.
///
/// The controller stores `min(bytes / PAGE_SIZE, PAGE_COUNTER_MAX)` pages and
/// reports `PAGE_COUNTER_MAX` as `max`.
const MAXIMUM_FINITE_MEMORY_BYTES: u64 = (SIGNED_LONG_MAX / BASE_PAGE_BYTES) * BASE_PAGE_BYTES - 1;

/// Exact Linux controller values, in kernel accounting units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CgroupV2Limits {
    /// Aggregate CPU microseconds available per scheduling period.
    pub cpu_quota_us: u64,
    /// Scheduling period in microseconds, not a total CPU-time budget.
    pub cpu_period_us: u64,
    /// Extra CPU burst allowance in microseconds.
    pub cpu_burst_us: u64,
    /// Charged-memory hard-limit setting, with kernel reclaim semantics.
    pub memory_bytes: u64,
    /// Separate swap hard-limit setting; zero prohibits swap.
    pub swap_bytes: u64,
    /// Descendant task hard-limit setting; threads count separately.
    pub tasks: u64,
    /// Whether the kernel treats the group as an indivisible workload.
    pub group_oom_kill: bool,
}

impl CgroupV2Limits {
    /// Checks one named child selection against its parent without native I/O.
    ///
    /// # Errors
    /// Refuses an empty or non-lowercase name, a non-finite limit, an
    /// unlimited memory or swap selection, burst, a disabled group workload,
    /// or a child that exceeds its parent.
    pub fn admit_child(name: &str, child: Self, parent: Self) -> io::Result<()> {
        finite(name, child)?;
        finite("parent", parent)?;
        if child.memory_bytes > parent.memory_bytes
            || child.swap_bytes > parent.swap_bytes
            || child.tasks > parent.tasks
            || u128::from(child.cpu_quota_us) * u128::from(parent.cpu_period_us)
                > u128::from(parent.cpu_quota_us) * u128::from(child.cpu_period_us)
        {
            return Err(invalid("child limits exceed the parent selection"));
        }
        Ok(())
    }
}

fn finite(name: &str, limits: CgroupV2Limits) -> io::Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || limits.cpu_quota_us == 0
        || !(1_000..=1_000_000).contains(&limits.cpu_period_us)
        || limits.cpu_burst_us != 0
        || limits.memory_bytes == 0
        || limits.memory_bytes > MAXIMUM_FINITE_MEMORY_BYTES
        || limits.swap_bytes > MAXIMUM_FINITE_MEMORY_BYTES
        || limits.tasks == 0
        || !limits.group_oom_kill
    {
        return Err(invalid("cgroup selection is not finite"));
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::{CgroupV2Limits, MAXIMUM_PINNED_BOOT_TREE_BYTES, MAXIMUM_PINNED_BOOT_TREE_ENTRIES};

    fn limits() -> CgroupV2Limits {
        CgroupV2Limits {
            cpu_quota_us: 100_000,
            cpu_period_us: 100_000,
            cpu_burst_us: 0,
            memory_bytes: 268_435_456,
            swap_bytes: 0,
            tasks: 32,
            group_oom_kill: true,
        }
    }

    #[test]
    fn child_selection_refuses_nonfinite_limits_and_a_child_over_its_parent() {
        CgroupV2Limits::admit_child("query-1", limits(), limits()).unwrap();
        let mut burst = limits();
        burst.cpu_burst_us = 1;
        assert!(CgroupV2Limits::admit_child("query-1", burst, limits()).is_err());
        let mut child = limits();
        child.memory_bytes = limits().memory_bytes + 1;
        assert!(CgroupV2Limits::admit_child("query-1", child, limits()).is_err());
        assert!(CgroupV2Limits::admit_child("Query", limits(), limits()).is_err());

        let mut unlimited = limits();
        unlimited.memory_bytes = u64::MAX;
        assert!(CgroupV2Limits::admit_child("query-1", unlimited, unlimited).is_err());
        unlimited = limits();
        unlimited.swap_bytes = u64::MAX;
        assert!(CgroupV2Limits::admit_child("query-1", unlimited, unlimited).is_err());

        let finite_max = ((1_u64 << 63) - 1) / 4096 * 4096 - 1;
        assert_eq!(finite_max, 9_223_372_036_854_771_711);
        let mut edge = limits();
        edge.memory_bytes = finite_max;
        edge.swap_bytes = finite_max;
        CgroupV2Limits::admit_child("query-1", edge, edge).unwrap();
        edge.memory_bytes = finite_max + 1;
        assert!(CgroupV2Limits::admit_child("query-1", edge, edge).is_err());
        edge.memory_bytes = finite_max;
        edge.swap_bytes = finite_max + 1;
        assert!(CgroupV2Limits::admit_child("query-1", edge, edge).is_err());
    }

    #[test]
    fn protected_tree_ceiling_is_the_shared_bound() {
        assert_eq!(MAXIMUM_PINNED_BOOT_TREE_ENTRIES, 4096);
        assert_eq!(MAXIMUM_PINNED_BOOT_TREE_BYTES, 256 * 1024 * 1024);
    }
}
