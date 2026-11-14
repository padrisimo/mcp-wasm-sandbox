//! Stdio entry point for the MCP server.

use std::net::SocketAddr;

use mcp_server::{Dispatcher, serve_http, serve_stdio};
use sandbox_core::{Sandbox, SandboxConfig};

/// Default address of the HTTP transport.
const DEFAULT_HTTP_ADDRESS: &str = "127.0.0.1:3000";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Logs go to stderr: stdout carries the JSON-RPC stream.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let sandbox = Sandbox::new(SandboxConfig::default())?;
    let dispatcher = Dispatcher::new(sandbox);

    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--http") => {
            let address: SocketAddr = args
                .next()
                .unwrap_or_else(|| DEFAULT_HTTP_ADDRESS.to_string())
                .parse()?;
            serve_http(address, dispatcher).await?;
        }
        _ => serve_stdio(dispatcher).await?,
    }

    Ok(())
}
