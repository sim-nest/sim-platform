#![forbid(unsafe_code)]
//! Bounded Linux mechanics. Native paths and portal calls cannot escape this package.

use serde::{Deserialize, Serialize};
use sim_platform_core::{Lifecycle, OpenSymbol, RefusalKind, ResolutionRefusal};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod transport;
pub use transport::{LinuxDnsPort, LinuxIpcPort, LinuxSocketPort, bind_transport_services};

/// Privacy-filtered facts supplied during registration.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct HostFacts {
    pub locale: String,
    pub timezone: String,
    pub memory_bytes: u64,
    pub storage_bytes: u64,
    pub parallelism: u16,
    pub pressure: u8,
}

/// Named preopened roots. Paths never appear in Cards, receipts, or attestations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XdgMounts {
    roots: BTreeMap<OpenSymbol, PathBuf>,
}
impl XdgMounts {
    /// Admit exactly the five caller-preopened XDG roots.
    #[must_use]
    pub fn new(
        config: PathBuf,
        cache: PathBuf,
        data: PathBuf,
        state: PathBuf,
        temp: PathBuf,
    ) -> Self {
        Self {
            roots: BTreeMap::from([
                (OpenSymbol("mount/xdg-config".into()), config),
                (OpenSymbol("mount/xdg-cache".into()), cache),
                (OpenSymbol("mount/xdg-data".into()), data),
                (OpenSymbol("mount/xdg-state".into()), state),
                (OpenSymbol("mount/temp".into()), temp),
            ]),
        }
    }
    #[must_use]
    pub fn contains(&self, mount: &OpenSymbol) -> bool {
        self.roots.contains_key(mount)
    }
    #[must_use]
    pub fn names(&self) -> Vec<OpenSymbol> {
        self.roots.keys().cloned().collect()
    }
}

/// Desktop operations remain behind one injected portal boundary.
pub trait Portal: Send {
    /// # Errors
    /// Returns a typed fail-closed portal refusal.
    fn call(&mut self, operation: DesktopOperation) -> Result<PortalReply, PortalError>;
    fn cancel(&mut self, token: u64);
    fn cleanup(&mut self);
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesktopOperation {
    Open(String),
    Share(String),
    Notify(String),
    ClipboardRead,
    ClipboardWrite(String),
    PermissionStatus(String),
    PermissionRequest(String),
    KeepAwake(bool),
    Activate(String),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortalReply {
    Accepted,
    Text(String),
    Permission(Permission),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permission {
    Granted,
    Denied,
    Revoked,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortalError {
    Unsupported,
    Denied,
    Revoked,
    Cancelled,
}

/// Fixed per-activation resource ceilings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Budget {
    pub requests: u32,
    pub queue: usize,
    pub entropy_bytes: usize,
    pub timer_ns: u64,
}

/// Requests accepted by the private capsule membrane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request {
    WallClock,
    MonotonicClock,
    Timer(u64),
    Entropy(usize),
    Locale,
    Timezone,
    Pressure,
    Limits,
    Mount(OpenSymbol),
    Desktop(DesktopOperation),
    Lifecycle,
    Cancel(u64),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reply {
    Integer(i128),
    Bytes(Vec<u8>),
    Text(String),
    Limits {
        memory_bytes: u64,
        storage_bytes: u64,
        parallelism: u16,
    },
    Mount(OpenSymbol),
    Portal(PortalReply),
    Lifecycle(Lifecycle),
    Cancelled,
}

/// One explicitly configured Linux activation.
pub struct Capsule<P: Portal> {
    facts: HostFacts,
    mounts: XdgMounts,
    portal: Option<P>,
    budget: Budget,
    used: u32,
    entropy_used: usize,
    seed: u64,
    start: Instant,
    lifecycle: Lifecycle,
    queue: VecDeque<u64>,
    cancelled: BTreeSet<u64>,
}
impl<P: Portal> Capsule<P> {
    #[must_use]
    pub fn new(
        facts: HostFacts,
        mounts: XdgMounts,
        portal: Option<P>,
        budget: Budget,
        seed: u64,
    ) -> Self {
        Self {
            facts,
            mounts,
            portal,
            budget,
            used: 0,
            entropy_used: 0,
            seed: seed.max(1),
            start: Instant::now(),
            lifecycle: Lifecycle::Ready,
            queue: VecDeque::new(),
            cancelled: BTreeSet::new(),
        }
    }
    pub fn suspend(&mut self) {
        self.lifecycle = Lifecycle::Suspended;
    }
    pub fn resume(&mut self) {
        self.lifecycle = Lifecycle::Ready;
    }
    pub fn revoke(&mut self, token: u64) {
        self.cancelled.insert(token);
    }
    /// Execute one bounded request. Unsupported and denied operations fail closed.
    ///
    /// # Errors
    /// Returns a typed refusal for unsupported, denied, revoked, suspended,
    /// cancelled, or over-budget work.
    pub fn apply(&mut self, token: u64, request: Request) -> Result<Reply, ResolutionRefusal> {
        if self.lifecycle == Lifecycle::Suspended {
            return Err(refusal(RefusalKind::Suspended, "activation is suspended"));
        }
        if self.cancelled.contains(&token) {
            return Err(refusal(
                RefusalKind::Cancelled,
                "request is cancelled or revoked",
            ));
        }
        if self.used >= self.budget.requests || self.queue.len() >= self.budget.queue {
            return Err(refusal(
                RefusalKind::BudgetExhausted,
                "activation budget exhausted",
            ));
        }
        self.used += 1;
        self.queue.push_back(token);
        let result = self.execute(token, request);
        self.queue.pop_front();
        result
    }
    fn execute(&mut self, _token: u64, request: Request) -> Result<Reply, ResolutionRefusal> {
        match request {
            Request::WallClock => Ok(Reply::Integer(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| refusal(RefusalKind::ProviderFault, "wall clock before epoch"))?
                    .as_nanos()
                    .cast_signed(),
            )),
            Request::MonotonicClock => Ok(Reply::Integer(
                self.start.elapsed().as_nanos().cast_signed(),
            )),
            Request::Timer(ns) if ns <= self.budget.timer_ns => {
                std::thread::sleep(Duration::from_nanos(ns));
                Ok(Reply::Integer(
                    self.start.elapsed().as_nanos().cast_signed(),
                ))
            }
            Request::Timer(_) => Err(refusal(
                RefusalKind::BudgetExhausted,
                "timer budget exceeded",
            )),
            Request::Entropy(bytes)
                if self.entropy_used.saturating_add(bytes) <= self.budget.entropy_bytes =>
            {
                self.entropy_used += bytes;
                Ok(Reply::Bytes(self.entropy(bytes)))
            }
            Request::Entropy(_) => Err(refusal(
                RefusalKind::BudgetExhausted,
                "entropy budget exceeded",
            )),
            Request::Locale => Ok(Reply::Text(self.facts.locale.clone())),
            Request::Timezone => Ok(Reply::Text(self.facts.timezone.clone())),
            Request::Pressure => Ok(Reply::Integer(i128::from(self.facts.pressure))),
            Request::Limits => Ok(Reply::Limits {
                memory_bytes: self.facts.memory_bytes,
                storage_bytes: self.facts.storage_bytes,
                parallelism: self.facts.parallelism,
            }),
            Request::Mount(name) if self.mounts.contains(&name) => Ok(Reply::Mount(name)),
            Request::Mount(_) => Err(refusal(RefusalKind::Unsupported, "mount was not preopened")),
            Request::Desktop(op) => self
                .portal
                .as_mut()
                .ok_or_else(|| refusal(RefusalKind::Unsupported, "desktop service is absent"))?
                .call(op)
                .map(Reply::Portal)
                .map_err(|e| {
                    refusal(
                        match e {
                            PortalError::Unsupported => RefusalKind::Unsupported,
                            PortalError::Denied | PortalError::Revoked => RefusalKind::Denied,
                            PortalError::Cancelled => RefusalKind::Cancelled,
                        },
                        "portal refused request",
                    )
                }),
            Request::Lifecycle => Ok(Reply::Lifecycle(self.lifecycle.clone())),
            Request::Cancel(cancelled) => {
                self.cancelled.insert(cancelled);
                if let Some(portal) = &mut self.portal {
                    portal.cancel(cancelled);
                }
                Ok(Reply::Cancelled)
            }
        }
    }
    fn entropy(&mut self, count: usize) -> Vec<u8> {
        (0..count)
            .map(|_| {
                self.seed ^= self.seed << 13;
                self.seed ^= self.seed >> 7;
                self.seed ^= self.seed << 17;
                self.seed.to_le_bytes()[0]
            })
            .collect()
    }
}
impl<P: Portal> Drop for Capsule<P> {
    fn drop(&mut self) {
        self.lifecycle = Lifecycle::Stopped;
        self.queue.clear();
        if let Some(portal) = &mut self.portal {
            portal.cleanup();
        }
    }
}
fn refusal(kind: RefusalKind, detail: &str) -> ResolutionRefusal {
    ResolutionRefusal {
        request: OpenSymbol("request/ubuntu-pc".into()),
        service: OpenSymbol("service/ubuntu-pc".into()),
        kind,
        detail: detail.into(),
    }
}

/// Portal used for a headless capsule; no desktop operation is ever supported.
pub struct HeadlessPortal;
impl Portal for HeadlessPortal {
    fn call(&mut self, _: DesktopOperation) -> Result<PortalReply, PortalError> {
        Err(PortalError::Unsupported)
    }
    fn cancel(&mut self, _: u64) {}
    fn cleanup(&mut self) {}
}
