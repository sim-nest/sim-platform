//! Platform realizations of the portable host-directory port.

use sim_storage_port::{
    Cancellation, HostDirError, HostDirErrorKind as Kind, HostDirPort, HostEntry, HostEntryKind,
    PortResult,
};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

fn err(kind: Kind, message: impl Into<String>) -> HostDirError {
    HostDirError::new(kind, message)
}

fn validate(path: &[String]) -> PortResult<()> {
    if path
        .iter()
        .any(|part| !sim_table_core::is_legal_table_segment(part) || Path::new(part).is_absolute())
    {
        return Err(err(Kind::Escape, "table/fs: illegal relative path"));
    }
    Ok(())
}

/// Deterministic in-memory host-directory model with quota and atomic commits.
#[derive(Clone)]
pub struct MemoryHostDirPort {
    state: Arc<Mutex<ModelState>>,
    prefix: Vec<String>,
    label: String,
}

#[derive(Default)]
struct ModelState {
    entries: BTreeMap<Vec<String>, ModelEntry>,
    quota: u64,
}
#[derive(Clone)]
enum ModelEntry {
    File(Vec<u8>),
    Dir,
}

impl MemoryHostDirPort {
    /// Creates an empty model mount with a byte quota.
    pub fn new(label: impl Into<String>, quota: u64) -> Self {
        let state = ModelState {
            quota,
            ..ModelState::default()
        };
        Self {
            state: Arc::new(Mutex::new(state)),
            prefix: Vec::new(),
            label: label.into(),
        }
    }
    fn full(&self, path: &[String]) -> PortResult<Vec<String>> {
        validate(path)?;
        Ok(self.prefix.iter().chain(path).cloned().collect())
    }
}

impl HostDirPort for MemoryHostDirPort {
    fn label(&self) -> &str {
        &self.label
    }
    fn list(&self, dir: &[String]) -> PortResult<Vec<HostEntry>> {
        let base = self.full(dir)?;
        let state = self
            .state
            .lock()
            .map_err(|_| err(Kind::Native, "model lock poisoned"))?;
        let mut rows = BTreeMap::new();
        for (path, value) in &state.entries {
            if path.len() == base.len() + 1 && path.starts_with(&base) {
                let (kind, len) = match value {
                    ModelEntry::File(bytes) => (HostEntryKind::File, bytes.len() as u64),
                    ModelEntry::Dir => (HostEntryKind::Directory, 0),
                };
                rows.insert(
                    path.last().cloned().unwrap_or_default(),
                    HostEntry {
                        name: path.last().cloned().unwrap_or_default(),
                        kind,
                        len,
                    },
                );
            }
        }
        Ok(rows.into_values().collect())
    }
    fn metadata(&self, path: &[String]) -> PortResult<Option<HostEntry>> {
        let full = self.full(path)?;
        let state = self
            .state
            .lock()
            .map_err(|_| err(Kind::Native, "model lock poisoned"))?;
        Ok(state.entries.get(&full).map(|value| {
            let (kind, len) = match value {
                ModelEntry::File(b) => (HostEntryKind::File, b.len() as u64),
                ModelEntry::Dir => (HostEntryKind::Directory, 0),
            };
            HostEntry {
                name: full.last().cloned().unwrap_or_default(),
                kind,
                len,
            }
        }))
    }
    fn read(&self, path: &[String]) -> PortResult<Vec<u8>> {
        let full = self.full(path)?;
        let state = self
            .state
            .lock()
            .map_err(|_| err(Kind::Native, "model lock poisoned"))?;
        match state.entries.get(&full) {
            Some(ModelEntry::File(b)) => Ok(b.clone()),
            Some(ModelEntry::Dir) => Err(err(Kind::SpecialFile, "entry is a directory")),
            None => Err(err(Kind::NotFound, "entry not found")),
        }
    }
    fn replace(&self, path: &[String], bytes: &[u8], cancel: &dyn Cancellation) -> PortResult<()> {
        if cancel.is_cancelled() {
            return Err(err(Kind::Cancelled, "write cancelled"));
        }
        let full = self.full(path)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| err(Kind::Native, "model lock poisoned"))?;
        let used: u64 = state
            .entries
            .values()
            .map(|e| match e {
                ModelEntry::File(b) => b.len() as u64,
                ModelEntry::Dir => 0,
            })
            .sum();
        let old = match state.entries.get(&full) {
            Some(ModelEntry::File(b)) => b.len() as u64,
            _ => 0,
        };
        if used - old + bytes.len() as u64 > state.quota {
            return Err(err(Kind::QuotaExceeded, "storage quota exceeded"));
        }
        state.entries.insert(full, ModelEntry::File(bytes.to_vec()));
        Ok(())
    }
    fn remove_file(&self, path: &[String]) -> PortResult<()> {
        let full = self.full(path)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| err(Kind::Native, "model lock poisoned"))?;
        match state.entries.remove(&full) {
            Some(ModelEntry::File(_)) => Ok(()),
            Some(ModelEntry::Dir) => {
                state.entries.insert(full, ModelEntry::Dir);
                Err(err(Kind::SpecialFile, "entry is a directory"))
            }
            None => Err(err(Kind::NotFound, "entry not found")),
        }
    }
    fn create_dir(&self, path: &[String]) -> PortResult<()> {
        let full = self.full(path)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| err(Kind::Native, "model lock poisoned"))?;
        state.entries.entry(full).or_insert(ModelEntry::Dir);
        Ok(())
    }
    fn remove_dir_all(&self, path: &[String]) -> PortResult<()> {
        let full = self.full(path)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| err(Kind::Native, "model lock poisoned"))?;
        if !matches!(state.entries.get(&full), Some(ModelEntry::Dir)) {
            return Err(err(Kind::NotFound, "directory not found"));
        }
        state.entries.retain(|key, _| !key.starts_with(&full));
        Ok(())
    }
    fn child(&self, name: &str) -> PortResult<Arc<dyn HostDirPort>> {
        validate(&[name.to_owned()])?;
        let mut prefix = self.prefix.clone();
        prefix.push(name.to_owned());
        Ok(Arc::new(Self {
            state: self.state.clone(),
            prefix,
            label: self.label.clone(),
        }))
    }
}

/// Ubuntu filesystem realization confined below one canonical root.
#[derive(Clone)]
pub struct UbuntuHostDirPort {
    root: PathBuf,
    prefix: Vec<String>,
    label: String,
}
impl UbuntuHostDirPort {
    /// Opens or creates a native mount root.
    pub fn open(root: PathBuf) -> PortResult<Self> {
        std::fs::create_dir_all(&root).map_err(map_io)?;
        let root = std::fs::canonicalize(root).map_err(map_io)?;
        let label = format!("ubuntu:{}", root.display());
        Ok(Self {
            root,
            prefix: Vec::new(),
            label,
        })
    }
    fn resolve(&self, path: &[String]) -> PortResult<PathBuf> {
        validate(path)?;
        let candidate = self
            .prefix
            .iter()
            .chain(path)
            .fold(self.root.clone(), |p, s| p.join(s));
        if candidate == self.root {
            return Ok(candidate);
        }
        let parent = candidate.parent().unwrap_or(&self.root);
        let canonical_parent = std::fs::canonicalize(parent).map_err(map_io)?;
        if !canonical_parent.starts_with(&self.root) {
            return Err(err(Kind::Escape, "path escapes root"));
        }
        Ok(candidate)
    }
}
fn map_io(error: std::io::Error) -> HostDirError {
    let kind = match error.kind() {
        std::io::ErrorKind::NotFound => Kind::NotFound,
        std::io::ErrorKind::AlreadyExists => Kind::AlreadyExists,
        _ => Kind::Native,
    };
    err(kind, format!("native storage: {error}"))
}
impl HostDirPort for UbuntuHostDirPort {
    fn label(&self) -> &str {
        &self.label
    }
    fn list(&self, dir: &[String]) -> PortResult<Vec<HostEntry>> {
        let path = self.resolve(dir)?;
        let mut out = Vec::new();
        for row in std::fs::read_dir(path).map_err(map_io)? {
            let row = row.map_err(map_io)?;
            let name = row
                .file_name()
                .into_string()
                .map_err(|_| err(Kind::Malformed, "native name is not UTF-8"))?;
            let ty = row.file_type().map_err(map_io)?;
            let (kind, len) = if ty.is_file() {
                (HostEntryKind::File, row.metadata().map_err(map_io)?.len())
            } else if ty.is_dir() {
                (HostEntryKind::Directory, 0)
            } else {
                return Err(err(
                    Kind::SpecialFile,
                    "path escapes root or is an unsupported special file",
                ));
            };
            out.push(HostEntry { name, kind, len });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
    fn metadata(&self, path: &[String]) -> PortResult<Option<HostEntry>> {
        let native = self.resolve(path)?;
        let meta = match std::fs::symlink_metadata(native) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(map_io(e)),
        };
        let ty = meta.file_type();
        let kind = if ty.is_file() {
            HostEntryKind::File
        } else if ty.is_dir() {
            HostEntryKind::Directory
        } else {
            return Err(err(Kind::SpecialFile, "unsupported special file"));
        };
        Ok(Some(HostEntry {
            name: path.last().cloned().unwrap_or_default(),
            kind,
            len: meta.len(),
        }))
    }
    fn read(&self, path: &[String]) -> PortResult<Vec<u8>> {
        std::fs::read(self.resolve(path)?).map_err(map_io)
    }
    fn replace(&self, path: &[String], bytes: &[u8], cancel: &dyn Cancellation) -> PortResult<()> {
        if cancel.is_cancelled() {
            return Err(err(Kind::Cancelled, "write cancelled"));
        }
        let target = self.resolve(path)?;
        let parent = target
            .parent()
            .ok_or_else(|| err(Kind::Escape, "missing parent"))?;
        for n in 0..64 {
            let temp = parent.join(format!(".sim-replace-{}-{n}", std::process::id()));
            match OpenOptions::new().create_new(true).write(true).open(&temp) {
                Ok(mut file) => {
                    if let Err(e) = file.write_all(bytes).and_then(|_| file.sync_all()) {
                        let _ = std::fs::remove_file(&temp);
                        return Err(map_io(e));
                    }
                    if cancel.is_cancelled() {
                        let _ = std::fs::remove_file(&temp);
                        return Err(err(Kind::Cancelled, "write cancelled"));
                    }
                    std::fs::rename(&temp, &target).map_err(map_io)?;
                    OpenOptions::new()
                        .read(true)
                        .open(parent)
                        .and_then(|f| f.sync_all())
                        .map_err(map_io)?;
                    return Ok(());
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(map_io(e)),
            }
        }
        Err(err(Kind::Native, "temporary namespace exhausted"))
    }
    fn remove_file(&self, path: &[String]) -> PortResult<()> {
        std::fs::remove_file(self.resolve(path)?).map_err(map_io)
    }
    fn create_dir(&self, path: &[String]) -> PortResult<()> {
        std::fs::create_dir_all(self.resolve(path)?).map_err(map_io)
    }
    fn remove_dir_all(&self, path: &[String]) -> PortResult<()> {
        std::fs::remove_dir_all(self.resolve(path)?).map_err(map_io)
    }
    fn child(&self, name: &str) -> PortResult<Arc<dyn HostDirPort>> {
        validate(&[name.to_owned()])?;
        let mut prefix = self.prefix.clone();
        prefix.push(name.to_owned());
        Ok(Arc::new(Self {
            root: self.root.clone(),
            prefix,
            label: self.label.clone(),
        }))
    }
}
