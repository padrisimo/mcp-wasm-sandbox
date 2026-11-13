//! Stdio entry point for the MCP server.

use mcp_server::{Dispatcher, serve_stdio};
use sandbox_core::{Sandbox, SandboxConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Logs go to stderr: stdout carries the JSON-RPC stream.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let sandbox = Sandbox::new(SandboxConfig::default())?;
    serve_stdio(Dispatcher::new(sandbox)).await?;
    Ok(())
}
