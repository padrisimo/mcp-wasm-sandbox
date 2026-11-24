//! Python bindings for the WebAssembly sandbox.
//!
//! The binding exposes one class, [`WasmSandbox`], that runs guest code with a
//! fuel and memory budget. The GIL is always released around the execution so a
//! long running module cannot block the rest of the interpreter.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use sandbox_core::{DEFAULT_TIMEOUT, Number, Sandbox, SandboxConfig};

/// Sandboxed WebAssembly execution exposed to Python.
#[pyclass(module = "mcp_wasm_sandbox")]
pub struct WasmSandbox {
    sandbox: Sandbox,
}

#[pymethods]
impl WasmSandbox {
    /// Creates a sandbox with the given linear memory ceiling and fuel budget.
    #[new]
    fn new(max_memory_bytes: usize, fuel: u64) -> PyResult<Self> {
        let sandbox = Sandbox::new(SandboxConfig {
            max_memory_bytes,
            fuel,
            timeout: DEFAULT_TIMEOUT,
        })
        .map_err(|err| PyValueError::new_err(err.to_string()))?;

        Ok(Self { sandbox })
    }

    /// Runs an exported function with integer arguments and returns its results.
    ///
    /// The GIL is released while the module runs.
    fn run(
        &self,
        py: Python<'_>,
        wasm: Vec<u8>,
        export: String,
        args: Vec<i32>,
    ) -> PyResult<Vec<i32>> {
        let sandbox = self.sandbox.clone();

        py.detach(move || run_sandbox(&sandbox, &wasm, &export, &args))
            .map_err(PyValueError::new_err)
    }

    /// Runs an exported function and returns its results plus the guest output.
    ///
    /// The first element of the tuple is the results of the call, the second
    /// everything the guest wrote to stdout or stderr.
    ///
    /// The GIL is released while the module runs.
    fn run_with_output(
        &self,
        py: Python<'_>,
        wasm: Vec<u8>,
        export: String,
        args: Vec<i32>,
    ) -> PyResult<(Vec<i32>, String)> {
        let sandbox = self.sandbox.clone();
        let args_numbers: Vec<Number> = args.iter().copied().map(Number::I32).collect();

        py.detach(move || run_with(&sandbox, &wasm, &export, &args_numbers))
            .map_err(PyValueError::new_err)
    }
}

/// Runs the sandbox on its own runtime, blocking the calling thread.
fn run_sandbox(
    sandbox: &Sandbox,
    wasm: &[u8],
    export: &str,
    args: &[i32],
) -> Result<Vec<i32>, String> {
    let numbers: Vec<Number> = args.iter().copied().map(Number::I32).collect();

    run_with(sandbox, wasm, export, &numbers).map(|(results, _)| results)
}

/// Runs the sandbox on its own runtime and keeps the guest output.
fn run_with(
    sandbox: &Sandbox,
    wasm: &[u8],
    export: &str,
    args: &[Number],
) -> Result<(Vec<i32>, String), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())?;

    let output = runtime
        .block_on(sandbox.call_with_output(wasm, export, args))
        .map_err(|err| err.to_string())?;
    let results = output
        .results
        .iter()
        .map(|value| {
            value
                .as_i32()
                .ok_or_else(|| format!("unexpected result: {value:?}"))
        })
        .collect::<Result<Vec<i32>, String>>()?;

    Ok((results, String::from_utf8_lossy(&output.stdout).into_owned()))
}

/// The `mcp_wasm_sandbox` extension module.
#[pymodule]
fn mcp_wasm_sandbox(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<WasmSandbox>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use pyo3::Python;

    use super::{run_sandbox, run_with, Sandbox, SandboxConfig, Number};

    const ADD_MODULE: &str = r#"(module
        (func (export "add") (param i32 i32) (result i32)
            local.get 0
            local.get 1
            i32.add))"#;

    #[test]
    fn the_sandbox_runs_on_a_blocking_runtime() -> Result<(), Box<dyn std::error::Error>> {
        let sandbox = Sandbox::new(SandboxConfig::default())?;

        let results = run_sandbox(&sandbox, ADD_MODULE.as_bytes(), "add", &[2, 3])?;

        assert_eq!(results, vec![5]);
        Ok(())
    }

    #[test]
    fn guest_failures_are_reported_as_errors() -> Result<(), Box<dyn std::error::Error>> {
        let sandbox = Sandbox::new(SandboxConfig::default())?;

        match run_sandbox(&sandbox, b"not a wasm module", "add", &[]) {
            Err(message) => {
                assert!(
                    message.contains("compilation failed"),
                    "unexpected message: {message}"
                );
                Ok(())
            }
            Ok(results) => {
                Err(std::io::Error::other(format!("unexpected results: {results:?}")).into())
            }
        }
    }

    #[test]
    fn the_guest_output_is_returned() -> Result<(), Box<dyn std::error::Error>> {
        const PRINT_MODULE: &str = r#"(module
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "hello\n")
            (func (export "print")
                (i32.store (i32.const 0) (i32.const 100))
                (i32.store (i32.const 4) (i32.const 6))
                (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 20)))))"#;

        let sandbox = Sandbox::new(SandboxConfig::default())?;
        let (results, stdout) = run_with(&sandbox, PRINT_MODULE.as_bytes(), "print", &[])?;

        assert!(results.is_empty());
        assert_eq!(stdout, "hello\n");
        Ok(())
    }

    /// The GIL must be released while the module runs, otherwise a long module
    /// would freeze the whole interpreter.
    ///
    /// Needs a Python interpreter with a shared libpython. Run it with
    /// `cargo test -p mcp-wasm-sandbox -- --ignored`.
    #[test]
    #[ignore = "requires a Python interpreter with a shared libpython"]
    fn the_gil_is_released_while_the_sandbox_runs() -> Result<(), Box<dyn std::error::Error>> {
        use std::time::{Duration, Instant};

        Python::initialize();
        let sandbox = Sandbox::new(SandboxConfig::default())?;
        let worker_sandbox = sandbox.clone();

        let worker = std::thread::spawn(move || {
            Python::attach(|py| {
                let _ = py.detach(|| {
                    std::thread::sleep(Duration::from_millis(200));
                    run_sandbox(&worker_sandbox, ADD_MODULE.as_bytes(), "add", &[2, 3])
                });
            });
        });

        std::thread::sleep(Duration::from_millis(50));
        let started = Instant::now();
        Python::attach(|_| {});
        let waited = started.elapsed();

        worker
            .join()
            .map_err(|_| std::io::Error::other("the worker thread panicked"))?;

        assert!(
            waited < Duration::from_millis(150),
            "the GIL was held for {waited:?}"
        );
        Ok(())
    }
}
