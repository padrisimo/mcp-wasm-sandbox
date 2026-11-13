//! Sandboxed WebAssembly execution under a fuel and memory budget.

use wasmtime::{Engine, Linker, Module, Store, Val, ValType};
use wasmtime_wasi::I32Exit;
use wasmtime_wasi::preview1;

use crate::SandboxError;
use crate::cache::ModuleCache;
use crate::wasi::WasiState;

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

/// A number passed to, or returned by, a guest function.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    /// 32 bit integer.
    I32(i32),
    /// 64 bit integer.
    I64(i64),
    /// 32 bit float.
    F32(f32),
    /// 64 bit float.
    F64(f64),
}

impl Number {
    /// The number as an `i32`, when it fits.
    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Number::I32(value) => Some(*value),
            Number::I64(value) => i32::try_from(*value).ok(),
            Number::F32(value) => Some(*value as i32),
            Number::F64(value) => Some(*value as i32),
        }
    }

    /// The number as text.
    pub fn to_text(&self) -> String {
        match self {
            Number::I32(value) => value.to_string(),
            Number::I64(value) => value.to_string(),
            Number::F32(value) => value.to_string(),
            Number::F64(value) => value.to_string(),
        }
    }
}

/// Converts a number to the type the guest expects.
fn coerce(value: Number, ty: &ValType) -> Result<Val, String> {
    match ty {
        ValType::I32 => value
            .as_i32()
            .map(Val::I32)
            .ok_or_else(|| format!("{value:?} does not fit an i32")),
        ValType::I64 => Ok(Val::I64(match value {
            Number::I32(value) => i64::from(value),
            Number::I64(value) => value,
            Number::F32(value) => value as i64,
            Number::F64(value) => value as i64,
        })),
        ValType::F32 => Ok(Val::F32(match value {
            Number::I32(value) => (value as f32).to_bits(),
            Number::I64(value) => (value as f32).to_bits(),
            Number::F32(value) => value.to_bits(),
            Number::F64(value) => (value as f32).to_bits(),
        })),
        ValType::F64 => Ok(Val::F64(match value {
            Number::I32(value) => f64::from(value).to_bits(),
            Number::I64(value) => (value as f64).to_bits(),
            Number::F32(value) => f64::from(value).to_bits(),
            Number::F64(value) => value.to_bits(),
        })),
        _ => Err(format!("unsupported argument type: {ty:?}")),
    }
}

/// Converts a value returned by the guest into a number.
fn to_number(value: &Val) -> Option<Number> {
    match value {
        Val::I32(value) => Some(Number::I32(*value)),
        Val::I64(value) => Some(Number::I64(*value)),
        Val::F32(bits) => Some(Number::F32(f32::from_bits(*bits))),
        Val::F64(bits) => Some(Number::F64(f64::from_bits(*bits))),
        _ => None,
    }
}

/// Store data that enforces the linear memory ceiling.
struct StoreState {
    max_bytes: usize,
    denied: bool,
    wasi: WasiState,
}

impl StoreState {
    /// The WASI context of the guest, without any host access.
    fn wasi_ctx(&mut self) -> &mut wasmtime_wasi::preview1::WasiP1Ctx {
        self.wasi.ctx()
    }
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
    ///
    /// The arguments are converted to the types the exported function expects.
    pub async fn call(
        &self,
        wasm: &[u8],
        export: &str,
        args: &[Number],
    ) -> Result<Vec<Number>, SandboxError> {
        Ok(self.call_with_output(wasm, export, args).await?.results)
    }

    /// Runs an exported function with `i32` arguments and returns its `i32` results.
    pub async fn call_i32(
        &self,
        wasm: &[u8],
        export: &str,
        args: &[i32],
    ) -> Result<Vec<i32>, SandboxError> {
        let numbers: Vec<Number> = args.iter().copied().map(Number::I32).collect();
        let results = self.call(wasm, export, &numbers).await?;

        results
            .iter()
            .map(|value| {
                value
                    .as_i32()
                    .ok_or_else(|| SandboxError::Execution(format!("unexpected result: {value:?}")))
            })
            .collect()
    }

    /// Runs an exported function and returns its results plus the guest output.
    pub async fn call_with_output(
        &self,
        wasm: &[u8],
        export: &str,
        args: &[Number],
    ) -> Result<SandboxOutput, SandboxError> {
        let engine = self.engine.clone();
        let cache = self.cache.clone();
        let config = self.config;
        let wasm = wasm.to_vec();
        let export = export.to_string();
        let args = args.to_vec();

        tokio::task::spawn_blocking(move || {
            let module = cache.get_or_compile(&engine, &wasm)?;
            run(&engine, &module, &config, &export, &args)
        })
        .await
        .map_err(|err| SandboxError::Execution(format!("wasm task failed: {err}")))?
        .map(|(results, stdout)| SandboxOutput { results, stdout })
    }
}

/// The outcome of a sandboxed execution.
#[derive(Debug)]
pub struct SandboxOutput {
    /// Values returned by the exported function.
    pub results: Vec<Number>,
    /// Everything the guest wrote to stdout or stderr.
    pub stdout: Vec<u8>,
}

fn run(
    engine: &Engine,
    module: &Module,
    config: &SandboxConfig,
    export: &str,
    args: &[Number],
) -> Result<(Vec<Number>, Vec<u8>), SandboxError> {
    let mut store = Store::new(
        engine,
        StoreState {
            max_bytes: config.max_memory_bytes,
            denied: false,
            wasi: WasiState::new(),
        },
    );
    store.limiter(|state| state);
    store
        .set_fuel(config.fuel)
        .map_err(|err| SandboxError::Execution(err.to_string()))?;

    let mut linker = Linker::new(engine);
    preview1::add_to_linker_sync(&mut linker, |state: &mut StoreState| state.wasi_ctx())
        .map_err(|err| SandboxError::Execution(err.to_string()))?;

    let instance = linker
        .instantiate(&mut store, module)
        .map_err(|err| SandboxError::Instantiate(err.to_string()))?;

    let func = instance
        .get_func(&mut store, export)
        .ok_or_else(|| SandboxError::Execution(format!("export not found: {export}")))?;

    let signature = func.ty(&store);
    let expected: Vec<ValType> = signature.params().collect();
    if expected.len() != args.len() {
        return Err(SandboxError::Execution(format!(
            "export {export} expects {} arguments, got {}",
            expected.len(),
            args.len()
        )));
    }

    let params = expected
        .iter()
        .zip(args.iter())
        .map(|(ty, value)| {
            coerce(*value, ty).map_err(|message| SandboxError::Execution(message.clone()))
        })
        .collect::<Result<Vec<Val>, SandboxError>>()?;

    let mut results = vec![Val::I32(0); signature.results().len()];
    let outcome = func.call(&mut store, &params, &mut results);
    let stdout = store.data().wasi.output().contents();

    match outcome {
        Ok(()) => {
            let numbers = results
                .iter()
                .map(|value| {
                    to_number(value).ok_or_else(|| {
                        SandboxError::Execution(format!("unexpected result: {value:?}"))
                    })
                })
                .collect::<Result<Vec<Number>, SandboxError>>()?;
            Ok((numbers, stdout))
        }
        Err(err) => {
            // A guest that exits with code zero is not a failure.
            if let Some(exit) = err.downcast_ref::<I32Exit>()
                && exit.0 == 0
            {
                return Ok((Vec::new(), stdout));
            }
            Err(classify(&store, config, err))
        }
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

    use super::{Number, Sandbox, SandboxConfig};
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

    const PRINT_MODULE: &str = r#"(module
        (import "wasi_snapshot_preview1" "fd_write"
            (func $fd_write (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 100) "hello\n")
        (func (export "print")
            (i32.store (i32.const 0) (i32.const 100))
            (i32.store (i32.const 4) (i32.const 6))
            (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 20)))))"#;

    const I64_MODULE: &str = r#"(module
        (func (export "double") (param i64) (result i64)
            local.get 0
            i64.const 2
            i64.mul))"#;

    #[tokio::test]
    async fn call_add_returns_the_sum() -> Result<(), Box<dyn std::error::Error>> {
        let sandbox = Sandbox::new(SandboxConfig::default())?;

        let results = sandbox
            .call(
                ADD_MODULE.as_bytes(),
                "add",
                &[Number::I32(2), Number::I32(3)],
            )
            .await?;

        assert!(
            matches!(results.as_slice(), [Number::I32(5)]),
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
    async fn arguments_follow_the_function_signature() -> Result<(), Box<dyn std::error::Error>> {
        let sandbox = Sandbox::new(SandboxConfig::default())?;

        let results = sandbox
            .call(I64_MODULE.as_bytes(), "double", &[Number::I32(21)])
            .await?;

        assert_eq!(results, vec![Number::I64(42)]);
        Ok(())
    }

    #[tokio::test]
    async fn the_argument_count_must_match() -> Result<(), Box<dyn std::error::Error>> {
        let sandbox = Sandbox::new(SandboxConfig::default())?;

        match sandbox.call(I64_MODULE.as_bytes(), "double", &[]).await {
            Err(SandboxError::Execution(message)) => {
                assert!(
                    message.contains("expects 1 arguments"),
                    "unexpected: {message}"
                );
                Ok(())
            }
            other => Err(std::io::Error::other(format!("unexpected outcome: {other:?}")).into()),
        }
    }

    #[tokio::test]
    async fn guest_stdout_is_captured() -> Result<(), Box<dyn std::error::Error>> {
        let sandbox = Sandbox::new(SandboxConfig::default())?;

        let output = sandbox
            .call_with_output(PRINT_MODULE.as_bytes(), "print", &[])
            .await?;

        assert_eq!(output.stdout, b"hello\n".to_vec());
        Ok(())
    }

    #[tokio::test]
    async fn compiled_modules_are_cached() -> Result<(), Box<dyn std::error::Error>> {
        let sandbox = Sandbox::new(SandboxConfig::default())?;

        let _ = sandbox
            .call(
                ADD_MODULE.as_bytes(),
                "add",
                &[Number::I32(1), Number::I32(1)],
            )
            .await?;
        let _ = sandbox
            .call(
                ADD_MODULE.as_bytes(),
                "add",
                &[Number::I32(2), Number::I32(2)],
            )
            .await?;

        assert_eq!(sandbox.cached_modules(), 1);
        Ok(())
    }
}
