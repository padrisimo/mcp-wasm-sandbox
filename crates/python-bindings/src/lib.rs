//! Python bindings for the WebAssembly sandbox.
//!
//! The binding exposes one class, [`WasmSandbox`], that runs guest code with a
//! fuel and memory budget. The GIL is always released around the execution so a
//! long running module cannot block the rest of the interpreter.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use sandbox_core::{Sandbox, SandboxConfig};

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
}

/// Runs the sandbox on its own runtime, blocking the calling thread.
fn run_sandbox(
    sandbox: &Sandbox,
    wasm: &[u8],
    export: &str,
    args: &[i32],
) -> Result<Vec<i32>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())?;

    runtime
        .block_on(sandbox.call_i32(wasm, export, args))
        .map_err(|err| err.to_string())
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

    use super::{Sandbox, SandboxConfig, run_sandbox};

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
