//! Stdio entry point for the MCP server.

use mcp_server::{Dispatcher, serve_stdio};
use sandbox_core::{Sandbox, SandboxConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sandbox = Sandbox::new(SandboxConfig::default())?;
    serve_stdio(Dispatcher::new(sandbox)).await?;
    Ok(())
}
