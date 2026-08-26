// conformance: product runtime composition keeps the platform membrane explicit.

use std::{sync::Arc, time::Duration};

use sim_kernel::{
    AbiVersion, Args, Callable, CapabilityName, Consistency, Cx, Error, EvalMode, EvalRequest,
    Export, Expr, Lib, LibManifest, LibTarget, Linker, LoadCx, Object, ObjectCompat, Result,
    Symbol, Value, Version,
};

/// The lexical binding holding the platform site for the current realization.
///
/// The binding is deliberately neither a registry singleton nor a target lookup:
/// child realizations can replace it without changing the loaded API.
pub const ACTIVE_PLATFORM_SITE: &str = "active-site";

/// The thirteen operations in the Small SIM API. This enum is intentionally closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformFunctionKind {
    Card,
    Require,
    Mounts,
    Open,
    Share,
    Notify,
    ClipboardRead,
    ClipboardWrite,
    PermissionStatus,
    PermissionRequest,
    KeepAwake,
    Lifecycle,
    Activations,
}

impl PlatformFunctionKind {
    /// Every platform API operation, in the public contract order.
    pub const ALL: [Self; 13] = [
        Self::Card,
        Self::Require,
        Self::Mounts,
        Self::Open,
        Self::Share,
        Self::Notify,
        Self::ClipboardRead,
        Self::ClipboardWrite,
        Self::PermissionStatus,
        Self::PermissionRequest,
        Self::KeepAwake,
        Self::Lifecycle,
        Self::Activations,
    ];

    #[must_use]
    pub fn symbol(self) -> Symbol {
        Symbol::qualified(
            "platform",
            match self {
                Self::Card => "card",
                Self::Require => "require",
                Self::Mounts => "mounts",
                Self::Open => "open",
                Self::Share => "share",
                Self::Notify => "notify",
                Self::ClipboardRead => "clipboard-read",
                Self::ClipboardWrite => "clipboard-write",
                Self::PermissionStatus => "permission-status",
                Self::PermissionRequest => "permission-request",
                Self::KeepAwake => "keep-awake",
                Self::Lifecycle => "lifecycle",
                Self::Activations => "activations",
            },
        )
    }

    fn arity(self) -> usize {
        match self {
            Self::Card | Self::Mounts | Self::Lifecycle | Self::Activations => 0,
            Self::Require
            | Self::Open
            | Self::Share
            | Self::Notify
            | Self::ClipboardRead
            | Self::ClipboardWrite
            | Self::PermissionStatus
            | Self::PermissionRequest
            | Self::KeepAwake => 1,
        }
    }

    fn capability(self, args: &[Expr]) -> Result<Option<CapabilityName>> {
        let fixed = match self {
            Self::Open => Some("platform.open"),
            Self::Share => Some("platform.share"),
            Self::Notify => Some("platform.notify"),
            Self::ClipboardRead => Some("platform.clipboard.read"),
            Self::ClipboardWrite => Some("platform.clipboard.write"),
            Self::KeepAwake => Some("platform.power.keep-awake"),
            _ => None,
        };
        if let Some(name) = fixed {
            return Ok(Some(CapabilityName::new(name)));
        }
        if self == Self::PermissionRequest {
            let Expr::Symbol(service) = &args[0] else {
                return Err(Error::Eval(
                    "platform/permission-request expects a service symbol".to_owned(),
                ));
            };
            let name = service.to_string().replace(['/', '-'], ".");
            return Ok(Some(CapabilityName::new(name)));
        }
        Ok(None)
    }
}

/// Returns the exact public function symbols; useful to loaders and conformance tests.
#[must_use]
pub fn platform_function_symbols() -> Vec<Symbol> {
    PlatformFunctionKind::ALL
        .iter()
        .map(|kind| kind.symbol())
        .collect()
}

/// Loadable Small SIM API library.
pub struct PlatformLib;

impl Lib for PlatformLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("sim", "platform"),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: platform_function_symbols()
                .into_iter()
                .map(|symbol| Export::Function {
                    symbol,
                    function_id: None,
                })
                .collect(),
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        for kind in PlatformFunctionKind::ALL {
            linker.function_value(
                kind.symbol(),
                cx.factory().opaque(Arc::new(PlatformFunction(kind)))?,
            )?;
        }
        Ok(())
    }
}

/// Install the platform facade once in a context.
///
/// # Errors
/// Returns a kernel load error when the manifest or one of its function exports
/// cannot be registered atomically.
pub fn install_platform_lib(cx: &mut Cx) -> Result<()> {
    if cx
        .registry()
        .lib(&Symbol::qualified("sim", "platform"))
        .is_none()
    {
        cx.load_lib(&PlatformLib)?;
    }
    Ok(())
}

/// One callable from the closed platform surface.
#[derive(Clone, Copy, Debug)]
pub struct PlatformFunction(pub PlatformFunctionKind);

impl Object for PlatformFunction {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<function {}>", self.0.symbol()))
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for PlatformFunction {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for PlatformFunction {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let values = args.into_vec();
        if values.len() != self.0.arity() {
            return Err(Error::Eval(format!(
                "{} expects {} argument(s), found {}",
                self.0.symbol(),
                self.0.arity(),
                values.len()
            )));
        }
        let exprs = values
            .into_iter()
            .map(|value| value.object().as_expr(cx))
            .collect::<Result<Vec<_>>>()?;
        // Capability precedes provider permission handling, so an unprivileged
        // caller cannot use the OS prompt as a capability oracle.
        if let Some(capability) = self.0.capability(&exprs)? {
            cx.require(&capability)?;
        }
        let site = cx
            .env()
            .get(&Symbol::qualified("platform", ACTIVE_PLATFORM_SITE))
            .ok_or_else(|| {
                Error::Eval(
                    "no PlatformSite is bound in the current realization environment".to_owned(),
                )
            })?;
        let fabric = site.object().as_eval_fabric().ok_or(Error::TypeMismatch {
            expected: "platform-site EvalFabric",
            found: "non-eval-fabric",
        })?;
        let required_capabilities = self.0.capability(&exprs)?.into_iter().collect();
        let reply = fabric.realize(
            cx,
            EvalRequest {
                expr: Expr::Call {
                    operator: Box::new(Expr::Symbol(self.0.symbol())),
                    args: exprs,
                },
                result_shape: None,
                required_capabilities,
                deadline: Some(Duration::from_secs(30)),
                consistency: Consistency::LocalOnly,
                mode: EvalMode::Eval,
                answer_limit: Some(1),
                stream_buffer: None,
                stream: false,
                trace: true,
            },
        )?;
        Ok(reply.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_kernel::{EvalFabric, EvalReply};
    use std::sync::Mutex;

    struct RecordingSite(Mutex<Vec<Expr>>);
    impl Object for RecordingSite {
        fn display(&self, _: &mut Cx) -> Result<String> {
            Ok("#<recording-platform-site>".into())
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
    impl ObjectCompat for RecordingSite {
        fn as_eval_fabric(&self) -> Option<&dyn EvalFabric> {
            Some(self)
        }
    }
    impl EvalFabric for RecordingSite {
        fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
            self.0.lock().unwrap().push(request.expr.clone());
            Ok(EvalReply {
                value: cx.factory().expr(request.expr)?,
                diagnostics: Vec::new(),
                trace: None,
            })
        }
    }

    fn cx_with(site: Arc<RecordingSite>) -> Cx {
        let mut cx = sim_kernel::testing::bare_cx();
        let value = cx.factory().opaque(site).unwrap();
        cx.env_mut()
            .define(Symbol::qualified("platform", ACTIVE_PLATFORM_SITE), value);
        cx
    }

    #[test]
    fn manifest_is_the_closed_thirteen_function_surface() {
        let symbols = platform_function_symbols();
        assert_eq!(symbols.len(), 13);
        assert_eq!(
            symbols.iter().map(ToString::to_string).collect::<Vec<_>>(),
            vec![
                "platform/card",
                "platform/require",
                "platform/mounts",
                "platform/open",
                "platform/share",
                "platform/notify",
                "platform/clipboard-read",
                "platform/clipboard-write",
                "platform/permission-status",
                "platform/permission-request",
                "platform/keep-awake",
                "platform/lifecycle",
                "platform/activations",
            ]
        );
    }

    #[test]
    fn dispatch_uses_only_the_current_environment_site() {
        let outer = Arc::new(RecordingSite(Mutex::new(Vec::new())));
        let inner = Arc::new(RecordingSite(Mutex::new(Vec::new())));
        let mut cx = cx_with(outer.clone());
        let mut child = sim_kernel::Env::child(Arc::new(cx.env().clone()));
        child.define(
            Symbol::qualified("platform", ACTIVE_PLATFORM_SITE),
            cx.factory().opaque(inner.clone()).unwrap(),
        );
        cx.with_env(child, |cx| {
            PlatformFunction(PlatformFunctionKind::Card).call(cx, Args::default())
        })
        .unwrap();
        assert!(outer.0.lock().unwrap().is_empty());
        assert_eq!(inner.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn capability_is_checked_before_provider_permission_dispatch() {
        let site = Arc::new(RecordingSite(Mutex::new(Vec::new())));
        let mut cx = cx_with(site.clone());
        let arg = cx
            .factory()
            .symbol(Symbol::qualified("platform", "clipboard-read"))
            .unwrap();
        let error = PlatformFunction(PlatformFunctionKind::PermissionRequest)
            .call(&mut cx, Args::new(vec![arg]))
            .unwrap_err();
        assert!(matches!(error, Error::CapabilityDenied { .. }));
        assert!(site.0.lock().unwrap().is_empty());
        cx.grant_named("platform.clipboard.read");
        let arg = cx
            .factory()
            .symbol(Symbol::qualified("platform", "clipboard-read"))
            .unwrap();
        PlatformFunction(PlatformFunctionKind::PermissionRequest)
            .call(&mut cx, Args::new(vec![arg]))
            .unwrap();
        assert_eq!(site.0.lock().unwrap().len(), 1);
    }
}
