use sim_storage_port::{HostDirPort, HostEntryKind, NeverCancel};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

use sim_kernel::{
    Cx, Error, Expr, Object, ObjectEncode, ObjectEncoding, Result, Symbol, TableCompareExchange,
    TableExpected, TableObserved, TableReplacement, Value,
    id::CORE_TABLE_CLASS_ID,
    object::ClassRef,
    table::{Dir, Table},
};

use crate::{
    capabilities::{require_table_fs_read, require_table_fs_write},
    citizen::fs_dir_class_symbol,
    roadmap11::{infer_ext_from_expr, known_exts},
    table_fs_capability,
};

mod leaf_io;

const DEFAULT_EXT: &str = "siml";

/// A SIM table backed by a host directory rooted at a canonical path.
#[derive(Clone)]
pub struct FsDir {
    pub(crate) port: Arc<dyn HostDirPort>,
    reopen_root: Option<PathBuf>,
}

impl FsDir {
    /// Opens (creating if needed) the directory at `root` as a filesystem table.
    ///
    /// The root is created if it does not exist and then canonicalized; an I/O
    /// failure on either step is reported as an error.
    pub fn open(root: PathBuf) -> Result<Self> {
        let port = Arc::new(crate::UbuntuHostDirPort::open(root.clone()).map_err(port_error)?);
        Ok(Self {
            port,
            reopen_root: Some(root),
        })
    }

    /// Opens a filesystem table over an injected platform storage realization.
    pub fn from_port(port: Arc<dyn HostDirPort>) -> Self {
        Self {
            port,
            reopen_root: None,
        }
    }
}

impl Object for FsDir {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("table/fs[{}]", self.port.label()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for FsDir {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        let symbol = fs_dir_class_symbol();
        if let Some(value) = cx.registry().class_by_symbol(&symbol) {
            return Ok(value.clone());
        }
        let symbol = Symbol::qualified("core", "Table");
        if let Some(value) = cx.registry().class_by_symbol(&symbol) {
            return Ok(value.clone());
        }
        cx.factory().class_stub(CORE_TABLE_CLASS_ID, symbol)
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table_expr(cx)
    }
    fn truth(&self, cx: &mut Cx) -> Result<bool> {
        Ok(!self.is_empty(cx)?)
    }
    fn as_table_impl(&self) -> Option<&dyn Table> {
        Some(self)
    }
    fn as_dir(&self) -> Option<&dyn Dir> {
        Some(self)
    }
    fn as_object_encoder(&self) -> Option<&dyn ObjectEncode> {
        Some(self)
    }
}

impl ObjectEncode for FsDir {
    fn object_encoding(&self, _cx: &mut Cx) -> Result<ObjectEncoding> {
        Ok(ObjectEncoding::Constructor {
            class: fs_dir_class_symbol(),
            args: vec![
                Expr::Symbol(Symbol::new("v0")),
                Expr::String(self.reopen_root.as_ref().map_or_else(
                    || self.port.label().to_owned(),
                    |path| path.display().to_string(),
                )),
            ],
        })
    }
}

impl sim_citizen::Citizen for FsDir {
    fn citizen_symbol() -> Symbol {
        fs_dir_class_symbol()
    }

    fn citizen_version() -> u32 {
        0
    }

    fn citizen_arity() -> usize {
        1
    }

    fn citizen_fields() -> &'static [&'static str] {
        &["root"]
    }
}

impl Table for FsDir {
    fn backend_symbol(&self) -> Symbol {
        Symbol::qualified("table", "fs")
    }

    fn get(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        require_table_fs_read(cx)?;
        match self.leaf_path_for_read(&key)? {
            Some(_) => {
                let (_, _, expr) = self.read_leaf_expr(cx, &key)?;
                cx.factory().expr(expr)
            }
            None => cx.factory().nil(),
        }
    }

    fn set(&self, cx: &mut Cx, key: Symbol, value: Value) -> Result<()> {
        require_table_fs_write(cx)?;
        let base = self.segment(&key)?;
        if matches!(
            self.port
                .metadata(&base)
                .map_err(port_error)?
                .map(|m| m.kind),
            Some(HostEntryKind::Directory)
        ) {
            return Err(Error::Eval(format!("table/fs: {key} is a directory")));
        }
        let existing_leaf = self.leaf_path_for_read(&key)?;
        for (path, candidate_ext) in self.leaf_candidates(&key)? {
            if Some(path.clone()) != existing_leaf.as_ref().map(|(path, _)| path.clone())
                && candidate_ext != DEFAULT_EXT
            {
                self.port.remove_file(&path).map_err(port_error)?;
            }
        }
        let expr = value.object().as_expr(cx)?;
        let ext = existing_leaf
            .as_ref()
            .map(|(_, ext)| *ext)
            .or_else(|| infer_ext_from_expr(&expr))
            .unwrap_or(DEFAULT_EXT);
        let path = self.leaf_name(&key, ext)?;
        let bytes = Self::encode_leaf_expr(cx, ext, &expr)?;
        self.port
            .replace(&path, &bytes, &NeverCancel)
            .map_err(port_error)?;
        Ok(())
    }

    fn has(&self, cx: &mut Cx, key: Symbol) -> Result<bool> {
        require_table_fs_read(cx)?;
        let path = self.segment(&key)?;
        Ok(matches!(
            self.port
                .metadata(&path)
                .map_err(port_error)?
                .map(|m| m.kind),
            Some(HostEntryKind::Directory)
        ) || self.leaf_path_for_read(&key)?.is_some())
    }

    fn del(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        require_table_fs_read(cx)?;
        require_table_fs_write(cx)?;
        match self.leaf_path_for_read(&key)? {
            Some((path, ext)) => {
                let expr = self.read_leaf_path(cx, &path, ext)?;
                self.port.remove_file(&path).map_err(port_error)?;
                cx.factory().expr(expr)
            }
            None => cx.factory().nil(),
        }
    }

    fn compare_exchange(
        &self,
        cx: &mut Cx,
        key: Symbol,
        expected: TableExpected,
        replacement: TableReplacement,
    ) -> Result<TableCompareExchange> {
        require_table_fs_read(cx)?;
        require_table_fs_write(cx)?;
        let existing = self.leaf_path_for_read(&key)?;
        let replacement_expr = match replacement {
            TableReplacement::Delete => None,
            TableReplacement::Value(value) => Some(value.object().as_expr(cx)?),
        };
        let ext = existing
            .as_ref()
            .map(|(_, ext)| *ext)
            .or_else(|| replacement_expr.as_ref().and_then(infer_ext_from_expr))
            .unwrap_or(DEFAULT_EXT);
        let path = existing
            .as_ref()
            .map(|(path, _)| path.clone())
            .unwrap_or(self.leaf_name(&key, ext)?);
        let expected_bytes = match &expected {
            TableExpected::Absent => None,
            TableExpected::Value(expr) => Some(Self::encode_leaf_expr(cx, ext, expr)?),
        };
        let replacement_bytes = replacement_expr
            .as_ref()
            .map(|expr| Self::encode_leaf_expr(cx, ext, expr))
            .transpose()?;
        let outcome = self
            .port
            .compare_exchange(
                &path,
                expected_bytes.as_deref(),
                replacement_bytes.as_deref(),
                &NeverCancel,
            )
            .map_err(port_error)?;
        let observed = match outcome.observed {
            None => TableObserved::Absent,
            Some(bytes) => TableObserved::Value(Self::decode_leaf_bytes(cx, ext, &bytes)?),
        };
        Ok(TableCompareExchange {
            exchanged: outcome.exchanged,
            observed,
        })
    }

    fn keys(&self, cx: &mut Cx) -> Result<Vec<Symbol>> {
        require_table_fs_read(cx)?;
        let mut keys = BTreeSet::new();
        for entry in self.port.list(&[]).map_err(port_error)? {
            let name = entry.name;
            if name.starts_with('.') {
                continue;
            }
            if entry.kind == HostEntryKind::Directory {
                keys.insert(Symbol::new(name));
                continue;
            }
            let Some(stem) = known_exts().into_iter().find_map(|ext| {
                name.strip_suffix(&format!(".{ext}"))
                    .map(std::borrow::ToOwned::to_owned)
            }) else {
                continue;
            };
            keys.insert(Symbol::new(stem));
        }
        Ok(keys.into_iter().collect())
    }

    fn entries(&self, cx: &mut Cx) -> Result<Vec<(Symbol, Value)>> {
        require_table_fs_read(cx)?;
        let mut entries = Vec::new();
        for key in self.keys(cx)? {
            if self.is_dir(cx, key.clone())? {
                continue;
            }
            entries.push((key.clone(), self.get(cx, key)?));
        }
        Ok(entries)
    }

    fn len(&self, cx: &mut Cx) -> Result<usize> {
        Ok(self.entries(cx)?.len())
    }

    fn clear(&self, cx: &mut Cx) -> Result<()> {
        require_table_fs_write(cx)?;
        for key in self.keys(cx)? {
            if !self.is_dir(cx, key.clone())? {
                let _ = self.del(cx, key)?;
            }
        }
        Ok(())
    }
}

impl Dir for FsDir {
    fn mkdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        require_table_fs_write(cx)?;
        let path = self.segment(&name)?;
        if self.leaf_path_for_read(&name)?.is_some() {
            return Err(Error::Eval(format!("table/fs: {name} is a file")));
        }
        self.port.create_dir(&path).map_err(port_error)?;
        let child = self.port.child(name.name.as_ref()).map_err(port_error)?;
        cx.factory().opaque(Arc::new(Self::from_port(child)))
    }

    fn opendir(&self, cx: &mut Cx, name: Symbol) -> Result<Option<Value>> {
        require_table_fs_read(cx)?;
        let path = self.segment(&name)?;
        if matches!(
            self.port
                .metadata(&path)
                .map_err(port_error)?
                .map(|m| m.kind),
            Some(HostEntryKind::Directory)
        ) {
            Ok(Some(cx.factory().opaque(Arc::new(Self::from_port(
                self.port.child(name.name.as_ref()).map_err(port_error)?,
            )))?))
        } else if self.port.metadata(&path).map_err(port_error)?.is_some()
            || self.leaf_path_for_read(&name)?.is_some()
        {
            Err(Error::Eval(format!("table/fs: {name} is not a directory")))
        } else {
            Ok(None)
        }
    }

    fn rmdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        require_table_fs_write(cx)?;
        let path = self.segment(&name)?;
        if !matches!(
            self.port
                .metadata(&path)
                .map_err(port_error)?
                .map(|m| m.kind),
            Some(HostEntryKind::Directory)
        ) {
            return Err(Error::Eval(format!("table/fs: {name} is not a directory")));
        }
        self.port.remove_dir_all(&path).map_err(port_error)?;
        cx.factory().nil()
    }

    fn is_dir(&self, cx: &mut Cx, name: Symbol) -> Result<bool> {
        require_table_fs_read(cx)?;
        Ok(matches!(
            self.port
                .metadata(&self.segment(&name)?)
                .map_err(port_error)?
                .map(|m| m.kind),
            Some(HostEntryKind::Directory)
        ))
    }
}

pub(crate) fn port_error(err: sim_storage_port::HostDirError) -> Error {
    Error::Eval(format!("table/fs: {err}"))
}

/// Opens a filesystem table at `root` and returns it as a runtime table value.
///
/// Requires the table-fs capability; the returned value wraps an [`FsDir`].
pub fn install_fs_dir_lib(cx: &mut Cx, root: &str) -> Result<Value> {
    cx.require(&table_fs_capability())?;
    let dir = FsDir::open(PathBuf::from(root))?;
    cx.factory().opaque(Arc::new(dir))
}
