//! Read-only grep and glob search over the storage port.

use crate::{
    FsDir,
    capabilities::{require_table_fs_find, require_table_fs_read},
    fs_dir::port_error,
    roadmap11::known_exts,
};
use globset::{GlobBuilder, GlobMatcher};
use regex::Regex;
use sim_kernel::{Cx, Error, Expr, Result};
use sim_storage_port::HostEntryKind;

/// One text match returned by [`FsDir::find_grep`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FindMatch {
    /// Relative portable path.
    pub path: String,
    /// One-based line number.
    pub line: u32,
    /// Matching line without its terminator.
    pub text: String,
}
/// Bounded grep result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FindGrepResult {
    /// Retained matches.
    pub matches: Vec<FindMatch>,
    /// Whether more matches existed.
    pub truncated: bool,
}
/// Bounded glob result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FindGlobResult {
    /// Retained paths.
    pub paths: Vec<String>,
    /// Whether more paths existed.
    pub truncated: bool,
}

impl FsDir {
    /// Regex-searches string leaves, bounded by `max`.
    pub fn find_grep(
        &self,
        cx: &mut Cx,
        pattern: &str,
        glob: Option<&str>,
        max: usize,
    ) -> Result<FindGrepResult> {
        require_table_fs_read(cx)?;
        require_table_fs_find(cx)?;
        let regex = Regex::new(pattern).map_err(|e| Error::Eval(format!("table/fs: regex {e}")))?;
        let glob = glob.map(glob_matcher).transpose()?;
        let mut files = Vec::new();
        self.walk(Vec::new(), &mut files)?;
        let mut result = FindGrepResult {
            matches: Vec::new(),
            truncated: false,
        };
        for path in files {
            let rel = path.join("/");
            if !glob.as_ref().is_none_or(|g| g.is_match(&rel)) {
                continue;
            }
            let Some(ext) = path
                .last()
                .and_then(|n| n.rsplit_once('.').map(|(_, e)| e))
                .and_then(|e| known_exts().into_iter().find(|k| *k == e))
            else {
                continue;
            };
            let expr = self.read_leaf_path(cx, &path, ext)?;
            let text = match expr {
                Expr::String(s) => s,
                Expr::Extension { payload, .. } => match *payload {
                    Expr::String(s) => s,
                    _ => continue,
                },
                _ => continue,
            };
            for (line, value) in text.lines().enumerate() {
                if regex.is_match(value) {
                    if result.matches.len() >= max {
                        result.truncated = true;
                        return Ok(result);
                    }
                    result.matches.push(FindMatch {
                        path: rel.clone(),
                        line: (line + 1).try_into().unwrap_or(u32::MAX),
                        text: value.to_owned(),
                    });
                }
            }
        }
        Ok(result)
    }
    /// Globs portable relative paths, bounded by `max`.
    pub fn find_glob(&self, cx: &mut Cx, pattern: &str, max: usize) -> Result<FindGlobResult> {
        require_table_fs_read(cx)?;
        require_table_fs_find(cx)?;
        let glob = glob_matcher(pattern)?;
        let mut paths = Vec::new();
        self.walk(Vec::new(), &mut paths)?;
        let mut result = FindGlobResult {
            paths: Vec::new(),
            truncated: false,
        };
        for path in paths {
            let rel = path.join("/");
            if glob.is_match(&rel) {
                if result.paths.len() >= max {
                    result.truncated = true;
                    break;
                }
                result.paths.push(rel);
            }
        }
        Ok(result)
    }
    fn walk(&self, prefix: Vec<String>, out: &mut Vec<Vec<String>>) -> Result<()> {
        for entry in self.port.list(&prefix).map_err(port_error)? {
            if entry.name.starts_with('.') {
                continue;
            }
            let mut path = prefix.clone();
            path.push(entry.name);
            out.push(path.clone());
            if entry.kind == HostEntryKind::Directory {
                self.walk(path, out)?;
            }
        }
        Ok(())
    }
}
fn glob_matcher(pattern: &str) -> Result<GlobMatcher> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map_err(|e| Error::Eval(format!("table/fs: glob {e}")))
        .map(|g| g.compile_matcher())
}
