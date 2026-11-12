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

/// `LONG_MAX`. A 64-bit page counter uses `LONG_MAX / PAGE_SIZE`, and that
/// page count is the unlimited `max` setting.
const SIGNED_LONG_MAX: u64 = (1_u64 << 63) - 1;

/// Base pages a 64-bit Linux kernel can select: 4 KiB, 16 KiB, and 64 KiB.
const SUPPORTED_BASE_PAGES: [u64; 3] = [4 * 1024, 16 * 1024, 64 * 1024];

const fn finite_page_bytes(page: u64) -> u64 {
    (SIGNED_LONG_MAX / page) * page - 1
}

/// Largest memory or swap byte count that stays finite on every supported page.
const MAXIMUM_FINITE_MEMORY_BYTES: u64 = {
    let mut ceiling = finite_page_bytes(SUPPORTED_BASE_PAGES[0]);
    let mut index = 1;
    while index < SUPPORTED_BASE_PAGES.len() {
        let finite = finite_page_bytes(SUPPORTED_BASE_PAGES[index]);
        if finite < ceiling {
            ceiling = finite;
        }
        index += 1;
    }
    ceiling
};

/// `min_bw_quota_period_us` and `max_bw_quota_period_us`, in microseconds.
const MINIMUM_CPU_USEC: u64 = 1_000;
const MAXIMUM_CPU_PERIOD_US: u64 = 1_000_000;

/// `MAX_BW` at `BW_SHIFT` 20. A quota above this is rejected by the scheduler.
const MAXIMUM_CPU_QUOTA_US: u64 = (1_u64 << (64 - 20)) - 1;

/// Standard 64-bit `PID_MAX_LIMIT`. Numeric `pids.max` must be below
/// `PID_MAX_LIMIT + 1`.
const MAXIMUM_FINITE_TASKS: u64 = 4 * 1024 * 1024;

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
    /// Refuses an empty or non-lowercase name, a non-finite limit, burst, a
    /// disabled group workload, or a child that exceeds its parent.
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
        || !(MINIMUM_CPU_USEC..=MAXIMUM_CPU_QUOTA_US).contains(&limits.cpu_quota_us)
        || !(MINIMUM_CPU_USEC..=MAXIMUM_CPU_PERIOD_US).contains(&limits.cpu_period_us)
        || limits.cpu_burst_us != 0
        || limits.memory_bytes == 0
        || limits.memory_bytes > MAXIMUM_FINITE_MEMORY_BYTES
        || limits.swap_bytes > MAXIMUM_FINITE_MEMORY_BYTES
        || limits.tasks == 0
        || limits.tasks > MAXIMUM_FINITE_TASKS
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

        let finite_max = ((1_u64 << 63) - 1) / 65_536 * 65_536 - 1;
        assert_eq!(finite_max, 9_223_372_036_854_710_271);
        let mut edge = limits();
        edge.memory_bytes = finite_max;
        edge.swap_bytes = finite_max;
        CgroupV2Limits::admit_child("query-1", edge, edge).unwrap();
        edge.memory_bytes = finite_max + 1;
        assert!(CgroupV2Limits::admit_child("query-1", edge, edge).is_err());
        edge.memory_bytes = finite_max;
        edge.swap_bytes = finite_max + 1;
        assert!(CgroupV2Limits::admit_child("query-1", edge, edge).is_err());
        let four_kib_only = ((1_u64 << 63) - 1) / 4096 * 4096 - 1;
        assert_eq!(four_kib_only, 9_223_372_036_854_771_711);
        edge.swap_bytes = finite_max;
        edge.memory_bytes = four_kib_only;
        assert!(CgroupV2Limits::admit_child("query-1", edge, edge).is_err());

        let mut quota = limits();
        quota.cpu_quota_us = 1;
        assert!(CgroupV2Limits::admit_child("query-1", quota, quota).is_err());
        quota.cpu_quota_us = 1_000;
        CgroupV2Limits::admit_child("query-1", quota, quota).unwrap();
        quota.cpu_quota_us = (1_u64 << 44) - 1;
        CgroupV2Limits::admit_child("query-1", quota, quota).unwrap();
        quota.cpu_quota_us = 1_u64 << 44;
        assert!(CgroupV2Limits::admit_child("query-1", quota, quota).is_err());

        let mut tasks = limits();
        tasks.tasks = 4 * 1024 * 1024;
        CgroupV2Limits::admit_child("query-1", tasks, tasks).unwrap();
        tasks.tasks = 4 * 1024 * 1024 + 1;
        assert!(CgroupV2Limits::admit_child("query-1", tasks, tasks).is_err());
        tasks.tasks = u64::MAX;
        assert!(CgroupV2Limits::admit_child("query-1", tasks, tasks).is_err());
    }

    #[test]
    fn protected_tree_ceiling_is_the_shared_bound() {
        assert_eq!(MAXIMUM_PINNED_BOOT_TREE_ENTRIES, 4096);
        assert_eq!(MAXIMUM_PINNED_BOOT_TREE_BYTES, 256 * 1024 * 1024);
    }
}
