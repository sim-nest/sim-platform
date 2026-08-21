#![forbid(unsafe_code)]
#![allow(deprecated)]
#![deny(missing_docs)]
#![allow(
    clippy::ignored_unit_patterns,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::needless_continue,
    clippy::needless_pass_by_value,
    clippy::single_match_else,
    clippy::unused_self
)]
//! Filesystem-backed table store for SIM.
//!
//! This crate exposes a host directory as a SIM table: each table key maps to a
//! file, and nested tables map to subdirectories. Reads are gated by `fs/read`;
//! writes, deletes, and directory mutation are gated by `fs/write`.
//! Compatibility `table.fs.*` aliases are accepted by the gates. Values are encoded through
//! the configured codec. With the optional format features enabled, recognized
//! extensions (for example `.mid`, `.music`, `.tone`, `.scl`, `.ly`) round-trip
//! through their domain shapes. `FsDir::edit` patches string leaves with
//! `fs/read`, `fs/write`, and `edit`, then writes by same-directory temp file
//! and rename.
//! `FsDir::find_grep` and `FsDir::find_glob` search the directory read-only
//! with `fs/read` and `find`.

mod adapters;
pub mod capabilities;
mod citizen;
mod dir_edit;
mod find;
mod fs_dir;
mod roadmap11;

pub use adapters::{MemoryHostDirPort, UbuntuHostDirPort};
pub use capabilities::{
    table_fs_capability, table_fs_edit_capability, table_fs_find_capability,
    table_fs_mkdir_capability, table_fs_read_capability, table_fs_rmdir_capability,
    table_fs_write_capability,
};
pub use citizen::{FsDirDescriptor, fs_dir_class_symbol};
pub use find::{FindGlobResult, FindGrepResult, FindMatch};
pub use fs_dir::{FsDir, install_fs_dir_lib};

/// Cookbook recipes for this lib, embedded at build time.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

#[cfg(test)]
mod tests;
