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

    let mut config = SandboxConfig::default();
    let mut http = None;
    let mut args = std::env::args().skip(1);

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--http" => {
                let address = args
                    .next()
                    .unwrap_or_else(|| DEFAULT_HTTP_ADDRESS.to_string());
                http = Some(address.parse::<SocketAddr>()?);
            }
            "--fuel" => {
                config.fuel = args
                    .next()
                    .ok_or("--fuel needs a value")?
                    .parse()
                    .map_err(|_| "--fuel needs a whole number of units")?;
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    let sandbox = Sandbox::new(config)?;

    match http {
        // The HTTP transport has no session, so it builds a handler per request.
        Some(address) => {
            serve_http(address, move || Dispatcher::stateless(sandbox.clone())).await?
        }
        None => serve_stdio(Dispatcher::new(sandbox)).await?,
    }

    Ok(())
}
