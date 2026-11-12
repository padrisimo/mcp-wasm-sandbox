//! Sandboxed WebAssembly execution under a fuel and memory budget.

use wasmtime::{Engine, Linker, Module, Store, Val};

use crate::SandboxError;
use crate::cache::ModuleCache;

/// Default linear memory ceiling of a sandboxed module: 32 MiB.
pub const DEFAULT_MAX_MEMORY_BYTES: usize = 32 * 1024 * 1024;

/// Default fuel budget granted to a single execution.
pub const DEFAULT_FUEL: u64 = 10_000_000;

/// Resource budget applied to every sandboxed execution.
#[derive(Debug, Clone, Copy)]
pub struct SandboxConfig {
    /// Maximum linear memory a module may allocate, in bytes.
    pub max_memory_bytes: usize,
    /// Fuel budget granted to a single execution.
    pub fuel: u64,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            fuel: DEFAULT_FUEL,
        }
    }
}

/// Store data that enforces the linear memory ceiling.
struct StoreState {
    max_bytes: usize,
    denied: bool,
}

impl wasmtime::ResourceLimiter for StoreState {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        if desired > self.max_bytes {
            self.denied = true;
            // A hard error turns the growth into a trap instead of a silent -1.
            return Err(wasmtime::Error::msg(format!(
                "memory limit exceeded: {desired} bytes requested, {} bytes allowed",
                self.max_bytes
            )));
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: u32,
        _desired: u32,
        _maximum: Option<u32>,
    ) -> Result<bool, wasmtime::Error> {
        Ok(true)
    }
}

/// Executes untrusted WebAssembly under a fixed fuel and memory budget.
#[derive(Clone)]
pub struct Sandbox {
    engine: Engine,
    cache: ModuleCache,
    config: SandboxConfig,
}

impl Sandbox {
    /// Builds a sandbox that enforces `config`.
    pub fn new(config: SandboxConfig) -> Result<Self, SandboxError> {
        let mut wasm_config = wasmtime::Config::new();
        wasm_config.consume_fuel(true);
        wasm_config.wasm_threads(false);
        let engine =
            Engine::new(&wasm_config).map_err(|err| SandboxError::Engine(err.to_string()))?;

        Ok(Self {
            engine,
            cache: ModuleCache::new(),
            config,
        })
    }

    /// The budget this sandbox enforces.
    pub fn config(&self) -> SandboxConfig {
        self.config
    }

    /// Number of modules compiled and cached by this sandbox.
    pub fn cached_modules(&self) -> usize {
        self.cache.len()
    }

    /// Compiles `wasm` once, then runs `export` with `args` on a blocking thread.
    pub async fn call(
        &self,
        wasm: &[u8],
        export: &str,
        args: &[Val],
    ) -> Result<Vec<Val>, SandboxError> {
        let engine = self.engine.clone();
        let cache = self.cache.clone();
        let config = self.config;
        let wasm = wasm.to_vec();
        let export = export.to_string();
        let args = args.to_vec();

        tokio::task::spawn_blocking(move || {
            let module = cache.get_or_compile(&engine, &wasm)?;
            execute(&engine, &module, &config, &export, &args)
        })
        .await
        .map_err(|err| SandboxError::Execution(format!("wasm task failed: {err}")))?
    }

    /// Runs an exported function with `i32` arguments and returns its `i32` results.
    pub async fn call_i32(
        &self,
        wasm: &[u8],
        export: &str,
        args: &[i32],
    ) -> Result<Vec<i32>, SandboxError> {
        let params: Vec<Val> = args.iter().copied().map(Val::I32).collect();
        let results = self.call(wasm, export, &params).await?;

        results
            .iter()
            .map(|value| match value {
                Val::I32(value) => Ok(*value),
                other => Err(SandboxError::Execution(format!(
                    "unexpected result type: {other:?}"
                ))),
            })
            .collect()
    }
}

fn execute(
    engine: &Engine,
    module: &Module,
    config: &SandboxConfig,
    export: &str,
    args: &[Val],
) -> Result<Vec<Val>, SandboxError> {
    let mut store = Store::new(
        engine,
        StoreState {
            max_bytes: config.max_memory_bytes,
            denied: false,
        },
    );
    store.limiter(|state| state);
    store
        .set_fuel(config.fuel)
        .map_err(|err| SandboxError::Execution(err.to_string()))?;

    let linker = Linker::new(engine);
    let instance = linker
        .instantiate(&mut store, module)
        .map_err(|err| SandboxError::Instantiate(err.to_string()))?;

    let func = instance
        .get_func(&mut store, export)
        .ok_or_else(|| SandboxError::Execution(format!("export not found: {export}")))?;

    let mut results = vec![Val::I32(0); func.ty(&store).results().len()];
    match func.call(&mut store, args, &mut results) {
        Ok(()) => Ok(results),
        Err(err) => Err(classify(&store, config, err)),
    }
}

/// Maps a trap to the most specific sandbox error.
fn classify(
    store: &Store<StoreState>,
    config: &SandboxConfig,
    err: wasmtime::Error,
) -> SandboxError {
    if store.data().denied {
        return SandboxError::MemoryLimit {
            limit_bytes: config.max_memory_bytes,
        };
    }

    if let Some(trap) = err.downcast_ref::<wasmtime::Trap>()
        && *trap == wasmtime::Trap::OutOfFuel
    {
        return SandboxError::FuelExhausted;
    }

    match store.get_fuel() {
        Ok(0) => SandboxError::FuelExhausted,
        _ => SandboxError::Execution(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use wasmtime::Val;

    use super::{Sandbox, SandboxConfig};
    use crate::SandboxError;

    const ADD_MODULE: &str = r#"(module
        (func (export "add") (param i32 i32) (result i32)
            local.get 0
            local.get 1
            i32.add))"#;

    const SPIN_MODULE: &str = r#"(module (func (export "spin") (loop br 0)))"#;

    const GROW_MODULE: &str = r#"(module
        (memory (export "mem") 1)
        (func (export "grow") (result i32) (memory.grow (i32.const 1000))))"#;

    const WORK_MODULE: &str = r#"(module
        (func (export "work") (local i32)
            (loop
                (local.set 0 (i32.add (local.get 0) (i32.const 1)))
                (br_if 0 (i32.lt_u (local.get 0) (i32.const 50000000))))))"#;

    #[tokio::test]
    async fn call_add_returns_the_sum() -> Result<(), Box<dyn std::error::Error>> {
        let sandbox = Sandbox::new(SandboxConfig::default())?;

        let results = sandbox
            .call(ADD_MODULE.as_bytes(), "add", &[Val::I32(2), Val::I32(3)])
            .await?;

        assert!(
            matches!(results.as_slice(), [Val::I32(5)]),
            "unexpected results: {results:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn invalid_module_is_a_compile_error() -> Result<(), Box<dyn std::error::Error>> {
        let sandbox = Sandbox::new(SandboxConfig::default())?;

        match sandbox.call(b"not a wasm module", "add", &[]).await {
            Err(SandboxError::Compile(_)) => Ok(()),
            other => Err(std::io::Error::other(format!("unexpected outcome: {other:?}")).into()),
        }
    }

    #[tokio::test]
    async fn missing_export_is_an_execution_error() -> Result<(), Box<dyn std::error::Error>> {
        let sandbox = Sandbox::new(SandboxConfig::default())?;

        match sandbox.call(ADD_MODULE.as_bytes(), "nope", &[]).await {
            Err(SandboxError::Execution(message)) => {
                assert!(
                    message.contains("export not found"),
                    "unexpected message: {message}"
                );
                Ok(())
            }
            other => Err(std::io::Error::other(format!("unexpected outcome: {other:?}")).into()),
        }
    }

    #[tokio::test]
    async fn out_of_fuel_is_reported_as_fuel_exhausted() -> Result<(), Box<dyn std::error::Error>> {
        let sandbox = Sandbox::new(SandboxConfig {
            fuel: 10_000,
            ..SandboxConfig::default()
        })?;

        match sandbox.call(SPIN_MODULE.as_bytes(), "spin", &[]).await {
            Err(SandboxError::FuelExhausted) => Ok(()),
            other => Err(std::io::Error::other(format!("unexpected outcome: {other:?}")).into()),
        }
    }

    #[tokio::test]
    async fn memory_growth_above_the_ceiling_is_rejected() -> Result<(), Box<dyn std::error::Error>>
    {
        let limit = 1024 * 1024;
        let sandbox = Sandbox::new(SandboxConfig {
            max_memory_bytes: limit,
            ..SandboxConfig::default()
        })?;

        match sandbox.call(GROW_MODULE.as_bytes(), "grow", &[]).await {
            Err(SandboxError::MemoryLimit { limit_bytes }) => {
                assert_eq!(limit_bytes, limit);
                Ok(())
            }
            other => Err(std::io::Error::other(format!("unexpected outcome: {other:?}")).into()),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execution_leaves_the_async_runtime_free() -> Result<(), Box<dyn std::error::Error>> {
        let sandbox = Sandbox::new(SandboxConfig {
            fuel: 100_000_000_000,
            ..SandboxConfig::default()
        })?;
        let done = Arc::new(AtomicBool::new(false));

        let setter = Arc::clone(&done);
        tokio::spawn(async move {
            setter.store(true, Ordering::SeqCst);
        });

        let _ = sandbox.call(WORK_MODULE.as_bytes(), "work", &[]).await;

        assert!(
            done.load(Ordering::SeqCst),
            "the wasm execution blocked the async runtime"
        );
        Ok(())
    }

    #[tokio::test]
    async fn compiled_modules_are_cached() -> Result<(), Box<dyn std::error::Error>> {
        let sandbox = Sandbox::new(SandboxConfig::default())?;

        let _ = sandbox
            .call(ADD_MODULE.as_bytes(), "add", &[Val::I32(1), Val::I32(1)])
            .await?;
        let _ = sandbox
            .call(ADD_MODULE.as_bytes(), "add", &[Val::I32(2), Val::I32(2)])
            .await?;

        assert_eq!(sandbox.cached_modules(), 1);
        Ok(())
    }
}
