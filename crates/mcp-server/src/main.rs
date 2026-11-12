//! Stdio entry point for the MCP server.

use mcp_server::{Dispatcher, McpWireError, serve_stdio};

#[tokio::main]
async fn main() -> Result<(), McpWireError> {
    serve_stdio(Dispatcher).await
}
