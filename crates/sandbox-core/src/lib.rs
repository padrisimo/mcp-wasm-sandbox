//! Core execution engine for the MCP WebAssembly sandbox.
//!
//! Phase 1 defines [`SandboxError`], the domain error type shared by the
//! workspace. Engine configuration, the module cache, and Wasmtime execution
//! arrive in a later phase.

/// Domain errors raised by the Wasm sandbox.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    /// WebAssembly compilation failed.
    #[error("wasm compilation failed: {0}")]
    Compile(String),
    /// WebAssembly instantiation failed.
    #[error("wasm instantiation failed: {0}")]
    Instantiate(String),
    /// WebAssembly execution failed, for example on a guest trap.
    #[error("wasm execution failed: {0}")]
    Execution(String),
    /// Execution used its whole fuel budget.
    #[error("wasm execution exhausted its fuel budget")]
    FuelExhausted,
    /// A linear memory allocation exceeded the byte ceiling.
    #[error("wasm memory limit exceeded: {limit_bytes} bytes")]
    MemoryLimit {
        /// Ceiling that the allocation would have exceeded.
        limit_bytes: usize,
    },
    /// The module bytes are not a valid WebAssembly module.
    #[error("invalid wasm module: {0}")]
    InvalidModule(String),
    /// Host I/O failed.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// Serialization failed; the message carries the cause.
    #[error("serialization error: {0}")]
    Serialization(String),
    /// The operation is not supported by this sandbox.
    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

#[cfg(test)]
mod tests {
    use super::SandboxError;

    #[test]
    fn fuel_exhausted_display() -> Result<(), Box<dyn std::error::Error>> {
        let err = SandboxError::FuelExhausted;
        assert_eq!(err.to_string(), "wasm execution exhausted its fuel budget");
        Ok(())
    }

    #[test]
    fn memory_limit_display() -> Result<(), Box<dyn std::error::Error>> {
        let err = SandboxError::MemoryLimit { limit_bytes: 4096 };
        assert_eq!(err.to_string(), "wasm memory limit exceeded: 4096 bytes");
        Ok(())
    }

    #[test]
    fn payload_variant_display() -> Result<(), Box<dyn std::error::Error>> {
        let cases = [
            (
                SandboxError::Compile("bad".to_string()),
                "wasm compilation failed: bad",
            ),
            (
                SandboxError::Instantiate("bad".to_string()),
                "wasm instantiation failed: bad",
            ),
            (
                SandboxError::Execution("trap".to_string()),
                "wasm execution failed: trap",
            ),
            (
                SandboxError::InvalidModule("bad".to_string()),
                "invalid wasm module: bad",
            ),
            (
                SandboxError::Serialization("bad".to_string()),
                "serialization error: bad",
            ),
            (
                SandboxError::Unsupported("x".to_string()),
                "unsupported operation: x",
            ),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }

        Ok(())
    }

    #[test]
    fn io_error_conversion() -> Result<(), Box<dyn std::error::Error>> {
        let err = SandboxError::from(std::io::Error::other("boom"));
        assert_eq!(err.to_string(), "i/o error: boom");
        assert!(matches!(&err, SandboxError::Io(_)));
        Ok(())
    }

    #[test]
    fn variant_matching() -> Result<(), Box<dyn std::error::Error>> {
        let fuel = SandboxError::FuelExhausted;
        assert!(matches!(fuel, SandboxError::FuelExhausted));

        let memory = SandboxError::MemoryLimit { limit_bytes: 4096 };
        assert!(matches!(memory, SandboxError::MemoryLimit { limit_bytes: 4096 }));

        let io = SandboxError::from(std::io::Error::other("boom"));
        assert!(matches!(&io, SandboxError::Io(_)));

        Ok(())
    }
}
