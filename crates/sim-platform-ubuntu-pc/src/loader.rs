//! Ubuntu realization of the run-owned loader domain port.

use sim_kernel::{Cx, Error, LibLoader, Result, Symbol};
use sim_run_loaders::{
    BinaryPackLoader, LispSourceLoader, LoadOutcome, LoadRequest, LoaderKind, LoaderPort,
    NativeDylibLoader, StaticRegistry, WasmLoader,
};
use std::sync::Arc;

const NATIVE: &str = "native-v1";
const WASM: &str = "wasm-v1";
const SOURCE: &str = "source-v1";
const STATIC: &str = "static-v1";
fn kind(name: &str) -> LoaderKind {
    LoaderKind::new(Symbol::qualified("loader", name))
}

/// Full loader service installed by an Ubuntu capsule after bootstrap.
pub struct UbuntuLoaderPort {
    native: NativeDylibLoader,
    wasm: WasmLoader,
    source: LispSourceLoader,
    binary: BinaryPackLoader,
    static_registry: StaticRegistry,
}

impl Default for UbuntuLoaderPort {
    fn default() -> Self {
        Self {
            native: NativeDylibLoader,
            wasm: WasmLoader::new(Arc::new(sim_wasm_abi::WasmiRuntime::new())),
            source: LispSourceLoader::default(),
            binary: BinaryPackLoader,
            static_registry: StaticRegistry::default(),
        }
    }
}

impl UbuntuLoaderPort {
    /// Registers an exact AOT artifact factory as capsule data.
    pub fn register_static(
        &self,
        artifact: Symbol,
        factory: impl Fn() -> Box<dyn sim_kernel::Lib> + Send + Sync + 'static,
    ) {
        self.static_registry.register(artifact, factory);
    }
    fn mechanism(&self, request: &LoadRequest) -> Result<&dyn LibLoader> {
        if request.kind == kind(NATIVE) {
            Ok(&self.native)
        } else if request.kind == kind(WASM) {
            Ok(&self.wasm)
        } else if request.kind == kind(SOURCE) && self.source.can_load(&request.source) {
            Ok(&self.source)
        } else if request.kind == kind(SOURCE) && self.binary.can_load(&request.source) {
            Ok(&self.binary)
        } else {
            Err(Error::HostError(format!(
                "Ubuntu capsule rejected loader kind {} or its exact source",
                request.kind.symbol()
            )))
        }
    }
}

impl LoaderPort for UbuntuLoaderPort {
    fn loader_kinds(&self) -> Vec<LoaderKind> {
        [NATIVE, WASM, SOURCE, STATIC]
            .into_iter()
            .map(kind)
            .collect()
    }
    fn realize(&self, cx: &mut Cx, request: LoadRequest) -> Result<LoadOutcome> {
        if request.kind == kind(STATIC) {
            let artifact = sim_run_loaders::static_artifact(&request.source)?.ok_or_else(|| {
                Error::HostError("Ubuntu static loader rejected the exact source".into())
            })?;
            return self.static_registry.realize(&artifact);
        }
        let mechanism = self.mechanism(&request)?;
        if !mechanism.can_load(&request.source) {
            return Err(Error::HostError(format!(
                "loader kind {} rejected the exact source",
                request.kind.symbol()
            )));
        }
        let library = mechanism.load(cx, request.source)?;
        let manifest = library.manifest();
        Ok(LoadOutcome { manifest, library })
    }
    fn inspect(
        &self,
        cx: &mut Cx,
        request: &LoadRequest,
    ) -> Result<Option<sim_kernel::LibManifest>> {
        if request.kind == kind(STATIC) {
            let artifact = sim_run_loaders::static_artifact(&request.source)?.ok_or_else(|| {
                Error::HostError("Ubuntu static loader rejected the exact source".into())
            })?;
            return self
                .static_registry
                .realize(&artifact)
                .map(|outcome| Some(outcome.manifest));
        }
        self.mechanism(request)?
            .inspect_manifest(cx, &request.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_kernel::{
        AbiVersion, DefaultFactory, HandleSeed, Lib, LibManifest, LibSource, LibTarget, Linker,
        LoadCx, NoopEvalPolicy, Version,
    };
    fn cx() -> Cx {
        Cx::new(
            Arc::new(NoopEvalPolicy),
            Arc::new(DefaultFactory),
            HandleSeed::new(1),
        )
    }
    struct TestLib;
    impl Lib for TestLib {
        fn manifest(&self) -> LibManifest {
            LibManifest {
                id: Symbol::qualified("test", "ubuntu-static"),
                version: Version("1.0.0".into()),
                abi: AbiVersion { major: 0, minor: 1 },
                target: LibTarget::HostRegistered,
                requires: vec![],
                capabilities: vec![],
                exports: vec![],
            }
        }
        fn load(&self, _: &mut LoadCx, _: &mut Linker<'_>) -> Result<()> {
            Ok(())
        }
    }
    #[test]
    fn advertises_exact_kinds_and_fails_closed() {
        let port = UbuntuLoaderPort::default();
        assert_eq!(
            port.loader_kinds(),
            [NATIVE, WASM, SOURCE, STATIC]
                .into_iter()
                .map(kind)
                .collect::<Vec<_>>()
        );
        let mut cx = cx();
        assert!(
            port.realize(
                &mut cx,
                LoadRequest {
                    kind: kind("invented-v1"),
                    source: sim_run_loaders::static_source(Symbol::qualified("artifact", "x"))
                }
            )
            .is_err()
        );
        assert!(
            port.realize(
                &mut cx,
                LoadRequest {
                    kind: kind(STATIC),
                    source: LibSource::open(
                        Symbol::qualified("loader", "static-artifact"),
                        sim_kernel::Datum::Bytes(vec![])
                    )
                }
            )
            .is_err()
        );
    }
    #[test]
    fn static_libraries_keep_ordinary_manifest_behavior() {
        let port = UbuntuLoaderPort::default();
        let artifact = Symbol::qualified("artifact", "ubuntu-static");
        port.register_static(artifact.clone(), || Box::new(TestLib));
        let mut cx = cx();
        let outcome = port
            .realize(
                &mut cx,
                LoadRequest {
                    kind: kind(STATIC),
                    source: sim_run_loaders::static_source(artifact),
                },
            )
            .unwrap();
        assert_eq!(outcome.manifest, outcome.library.manifest());
    }
}
