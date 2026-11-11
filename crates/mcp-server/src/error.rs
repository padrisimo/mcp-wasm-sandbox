//! Crate-level errors for the MCP wire layer.

/// Errors raised by the MCP wire layer: serialization and transport I/O.
#[derive(Debug, thiserror::Error)]
pub enum McpWireError {
    /// A JSON value could not be serialized.
    #[error("json-rpc serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    /// The underlying stream failed.
    #[error("transport i/o error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::McpWireError;

    #[test]
    fn wire_error_display() -> Result<(), Box<dyn std::error::Error>> {
        let io = McpWireError::from(std::io::Error::other("boom"));
        assert_eq!(io.to_string(), "transport i/o error: boom");

        let Err(error) = serde_json::from_str::<serde_json::Value>("{oops}") else {
            return Err("invalid JSON unexpectedly parsed".into());
        };
        let serde_error = McpWireError::from(error);
        assert!(
            serde_error
                .to_string()
                .starts_with("json-rpc serialization error:")
        );

        Ok(())
    }
}
