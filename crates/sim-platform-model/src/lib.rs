#![forbid(unsafe_code)]
//! Deterministic, host-free platform model and the standard platform site.

use serde::{Deserialize, Serialize};
use sim_kernel::{
    CapabilityName, ClassRef, Cx, DefaultFactory, Env, Error, EvalFabric, EvalReply, EvalRequest,
    Factory, Object, Result as KernelResult, ShapeId, Symbol,
};
use sim_platform_core::{
    BoundServices, BundleManifest, CapsuleManifest, ExecutionEvidence, FactPort, Lifecycle,
    OpenSymbol, RefusalKind, ResolutionReceipt, ResolutionRefusal, ValidationError, parse_bundle,
    parse_capsule, stable_digest,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

mod process;
pub use process::{ModelProcess, ModelProcessOutcome};

pub const FICTIONAL_CAPSULE: &str = r#"
schema = "sim.platform-capsule/v1"
provider = "platform/site/fictional"
services = ["service/fictional-clock"]
shells = ["shell/fictional"]
"#;

pub const FICTIONAL_BUNDLE: &str = r#"
schema = "sim.platform-bundle/v1"
capsule = "platform/site/fictional"
artifact = "lib/sim-platform-fictional"
artifact_content = "sha256:fictional-not-an-artifact"
entry = "sim_native_abi_v1"
shell = "shell/fictional"
"#;

/// Curated model-distribution bundle. Its single row is the complete closure.
pub const MODEL_BUNDLE_MANIFEST: &str = FICTIONAL_BUNDLE;

/// Parse both fictional records.
///
/// # Errors
/// Returns a parse error if the committed specimen stops matching its contract.
pub fn fictional_records() -> Result<(CapsuleManifest, BundleManifest), ValidationError> {
    Ok((
        parse_capsule(FICTIONAL_CAPSULE)?,
        parse_bundle(FICTIONAL_BUNDLE)?,
    ))
}

/// Privacy-filtered limits: no machine or process identity leaks through this record.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct MachineLimits {
    pub memory_bytes: u64,
    pub storage_bytes: u64,
    pub parallelism: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct LifecycleStep {
    pub at_mono_ns: u64,
    pub state: Lifecycle,
}

/// Every common fail-closed path can be selected explicitly by a test.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum InjectedFault {
    Unsupported,
    InsufficientEvidence,
    Timer,
    Entropy,
    Mount,
    Lifecycle,
    Limits,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct ModelConfig {
    pub seed: u64,
    pub wall_epoch_ns: i128,
    pub locale: String,
    pub timezone: String,
    pub limits: MachineLimits,
    #[serde(default)]
    pub lifecycle: Vec<LifecycleStep>,
    #[serde(default)]
    pub mounts: BTreeMap<OpenSymbol, Vec<u8>>,
    pub fault: Option<InjectedFault>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum ModelRequest {
    ReadFact(FactPort),
    AdvanceTime { nanos: u64 },
    Entropy { bytes: usize },
    ReadMount { mount: OpenSymbol },
    Lifecycle,
    Limits,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum ModelResult {
    Integer(i128),
    Text(String),
    Bytes(Vec<u8>),
    Lifecycle(Lifecycle),
    Limits(MachineLimits),
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct ModelReply {
    pub result: ModelResult,
    pub receipt: ResolutionReceipt,
    pub evidence: ExecutionEvidence,
}

#[derive(Clone)]
pub struct SeededPlatform {
    inner: Arc<Mutex<ModelState>>,
}
struct ModelState {
    config: ModelConfig,
    mono_ns: u64,
    rng: u64,
    sequence: u64,
    scripts: VecDeque<LifecycleStep>,
}

impl SeededPlatform {
    #[must_use]
    pub fn new(config: ModelConfig) -> Self {
        let scripts = config.lifecycle.iter().cloned().collect();
        let rng = config.seed.max(1);
        Self {
            inner: Arc::new(Mutex::new(ModelState {
                config,
                mono_ns: 0,
                rng,
                sequence: 0,
                scripts,
            })),
        }
    }
    /// Applies one request with no host interaction.
    ///
    /// # Errors
    /// Returns the configured injected refusal or an Unsupported refusal for
    /// an unbound modeled mount.
    ///
    /// # Panics
    /// Panics if another thread poisoned the model mutex or if serde cannot
    /// serialize the closed request/result enums.
    pub fn apply(
        &self,
        request: &ModelRequest,
    ) -> std::result::Result<ModelReply, ResolutionRefusal> {
        let mut state = self.inner.lock().expect("model mutex poisoned");
        state.sequence += 1;
        refuse_injected(&state.config, request)?;
        let result = match request {
            ModelRequest::ReadFact(FactPort::WallClock) => {
                ModelResult::Integer(state.config.wall_epoch_ns + i128::from(state.mono_ns))
            }
            ModelRequest::ReadFact(FactPort::MonotonicClock | FactPort::Timer) => {
                ModelResult::Integer(i128::from(state.mono_ns))
            }
            ModelRequest::ReadFact(FactPort::Entropy) => {
                ModelResult::Bytes(next_bytes(&mut state, 8))
            }
            ModelRequest::Entropy { bytes } => ModelResult::Bytes(next_bytes(&mut state, *bytes)),
            ModelRequest::ReadFact(FactPort::Locale) => {
                ModelResult::Text(state.config.locale.clone())
            }
            ModelRequest::ReadFact(FactPort::Timezone) => {
                ModelResult::Text(state.config.timezone.clone())
            }
            ModelRequest::ReadFact(FactPort::LifecyclePressure) | ModelRequest::Lifecycle => {
                ModelResult::Lifecycle(current_lifecycle(&mut state))
            }
            ModelRequest::ReadFact(FactPort::MachineLimits) | ModelRequest::Limits => {
                ModelResult::Limits(state.config.limits.clone())
            }
            ModelRequest::AdvanceTime { nanos } => {
                state.mono_ns = state.mono_ns.saturating_add(*nanos);
                ModelResult::Integer(i128::from(state.mono_ns))
            }
            ModelRequest::ReadMount { mount } => {
                ModelResult::Bytes(state.config.mounts.get(mount).cloned().ok_or_else(|| {
                    refusal(
                        state.sequence,
                        RefusalKind::Unsupported,
                        mount.clone(),
                        "mount is unbound",
                    )
                })?)
            }
        };
        let request_digest =
            stable_digest(&serde_json::to_vec(request).expect("request serializes"));
        let result_digest = stable_digest(&serde_json::to_vec(&result).expect("result serializes"));
        let request_id = OpenSymbol(format!("request/{request_digest}"));
        Ok(ModelReply {
            result,
            receipt: ResolutionReceipt {
                id: OpenSymbol(format!("receipt/model/{:016x}", state.sequence)),
                request: request_id,
                site: OpenSymbol("platform/site/model".into()),
                bindings: Vec::new(),
                card_digest: "fnv1a64:model".into(),
            },
            evidence: ExecutionEvidence {
                execution: OpenSymbol(format!("execution/model/{:016x}", state.sequence)),
                activation: OpenSymbol("activation/model".into()),
                request_digest,
                result_digest,
                ledger_ref: OpenSymbol(format!("ledger/model/{:016x}", state.sequence)),
            },
        })
    }
}
fn next_bytes(state: &mut ModelState, count: usize) -> Vec<u8> {
    (0..count)
        .map(|_| {
            state.rng ^= state.rng << 13;
            state.rng ^= state.rng >> 7;
            state.rng ^= state.rng << 17;
            state.rng.to_le_bytes()[0]
        })
        .collect()
}
fn current_lifecycle(state: &mut ModelState) -> Lifecycle {
    while state
        .scripts
        .front()
        .is_some_and(|step| step.at_mono_ns <= state.mono_ns)
    {
        state.scripts.pop_front();
    }
    state
        .scripts
        .front()
        .map_or(Lifecycle::Ready, |step| step.state.clone())
}
fn refusal(seq: u64, kind: RefusalKind, service: OpenSymbol, detail: &str) -> ResolutionRefusal {
    ResolutionRefusal {
        request: OpenSymbol(format!("request/model/{seq:016x}")),
        service,
        kind,
        detail: detail.into(),
    }
}
fn refuse_injected(
    config: &ModelConfig,
    request: &ModelRequest,
) -> std::result::Result<(), ResolutionRefusal> {
    let hit = matches!(
        (&config.fault, request),
        (
            Some(InjectedFault::Unsupported | InjectedFault::InsufficientEvidence),
            _
        ) | (
            Some(InjectedFault::Timer),
            ModelRequest::AdvanceTime { .. } | ModelRequest::ReadFact(FactPort::Timer)
        ) | (
            Some(InjectedFault::Entropy),
            ModelRequest::Entropy { .. } | ModelRequest::ReadFact(FactPort::Entropy)
        ) | (Some(InjectedFault::Mount), ModelRequest::ReadMount { .. })
            | (
                Some(InjectedFault::Lifecycle),
                ModelRequest::Lifecycle | ModelRequest::ReadFact(FactPort::LifecyclePressure)
            )
            | (
                Some(InjectedFault::Limits),
                ModelRequest::Limits | ModelRequest::ReadFact(FactPort::MachineLimits)
            )
    );
    if hit {
        let kind = if matches!(
            config.fault,
            Some(InjectedFault::InsufficientEvidence | InjectedFault::Unsupported)
        ) {
            RefusalKind::Unsupported
        } else {
            RefusalKind::ProviderFault
        };
        Err(refusal(
            0,
            kind,
            OpenSymbol("service/injected".into()),
            "deterministic injected refusal",
        ))
    } else {
        Ok(())
    }
}

/// Opaque service value bound into a child environment for one realization.
#[derive(Clone)]
pub struct PlatformService {
    pub symbol: Symbol,
    pub model: SeededPlatform,
}
impl Object for PlatformService {
    fn display(&self, _cx: &mut Cx) -> KernelResult<String> {
        Ok(format!("#<platform-service {}>", self.symbol))
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl sim_kernel::ObjectCompat for PlatformService {
    fn class(&self, _cx: &mut Cx) -> KernelResult<ClassRef> {
        DefaultFactory.class_stub(
            sim_kernel::CORE_CARD_CLASS_ID,
            Symbol::qualified("platform", "Service"),
        )
    }
}

/// Standard site-binding architecture, intentionally matching `TensorSite`.
#[derive(Clone)]
pub struct PlatformSite {
    symbol: Symbol,
    services: Arc<BTreeMap<Symbol, Arc<PlatformService>>>,
    capabilities: Arc<[CapabilityName]>,
}
impl PlatformSite {
    #[must_use]
    pub fn new(
        symbol: Symbol,
        services: BTreeMap<Symbol, Arc<PlatformService>>,
        capabilities: Vec<CapabilityName>,
    ) -> Self {
        Self {
            symbol,
            services: Arc::new(services),
            capabilities: capabilities.into(),
        }
    }
    #[must_use]
    pub fn symbol(&self) -> &Symbol {
        &self.symbol
    }
}
impl EvalFabric for PlatformSite {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> KernelResult<EvalReply> {
        cx.require_all(&self.capabilities)?;
        cx.require_all(&request.required_capabilities)?;
        if let sim_kernel::Expr::Symbol(symbol) = &request.expr
            && symbol.namespace.as_deref() == Some("platform")
            && !self.services.contains_key(symbol)
        {
            return Err(Error::Eval(format!(
                "platform service {symbol} is Unsupported because it is unbound"
            )));
        }
        let mut child = Env::child(Arc::new(cx.env().clone()));
        for (symbol, service) in self.services.iter() {
            child.define(symbol.clone(), cx.factory().opaque(service.clone())?);
        }
        let value = cx.with_env(child, |cx| cx.eval_expr(request.expr))?;
        if let Some(shape_value) = request.result_shape.clone() {
            let shape = shape_value.object().as_shape().ok_or(Error::TypeMismatch {
                expected: "shape",
                found: "non-shape",
            })?;
            let matched = shape.check_value(cx, value.clone())?;
            if !matched.accepted {
                return Err(Error::WrongShape {
                    expected: shape.id().unwrap_or(ShapeId(0)),
                    diagnostics: matched.diagnostics,
                });
            }
        }
        Ok(EvalReply {
            value,
            diagnostics: Vec::new(),
            trace: request.trace.then(|| {
                DefaultFactory
                    .symbol(Symbol::qualified("platform", "trace/model"))
                    .expect("symbol boxes")
            }),
        })
    }
}
impl Object for PlatformSite {
    fn display(&self, _cx: &mut Cx) -> KernelResult<String> {
        Ok(format!("#<platform-site {}>", self.symbol))
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl sim_kernel::ObjectCompat for PlatformSite {
    fn class(&self, cx: &mut Cx) -> KernelResult<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "EvalFabric"))
        {
            return Ok(value.clone());
        }
        DefaultFactory.class_stub(
            sim_kernel::CORE_EVAL_REQUEST_CLASS_ID,
            Symbol::qualified("core", "EvalFabric"),
        )
    }
    fn as_eval_fabric(&self) -> Option<&dyn EvalFabric> {
        Some(self)
    }
}

#[must_use]
pub fn bindings_for_site(bound: &BoundServices) -> Vec<OpenSymbol> {
    bound
        .bindings
        .iter()
        .map(|binding| binding.bound.clone())
        .collect()
}
