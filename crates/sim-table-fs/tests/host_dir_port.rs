use sim_storage_port::{Cancellation, HostDirErrorKind, HostDirPort, HostEntryKind, NeverCancel};
use sim_table_fs::{MemoryHostDirPort, UbuntuHostDirPort};
use std::{
    path::PathBuf,
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    },
};

struct Flag(AtomicBool);
impl Cancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn conformance(port: &dyn HostDirPort) {
    port.create_dir(&path(&["nested"])).unwrap();
    port.replace(&path(&["alpha.siml"]), b"old", &NeverCancel)
        .unwrap();
    port.replace(&path(&["alpha.siml"]), b"new", &NeverCancel)
        .unwrap();
    assert_eq!(port.read(&path(&["alpha.siml"])).unwrap(), b"new");
    assert_eq!(
        port.list(&[])
            .unwrap()
            .iter()
            .map(|e| (&e.name, e.kind))
            .collect::<Vec<_>>(),
        vec![
            (&"alpha.siml".to_owned(), HostEntryKind::File),
            (&"nested".to_owned(), HostEntryKind::Directory)
        ]
    );
    let child = port.child("nested").unwrap();
    child
        .replace(&path(&["leaf.siml"]), b"payload", &NeverCancel)
        .unwrap();
    assert_eq!(
        port.read(&path(&["nested", "leaf.siml"])).unwrap(),
        b"payload"
    );
    for bad in [
        vec!["..".to_owned()],
        vec!["a/b".to_owned()],
        vec!["a\\b".to_owned()],
    ] {
        assert_eq!(
            port.metadata(&bad).unwrap_err().kind,
            HostDirErrorKind::Escape
        );
    }
    let cancelled = Flag(AtomicBool::new(true));
    assert_eq!(
        port.replace(&path(&["cancelled"]), b"no", &cancelled)
            .unwrap_err()
            .kind,
        HostDirErrorKind::Cancelled
    );
    assert!(port.metadata(&path(&["cancelled"])).unwrap().is_none());
    port.remove_dir_all(&path(&["nested"])).unwrap();
    assert!(port.metadata(&path(&["nested"])).unwrap().is_none());
}

fn cas_conformance(port: &dyn HostDirPort) {
    let leaf = path(&["lease.siml"]);
    let first = port
        .compare_exchange(&leaf, None, Some(b"nil"), &NeverCancel)
        .unwrap();
    assert!(first.exchanged);
    assert_eq!(first.observed, None);
    let stale = port
        .compare_exchange(&leaf, Some(b"wrong"), None, &NeverCancel)
        .unwrap();
    assert!(!stale.exchanged);
    assert_eq!(stale.observed.as_deref(), Some(b"nil".as_slice()));
    assert!(
        port.compare_exchange(&leaf, Some(b"nil"), None, &NeverCancel)
            .unwrap()
            .exchanged
    );
    assert!(port.metadata(&leaf).unwrap().is_none());
}

fn concurrent_claimers(port: Arc<dyn HostDirPort>) {
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for value in [b"one".as_slice(), b"two".as_slice()] {
        let port = Arc::clone(&port);
        let barrier = Arc::clone(&barrier);
        let value = value.to_vec();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            port.compare_exchange(&path(&["claim"]), None, Some(&value), &NeverCancel)
                .unwrap()
                .exchanged
        }));
    }
    barrier.wait();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|won| *won)
            .count(),
        1
    );
}

#[test]
fn model_and_ubuntu_share_conformance() {
    conformance(&MemoryHostDirPort::new("model", 4096));
    cas_conformance(&MemoryHostDirPort::new("model-cas", 4096));
    concurrent_claimers(Arc::new(MemoryHostDirPort::new("model-race", 4096)));
    let root = temp_root("ubuntu");
    conformance(&UbuntuHostDirPort::open(root.clone()).unwrap());
    cas_conformance(&UbuntuHostDirPort::open(root.clone()).unwrap());
    concurrent_claimers(Arc::new(UbuntuHostDirPort::open(root.clone()).unwrap()));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn model_enforces_quota_without_partial_commit() {
    let port = MemoryHostDirPort::new("quota", 3);
    port.replace(&path(&["leaf"]), b"old", &NeverCancel)
        .unwrap();
    assert_eq!(
        port.replace(&path(&["leaf"]), b"larger", &NeverCancel)
            .unwrap_err()
            .kind,
        HostDirErrorKind::QuotaExceeded
    );
    assert_eq!(port.read(&path(&["leaf"])).unwrap(), b"old");
}

#[cfg(unix)]
#[test]
fn ubuntu_rejects_symlink_and_special_file_escape() {
    use std::os::unix::fs::symlink;
    let root = temp_root("escape");
    let outside = temp_root("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    symlink(&outside, root.join("link")).unwrap();
    let port = UbuntuHostDirPort::open(root.clone()).unwrap();
    assert_eq!(
        port.metadata(&path(&["link"])).unwrap_err().kind,
        HostDirErrorKind::SpecialFile
    );
    assert_eq!(
        port.child("link")
            .unwrap()
            .metadata(&path(&["leaf"]))
            .unwrap_err()
            .kind,
        HostDirErrorKind::Escape
    );
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "sim-host-dir-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
