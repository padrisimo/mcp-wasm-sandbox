//! Transport-agnostic JSON-RPC 2.0 envelope and MCP wire payloads.
//!
//! Phase 1 defines the message vocabulary only: the JSON-RPC constants and
//! envelope types, the `initialize`, `tools/list`, and `tools/call` payloads,
//! and the bridge from [`sandbox_core::SandboxError`] to [`JsonRpcError`].
//! The stdio transport and the request dispatcher arrive in Phase 1b.
