//! Capability names used by filesystem-backed tables.

use sim_kernel::{CapabilityName, Cx, Error, Result};

/// The capability gating filesystem-backed tables (`table.fs`).
pub fn table_fs_capability() -> CapabilityName {
    CapabilityName::new("table.fs")
}

/// The capability gating filesystem table reads (`fs/read`).
pub fn table_fs_read_capability() -> CapabilityName {
    CapabilityName::new("fs/read")
}

/// The capability gating filesystem table writes (`fs/write`).
pub fn table_fs_write_capability() -> CapabilityName {
    CapabilityName::new("fs/write")
}

/// The capability gating filesystem table directory creation (`fs/write`).
pub fn table_fs_mkdir_capability() -> CapabilityName {
    table_fs_write_capability()
}

/// The capability gating filesystem table directory removal (`fs/write`).
pub fn table_fs_rmdir_capability() -> CapabilityName {
    table_fs_write_capability()
}

/// The capability gating patch-only filesystem leaf edits (`edit`).
pub fn table_fs_edit_capability() -> CapabilityName {
    CapabilityName::new("edit")
}

/// The capability gating filesystem tree search (`find`).
pub fn table_fs_find_capability() -> CapabilityName {
    CapabilityName::new("find")
}

pub(crate) fn require_table_fs_read(cx: &Cx) -> Result<()> {
    require_with_aliases(cx, table_fs_read_capability(), fs_read_aliases())
}

pub(crate) fn require_table_fs_write(cx: &Cx) -> Result<()> {
    require_with_aliases(cx, table_fs_write_capability(), fs_write_aliases())
}

pub(crate) fn require_table_fs_edit(cx: &Cx) -> Result<()> {
    cx.require(&table_fs_edit_capability())
}

pub(crate) fn require_table_fs_find(cx: &Cx) -> Result<()> {
    cx.require(&table_fs_find_capability())
}

fn fs_read_aliases() -> &'static [&'static str] {
    &["table.fs.read", "stream.file.read", "file-read"]
}

fn fs_write_aliases() -> &'static [&'static str] {
    &[
        "table.fs.write",
        "table.fs.mkdir",
        "table.fs.rmdir",
        "stream.file.write",
        "file-write",
    ]
}

fn require_with_aliases(
    cx: &Cx,
    canonical: CapabilityName,
    aliases: &'static [&'static str],
) -> Result<()> {
    if cx.capabilities().contains(&canonical)
        || aliases
            .iter()
            .any(|alias| cx.capabilities().contains(&CapabilityName::new(*alias)))
    {
        Ok(())
    } else {
        Err(Error::CapabilityDenied {
            capability: canonical,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        table_fs_capability, table_fs_edit_capability, table_fs_find_capability,
        table_fs_mkdir_capability, table_fs_read_capability, table_fs_rmdir_capability,
        table_fs_write_capability,
    };

    #[test]
    fn capability_tokens_are_stable() {
        assert_eq!(table_fs_capability().as_str(), "table.fs");
        assert_eq!(table_fs_read_capability().as_str(), "fs/read");
        assert_eq!(table_fs_write_capability().as_str(), "fs/write");
        assert_eq!(table_fs_mkdir_capability().as_str(), "fs/write");
        assert_eq!(table_fs_rmdir_capability().as_str(), "fs/write");
        assert_eq!(table_fs_edit_capability().as_str(), "edit");
        assert_eq!(table_fs_find_capability().as_str(), "find");
    }
}
