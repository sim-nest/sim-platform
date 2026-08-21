//! Atomic text edits for filesystem-backed table leaves.

use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_storage_port::NeverCancel;

use crate::{
    FsDir,
    capabilities::{require_table_fs_edit, require_table_fs_read, require_table_fs_write},
};

impl FsDir {
    /// Applies an exact text replacement to a string leaf and atomically writes it.
    ///
    /// Requires `fs/read`, `fs/write`, and `edit`.
    pub fn edit(
        &self,
        cx: &mut Cx,
        key: Symbol,
        old: &str,
        new: &str,
        replace_all: bool,
    ) -> Result<()> {
        self.edit_text_leaf(cx, key, |text| apply_edit(text, old, new, replace_all))
    }

    /// Applies a 1-based inclusive line-range replacement to a string leaf.
    ///
    /// Requires `fs/read`, `fs/write`, and `edit`.
    pub fn edit_lines(
        &self,
        cx: &mut Cx,
        key: Symbol,
        start: usize,
        end: usize,
        new: &str,
    ) -> Result<()> {
        self.edit_text_leaf(cx, key, |text| apply_edit_lines(text, start, end, new))
    }

    fn edit_text_leaf<F>(&self, cx: &mut Cx, key: Symbol, edit: F) -> Result<()>
    where
        F: FnOnce(&str) -> Result<String>,
    {
        require_table_fs_read(cx)?;
        require_table_fs_write(cx)?;
        require_table_fs_edit(cx)?;

        let (path, ext, expr) = self.read_leaf_expr(cx, &key)?;
        let Expr::String(text) = expr else {
            return Err(Error::Eval(format!(
                "table/fs: dir/edit expects string leaf at {key}"
            )));
        };
        let edited = edit(&text)?;
        let bytes = FsDir::encode_leaf_expr(cx, ext, &Expr::String(edited))?;
        self.port
            .replace(&path, &bytes, &NeverCancel)
            .map_err(crate::fs_dir::port_error)
    }
}

fn apply_edit(text: &str, old: &str, new: &str, replace_all: bool) -> Result<String> {
    if old.is_empty() {
        return Err(Error::Eval("edit: old pattern is empty".to_owned()));
    }
    let matches = text.matches(old).count();
    match matches {
        0 => Err(Error::Eval(format!("edit: pattern not found: {old:?}"))),
        n if n > 1 && !replace_all => Err(Error::Eval(format!(
            "edit: pattern is not unique ({n} matches); pass replace_all"
        ))),
        _ if replace_all => Ok(text.replace(old, new)),
        _ => Ok(text.replacen(old, new, 1)),
    }
}

fn apply_edit_lines(text: &str, start: usize, end: usize, new: &str) -> Result<String> {
    if start == 0 {
        return Err(Error::Eval(
            "edit-lines: start must be at least 1".to_owned(),
        ));
    }
    if end < start {
        return Err(Error::Eval(
            "edit-lines: end must be greater than or equal to start".to_owned(),
        ));
    }
    let lines = text.split_inclusive('\n').collect::<Vec<_>>();
    if end > lines.len() {
        return Err(Error::Eval(format!(
            "edit-lines: range {start}..{end} exceeds {} line(s)",
            lines.len()
        )));
    }
    let mut edited = String::new();
    for line in &lines[..start - 1] {
        edited.push_str(line);
    }
    edited.push_str(new);
    for line in &lines[end..] {
        edited.push_str(line);
    }
    Ok(edited)
}
