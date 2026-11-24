# Phase 1: Domain Errors and JSON-RPC 2.0 / MCP Wire Types Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers-subagent-driven-development (recommended) or superpowers-executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `SandboxError` to `sandbox-core` and the transport-agnostic JSON-RPC 2.0 envelope plus typed MCP payloads to `mcp-server`, tests first, with no new dependencies.

**Architecture:** One module per crate: `SandboxError` at the root of `crates/sandbox-core/src/lib.rs`, and all wire types at the root of `crates/mcp-server/src/lib.rs`. `mcp-server` already depends on `sandbox-core`, so the `From<SandboxError> for JsonRpcError` bridge lives in `mcp-server`. `sandbox-core` gains no new dependencies and stays `serde`-free.

**Tech Stack:** Rust 1.99.1, edition 2024, `thiserror` 1.0, `serde` 1.0 (derive), `serde_json` 1.0. All already declared in the workspace and crate manifests.

**Spec:** `docs/superpowers/specs/2026-11-11-phase-1-error-types-jsonrpc-mcp-wire-design.md` (approved).

---

## Context and preconditions

Repository state when this plan starts:

- `Cargo.toml` (workspace root) lists three members: `crates/sandbox-core`, `crates/mcp-server`, `crates/python-bindings`.
- `crates/sandbox-core/Cargo.toml` and `crates/mcp-server/Cargo.toml` exist. **Neither crate has a `src/` directory**, so the workspace cannot load yet.
- `crates/sandbox-core` declares `wasmtime`, `thiserror`, `tracing`, and `tokio` (feature `rt`). It deliberately has no `serde`/`serde_json`.
- `crates/mcp-server` declares `sandbox-core` (path), `tokio`, `serde`, `serde_json`, `thiserror`, `tracing`, `tracing-subscriber`, and `axum`.
- `crates/python-bindings` contains only `pyproject.toml`, no Cargo manifest. **The user approved temporarily removing its line from `[workspace] members`.** Restore it in Phase 3; do not edit `crates/python-bindings/*` in this plan.
- `docs/superpowers/` is ignored by git; the plan file is not committed.

Run every command in this plan from the repository root (the directory that contains the workspace `Cargo.toml`).

Why Task 1 creates minimal crate roots: `cargo metadata` refuses to load any workspace whose members have no targets (`no targets specified in the manifest`). Removing `python-bindings` alone does **not** fix that, because both target crates also lack `src/lib.rs`. Task 1 therefore creates the two crate roots with only their crate-level docs; Tasks 2 and 4 overwrite them with the test modules, and Tasks 3 and 6 add the implementations. No workspace command is run before Task 1 completes.

Constraints that apply to every task:

- **No `unwrap`, `expect`, or `panic!` anywhere, tests included.** Tests return `Result<(), Box<dyn std::error::Error>>` and propagate with `?`; assertions use `assert_eq!`, `assert!`, and `matches!`.
- **Tests first (TDD).** A red run is local evidence only. Never commit a non-compiling or failing tree: tests and the minimum implementation are committed together once green.
- English doc comments and English `Display` strings only.
- JSON comparisons use `serde_json::Value` / `serde_json::json!`, never raw JSON strings.
- No new dependencies, no transport, no `McpWireError`, no parser, no Wasmtime execution.
- Only three files change: the two new `lib.rs` files and the root `Cargo.toml`.

---

## File structure

| File | Action | Responsibility |
| --- | --- | --- |
| `Cargo.toml` | Modify (Task 1) | Remove `"crates/python-bindings",` from `[workspace] members` |
| `crates/sandbox-core/src/lib.rs` | Create (Task 1), overwrite (Tasks 2–3) | Crate docs, `SandboxError`, unit tests |
| `crates/mcp-server/src/lib.rs` | Create (Task 1), overwrite (Task 4), insert (Tasks 5–6) | Crate docs, JSON-RPC constants/envelope, MCP payloads, `From<SandboxError> for JsonRpcError`, unit tests |

---

### Task 1: Toolchain, workspace member removal, crate roots

**Files:**
- Modify: `Cargo.toml:3-7`
- Create: `crates/sandbox-core/src/lib.rs`
- Create: `crates/mcp-server/src/lib.rs`

- [x] **Step 1: Load the Rust toolchain and confirm versions**

Run:

```bash
source "$HOME/.cargo/env"
cargo --version
rustc --version
cargo clippy --version
```

Expected (patch versions may be newer; all must be ≥ 1.85 for edition 2024):

```
cargo 1.99.1 (5f94df478 2026-08-27)
rustc 1.99.1 (58cf8f9c3 2026-10-13)
clippy 0.1.99 (58cf8f9c35 2026-10-13)
```

- [x] **Step 2: Confirm the workspace currently fails to load**

Run:

```bash
cargo metadata --no-deps --format-version 1 > /dev/null; echo "metadata exit: $?"
```

Expected: `metadata exit: 101`, preceded by `failed to load manifest for workspace member` and `no targets specified in the manifest`. This is the broken baseline Task 1 fixes.

- [x] **Step 3: Remove `python-bindings` from the workspace members**

In `Cargo.toml`, replace exactly:

```toml
members = [
    "crates/sandbox-core",
    "crates/mcp-server",
    "crates/python-bindings",
]
```

with exactly:

```toml
members = [
    "crates/sandbox-core",
    "crates/mcp-server",
]
```

Do not touch `[workspace.package]` or `[workspace.dependencies]`. Do not edit anything under `crates/python-bindings/`.

- [x] **Step 4: Create the `sandbox-core` crate root**

Create `crates/sandbox-core/src/lib.rs` with exactly:

```rust
//! Core execution engine for the MCP WebAssembly sandbox.
//!
//! Phase 1 defines [`SandboxError`], the domain error type shared by the
//! workspace. Engine configuration, the module cache, and Wasmtime execution
//! arrive in a later phase.
```

- [x] **Step 5: Create the `mcp-server` crate root**

Create `crates/mcp-server/src/lib.rs` with exactly:

```rust
//! Transport-agnostic JSON-RPC 2.0 envelope and MCP wire payloads.
//!
//! Phase 1 defines the message vocabulary only: the JSON-RPC constants and
//! envelope types, the `initialize`, `tools/list`, and `tools/call` payloads,
//! and the bridge from [`sandbox_core::SandboxError`] to [`JsonRpcError`].
//! The stdio transport and the request dispatcher arrive in Phase 1b.
```

- [x] **Step 6: Verify the workspace loads**

Run:

```bash
cargo metadata --no-deps --format-version 1 > /dev/null; echo "metadata exit: $?"
```

Expected: `metadata exit: 0` and no error output.

- [x] **Step 7: Commit**

Run:

```bash
git add Cargo.toml crates/sandbox-core/src/lib.rs crates/mcp-server/src/lib.rs
git commit -m "chore(workspace): load workspace without python-bindings member"
```

Expected: a new commit is created; `git status --short` shows no changes under `crates/` or in `Cargo.toml`.

---

### Task 2: Write the failing `sandbox-core` tests

**Files:**
- Modify (overwrite): `crates/sandbox-core/src/lib.rs`

- [x] **Step 1: Overwrite `crates/sandbox-core/src/lib.rs` with the test module only**

Content (crate docs + tests, no implementation):

```rust
//! Core execution engine for the MCP WebAssembly sandbox.
//!
//! Phase 1 defines [`SandboxError`], the domain error type shared by the
//! workspace. Engine configuration, the module cache, and Wasmtime execution
//! arrive in a later phase.

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
```

- [x] **Step 2: Run the tests and confirm the red signal**

Run:

```bash
cargo test -p sandbox-core
```

Expected: non-zero exit with

```
error[E0432]: unresolved import `super::SandboxError`
error: could not compile `sandbox-core` (lib test) due to 1 previous error
```

The first invocation compiles the `wasmtime` dependency tree and can take several minutes; that is expected. **Do not commit.** The tree does not compile yet.

---

### Task 3: Implement `SandboxError` and commit

**Files:**
- Modify (overwrite): `crates/sandbox-core/src/lib.rs`

- [x] **Step 1: Overwrite `crates/sandbox-core/src/lib.rs` with the implementation plus the unchanged tests**

Content:

```rust
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
```

- [x] **Step 2: Run the tests and confirm green**

Run:

```bash
cargo test -p sandbox-core
```

Expected:

```
running 5 tests
test tests::fuel_exhausted_display ... ok
test tests::io_error_conversion ... ok
test tests::memory_limit_display ... ok
test tests::payload_variant_display ... ok
test tests::variant_matching ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Test names are printed in completion order, so any order of the five `ok` lines is fine; the required line is `test result: ok. 5 passed; 0 failed`.

- [x] **Step 3: Commit**

Run:

```bash
git add crates/sandbox-core/src/lib.rs
git commit -m "feat(sandbox-core): add SandboxError domain type"
```

Expected: a new commit is created.

---

### Task 4: Write the failing `mcp-server` tests

**Files:**
- Modify (overwrite): `crates/mcp-server/src/lib.rs`

- [x] **Step 1: Overwrite `crates/mcp-server/src/lib.rs` with the test module only**

Content (crate docs + tests, no implementation):

```rust
//! Transport-agnostic JSON-RPC 2.0 envelope and MCP wire payloads.
//!
//! Phase 1 defines the message vocabulary only: the JSON-RPC constants and
//! envelope types, the `initialize`, `tools/list`, and `tools/call` payloads,
//! and the bridge from [`sandbox_core::SandboxError`] to [`JsonRpcError`].
//! The stdio transport and the request dispatcher arrive in Phase 1b.

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_core::SandboxError;
    use serde_json::json;

    #[test]
    fn constants() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(JSONRPC_VERSION, "2.0");
        assert_eq!(PARSE_ERROR, -32700);
        assert_eq!(INVALID_REQUEST, -32600);
        assert_eq!(METHOD_NOT_FOUND, -32601);
        assert_eq!(INVALID_PARAMS, -32602);
        assert_eq!(INTERNAL_ERROR, -32603);
        Ok(())
    }

    #[test]
    fn request_round_trip_number_id() -> Result<(), Box<dyn std::error::Error>> {
        let request = JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::Number(1),
            method: "tools/list".to_string(),
            params: None,
        };
        let value = serde_json::to_value(&request)?;
        assert_eq!(
            value,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})
        );
        let decoded: JsonRpcRequest = serde_json::from_value(value)?;
        assert_eq!(decoded.method, "tools/list");
        assert_eq!(decoded.id, JsonRpcId::Number(1));
        assert_eq!(decoded, request);
        Ok(())
    }

    #[test]
    fn request_round_trip_string_id() -> Result<(), Box<dyn std::error::Error>> {
        let request = JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::String("req-1".to_string()),
            method: "tools/call".to_string(),
            params: Some(json!({"name": "run_wasm", "arguments": {"module": "a.wasm"}})),
        };
        let value = serde_json::to_value(&request)?;
        assert_eq!(
            value,
            json!({
                "jsonrpc": "2.0",
                "id": "req-1",
                "method": "tools/call",
                "params": {"name": "run_wasm", "arguments": {"module": "a.wasm"}}
            })
        );
        let decoded: JsonRpcRequest = serde_json::from_value(value)?;
        assert_eq!(decoded.id, JsonRpcId::String("req-1".to_string()));
        assert_eq!(decoded, request);
        Ok(())
    }

    #[test]
    fn request_round_trip_null_id() -> Result<(), Box<dyn std::error::Error>> {
        let request = JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::Null,
            method: "ping".to_string(),
            params: None,
        };
        let value = serde_json::to_value(&request)?;
        assert_eq!(value, json!({"jsonrpc": "2.0", "id": null, "method": "ping"}));
        let decoded: JsonRpcRequest = serde_json::from_value(value)?;
        assert_eq!(decoded.id, JsonRpcId::Null);
        Ok(())
    }

    #[test]
    fn request_params_omitted_when_none() -> Result<(), Box<dyn std::error::Error>> {
        let request = JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::Number(1),
            method: "tools/list".to_string(),
            params: None,
        };
        let value = serde_json::to_value(&request)?;
        assert!(value.get("params").is_none());

        let decoded: JsonRpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": null
        }))?;
        assert_eq!(decoded.params, None);
        let value = serde_json::to_value(&decoded)?;
        assert!(value.get("params").is_none());
        Ok(())
    }

    #[test]
    fn notification_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let notification = JsonRpcNotification {
            jsonrpc: JSONRPC_VERSION.to_string(),
            method: "notifications/initialized".to_string(),
            params: None,
        };
        let value = serde_json::to_value(&notification)?;
        assert_eq!(
            value,
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        );
        assert!(value.get("id").is_none());
        let decoded: JsonRpcNotification = serde_json::from_value(value)?;
        assert_eq!(decoded.method, "notifications/initialized");
        assert_eq!(decoded, notification);

        let with_params = JsonRpcNotification {
            jsonrpc: JSONRPC_VERSION.to_string(),
            method: "notifications/progress".to_string(),
            params: Some(json!({"progress": 1})),
        };
        let value = serde_json::to_value(&with_params)?;
        assert_eq!(value["params"], json!({"progress": 1}));
        let decoded: JsonRpcNotification = serde_json::from_value(value)?;
        assert_eq!(decoded, with_params);
        Ok(())
    }

    #[test]
    fn response_success_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let response = JsonRpcResponse::Success {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::Number(1),
            result: json!({"tools": []}),
        };
        let value = serde_json::to_value(&response)?;
        assert_eq!(
            value,
            json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": []}})
        );
        let decoded: JsonRpcResponse = serde_json::from_value(value)?;
        assert_eq!(decoded, response);
        assert!(matches!(&decoded, JsonRpcResponse::Success { .. }));
        Ok(())
    }

    #[test]
    fn response_error_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let response = JsonRpcResponse::Error {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::Number(1),
            error: JsonRpcError {
                code: INTERNAL_ERROR,
                message: "boom".to_string(),
                data: None,
            },
        };
        let value = serde_json::to_value(&response)?;
        assert_eq!(
            value,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": {"code": -32603, "message": "boom"}
            })
        );
        let decoded: JsonRpcResponse = serde_json::from_value(value)?;
        assert_eq!(decoded, response);
        assert!(matches!(&decoded, JsonRpcResponse::Error { .. }));
        Ok(())
    }

    #[test]
    fn response_with_data() -> Result<(), Box<dyn std::error::Error>> {
        let response = JsonRpcResponse::Error {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::String("req-1".to_string()),
            error: JsonRpcError {
                code: INVALID_PARAMS,
                message: "bad params".to_string(),
                data: Some(json!({"detail": 1})),
            },
        };
        let value = serde_json::to_value(&response)?;
        assert_eq!(value["error"]["data"], json!({"detail": 1}));
        let decoded: JsonRpcResponse = serde_json::from_value(value)?;
        assert_eq!(decoded, response);
        Ok(())
    }

    #[test]
    fn fractional_id_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let result: Result<JsonRpcRequest, serde_json::Error> = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 1.5,
            "method": "x"
        }));
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn initialize_params_camel_case() -> Result<(), Box<dyn std::error::Error>> {
        let params = InitializeParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: json!({}),
            client_info: Implementation {
                name: "test-client".to_string(),
                version: "1.0.0".to_string(),
            },
        };
        let value = serde_json::to_value(&params)?;
        assert_eq!(
            value,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "1.0.0"}
            })
        );
        assert!(value.get("protocolVersion").is_some());
        assert!(value.get("clientInfo").is_some());
        assert!(value.get("protocol_version").is_none());
        assert!(value.get("client_info").is_none());
        let decoded: InitializeParams = serde_json::from_value(value)?;
        assert_eq!(decoded, params);
        Ok(())
    }

    #[test]
    fn initialize_result_camel_case() -> Result<(), Box<dyn std::error::Error>> {
        let result = InitializeResult {
            protocol_version: "2024-11-05".to_string(),
            capabilities: json!({}),
            server_info: Implementation {
                name: "mcp-wasm-sandbox".to_string(),
                version: "0.1.0".to_string(),
            },
        };
        let value = serde_json::to_value(&result)?;
        assert_eq!(
            value,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "mcp-wasm-sandbox", "version": "0.1.0"}
            })
        );
        assert!(value.get("serverInfo").is_some());
        assert!(value.get("server_info").is_none());
        let decoded: InitializeResult = serde_json::from_value(value)?;
        assert_eq!(decoded, result);
        Ok(())
    }

    #[test]
    fn tool_camel_case_and_optional_description() -> Result<(), Box<dyn std::error::Error>> {
        let tool = Tool {
            name: "run_wasm".to_string(),
            description: None,
            input_schema: json!({}),
        };
        let value = serde_json::to_value(&tool)?;
        assert_eq!(value, json!({"name": "run_wasm", "inputSchema": {}}));
        assert!(value.get("inputSchema").is_some());
        assert!(value.get("input_schema").is_none());
        assert!(value.get("description").is_none());
        let decoded: Tool = serde_json::from_value(value)?;
        assert_eq!(decoded, tool);

        let tool = Tool {
            name: "run_wasm".to_string(),
            description: Some("Runs wasm".to_string()),
            input_schema: json!({"type": "object"}),
        };
        let value = serde_json::to_value(&tool)?;
        assert_eq!(
            value,
            json!({
                "name": "run_wasm",
                "description": "Runs wasm",
                "inputSchema": {"type": "object"}
            })
        );
        let decoded: Tool = serde_json::from_value(value)?;
        assert_eq!(decoded, tool);
        Ok(())
    }

    #[test]
    fn tools_list_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let list = ToolsListResult {
            tools: vec![Tool {
                name: "run_wasm".to_string(),
                description: Some("Runs wasm".to_string()),
                input_schema: json!({"type": "object"}),
            }],
        };
        let value = serde_json::to_value(&list)?;
        assert_eq!(
            value,
            json!({
                "tools": [{
                    "name": "run_wasm",
                    "description": "Runs wasm",
                    "inputSchema": {"type": "object"}
                }]
            })
        );
        let decoded: ToolsListResult = serde_json::from_value(value)?;
        assert_eq!(decoded, list);
        Ok(())
    }

    #[test]
    fn tools_call_params_optional_arguments() -> Result<(), Box<dyn std::error::Error>> {
        let params = ToolsCallParams {
            name: "run_wasm".to_string(),
            arguments: None,
        };
        let value = serde_json::to_value(&params)?;
        assert_eq!(value, json!({"name": "run_wasm"}));
        assert!(value.get("arguments").is_none());
        let decoded: ToolsCallParams = serde_json::from_value(value)?;
        assert_eq!(decoded, params);

        let params = ToolsCallParams {
            name: "run_wasm".to_string(),
            arguments: Some(json!({"module": "a.wasm"})),
        };
        let value = serde_json::to_value(&params)?;
        assert_eq!(
            value,
            json!({"name": "run_wasm", "arguments": {"module": "a.wasm"}})
        );
        let decoded: ToolsCallParams = serde_json::from_value(value)?;
        assert_eq!(decoded, params);
        Ok(())
    }

    #[test]
    fn content_text_shape() -> Result<(), Box<dyn std::error::Error>> {
        let content = Content::Text {
            text: "42".to_string(),
        };
        let value = serde_json::to_value(&content)?;
        assert_eq!(value, json!({"type": "text", "text": "42"}));
        let decoded: Content = serde_json::from_value(value)?;
        assert_eq!(decoded, content);
        Ok(())
    }

    #[test]
    fn tools_call_result_camel_case() -> Result<(), Box<dyn std::error::Error>> {
        let result = ToolsCallResult {
            content: vec![Content::Text {
                text: "42".to_string(),
            }],
            is_error: Some(false),
        };
        let value = serde_json::to_value(&result)?;
        assert_eq!(
            value,
            json!({
                "content": [{"type": "text", "text": "42"}],
                "isError": false
            })
        );
        assert!(value.get("isError").is_some());
        assert!(value.get("is_error").is_none());
        let decoded: ToolsCallResult = serde_json::from_value(value)?;
        assert_eq!(decoded, result);

        let result = ToolsCallResult {
            content: vec![Content::Text {
                text: "boom".to_string(),
            }],
            is_error: Some(true),
        };
        let value = serde_json::to_value(&result)?;
        assert_eq!(value["isError"], json!(true));
        let decoded: ToolsCallResult = serde_json::from_value(value)?;
        assert_eq!(decoded, result);

        let result = ToolsCallResult {
            content: vec![Content::Text {
                text: "42".to_string(),
            }],
            is_error: None,
        };
        let value = serde_json::to_value(&result)?;
        assert!(value.get("isError").is_none());
        let decoded: ToolsCallResult = serde_json::from_value(value)?;
        assert_eq!(decoded, result);
        Ok(())
    }

    #[test]
    fn sandbox_error_maps_to_internal_error() -> Result<(), Box<dyn std::error::Error>> {
        let error = JsonRpcError::from(SandboxError::FuelExhausted);
        assert_eq!(error.code, INTERNAL_ERROR);
        assert!(error.data.is_none());
        assert_eq!(error.message, "wasm execution exhausted its fuel budget");
        Ok(())
    }

    #[test]
    fn sandbox_error_message_preserved() -> Result<(), Box<dyn std::error::Error>> {
        let error = JsonRpcError::from(SandboxError::Execution("trap".to_string()));
        assert_eq!(error.code, INTERNAL_ERROR);
        assert!(error.data.is_none());
        assert_eq!(error.message, "wasm execution failed: trap");
        Ok(())
    }
}
```

- [x] **Step 2: Run the tests and confirm the red signal**

Run:

```bash
cargo test -p mcp-server
```

Expected: non-zero exit. The first errors are unresolved items in the test module, for example:

```
error[E0425]: cannot find value `JSONRPC_VERSION` in this scope
error[E0422]: cannot find struct, variant or union type `JsonRpcRequest` in this scope
```

(`SandboxError` resolves because Task 3 already added it to `sandbox-core`.) **Do not commit.**

---

### Task 5: Implement the JSON-RPC constants and envelope (still red on MCP payloads)

**Files:**
- Modify: `crates/mcp-server/src/lib.rs` (insert between the crate docs and `#[cfg(test)] mod tests {`)

- [x] **Step 1: Insert the JSON-RPC block**

Replace exactly this anchor (the end of the crate docs plus the tests header):

```rust
//! The stdio transport and the request dispatcher arrive in Phase 1b.

#[cfg(test)]
mod tests {
```

with exactly:

```rust
//! The stdio transport and the request dispatcher arrive in Phase 1b.

use sandbox_core::SandboxError;
use serde::{Deserialize, Serialize};

/// JSON-RPC protocol version carried by every message.
pub const JSONRPC_VERSION: &str = "2.0";

/// Error code for a frame that is not valid JSON.
pub const PARSE_ERROR: i64 = -32700;

/// Error code for a JSON value that is not a valid request object.
pub const INVALID_REQUEST: i64 = -32600;

/// Error code for an unknown method.
pub const METHOD_NOT_FOUND: i64 = -32601;

/// Error code for params that do not match the method.
pub const INVALID_PARAMS: i64 = -32602;

/// Error code for an internal server fault.
pub const INTERNAL_ERROR: i64 = -32603;

/// Identifier of a JSON-RPC request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    /// Integer identifier.
    Number(i64),
    /// String identifier.
    String(String),
    /// Explicit `null` identifier.
    Null,
}

/// JSON-RPC 2.0 request: a call that expects a response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// Protocol version, normally [`JSONRPC_VERSION`].
    pub jsonrpc: String,
    /// Request identifier echoed in the response.
    pub id: JsonRpcId,
    /// Method name.
    pub method: String,
    /// Method parameters, omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 notification: a call that expects no response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    /// Protocol version, normally [`JSONRPC_VERSION`].
    pub jsonrpc: String,
    /// Method name.
    pub method: String,
    /// Method parameters, omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Error code.
    pub code: i64,
    /// Human-readable error message.
    pub message: String,
    /// Optional structured error data, omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 response: exactly one of a result or an error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcResponse {
    /// Successful response.
    Success {
        /// Protocol version, normally [`JSONRPC_VERSION`].
        jsonrpc: String,
        /// Identifier of the request being answered.
        id: JsonRpcId,
        /// Method result.
        result: serde_json::Value,
    },
    /// Failed response.
    Error {
        /// Protocol version, normally [`JSONRPC_VERSION`].
        jsonrpc: String,
        /// Identifier of the request being answered.
        id: JsonRpcId,
        /// Error object.
        error: JsonRpcError,
    },
}

#[cfg(test)]
mod tests {
```

- [x] **Step 2: Run the tests and confirm the remaining red signal**

Run:

```bash
cargo test -p mcp-server
```

Expected: non-zero exit. The envelope now compiles; the remaining errors are the missing MCP payload types and the missing `From` impl, for example:

```
error[E0422]: cannot find struct, variant or union type `InitializeParams` in this scope
error[E0422]: cannot find struct, variant or union type `Implementation` in this scope
```

A transient `warning: unused import: sandbox_core::SandboxError` is also expected at this stage because Task 6 adds the only use of that import; **do not remove the import.** **Do not commit.**

---

### Task 6: Implement the MCP payloads and `From<SandboxError> for JsonRpcError`, then commit

**Files:**
- Modify: `crates/mcp-server/src/lib.rs` (insert after the `JsonRpcResponse` enum and before `#[cfg(test)] mod tests {`)

- [x] **Step 1: Insert the MCP payloads and the error bridge**

Replace exactly this anchor (the end of the `JsonRpcResponse` enum plus the tests header):

```rust
    /// Failed response.
    Error {
        /// Protocol version, normally [`JSONRPC_VERSION`].
        jsonrpc: String,
        /// Identifier of the request being answered.
        id: JsonRpcId,
        /// Error object.
        error: JsonRpcError,
    },
}

#[cfg(test)]
mod tests {
```

with exactly:

```rust
    /// Failed response.
    Error {
        /// Protocol version, normally [`JSONRPC_VERSION`].
        jsonrpc: String,
        /// Identifier of the request being answered.
        id: JsonRpcId,
        /// Error object.
        error: JsonRpcError,
    },
}

/// A client or server implementation name and version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Implementation {
    /// Implementation name.
    pub name: String,
    /// Implementation version.
    pub version: String,
}

/// Params of the MCP `initialize` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    /// Protocol version requested by the client.
    pub protocol_version: String,
    /// Client capabilities, opaque in Phase 1.
    pub capabilities: serde_json::Value,
    /// Client name and version.
    pub client_info: Implementation,
}

/// Result of the MCP `initialize` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// Protocol version chosen by the server.
    pub protocol_version: String,
    /// Server capabilities, opaque in Phase 1.
    pub capabilities: serde_json::Value,
    /// Server name and version.
    pub server_info: Implementation,
}

/// Description of a tool exposed by the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    /// Tool name.
    pub name: String,
    /// Optional human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema of the tool arguments, opaque in Phase 1.
    pub input_schema: serde_json::Value,
}

/// Result of the MCP `tools/list` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsListResult {
    /// Available tools.
    pub tools: Vec<Tool>,
}

/// Params of the MCP `tools/call` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCallParams {
    /// Name of the tool to call.
    pub name: String,
    /// Tool arguments, opaque in Phase 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
}

/// A content block returned by a tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Content {
    /// Plain text content.
    Text {
        /// Text payload.
        text: String,
    },
}

/// Result of the MCP `tools/call` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCallResult {
    /// Content blocks produced by the tool.
    pub content: Vec<Content>,
    /// Whether the tool reported a failure, omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl From<SandboxError> for JsonRpcError {
    fn from(err: SandboxError) -> Self {
        Self {
            code: INTERNAL_ERROR,
            message: err.to_string(),
            data: None,
        }
    }
}

#[cfg(test)]
mod tests {
```

- [x] **Step 2: Run the tests and confirm green**

Run:

```bash
cargo test -p mcp-server
```

Expected:

```
running 19 tests
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

- [x] **Step 3: Commit**

Run:

```bash
git add crates/mcp-server/src/lib.rs
git commit -m "feat(mcp-server): add JSON-RPC 2.0 and MCP wire types"
```

Expected: a new commit is created.

---

### Task 7: Final verification gates

**Files:** none (verification only; commit only if a gate forces a code change)

- [x] **Step 1: Run the whole test suite**

Run:

```bash
cargo test --workspace
```

Expected: exit 0 and two passing unit-test binaries:

```
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

- [x] **Step 2: Run clippy with warnings denied**

Run:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: exit 0, ending with

```
Finished `dev` profile [unoptimized + debuginfo] target(s)
```

- [x] **Step 3: Check for forbidden panic calls**

Run:

```bash
rg -n "\.unwrap\(\)|\.expect\(" crates; echo "exit: $?"
```

Expected: no output before the exit line, and `exit: 1` (ripgrep's no-match status).

- [x] **Step 4: Check for `panic!`**

Run:

```bash
rg -n 'panic!' crates; echo "exit: $?"
```

Expected: no output before the exit line, and `exit: 1`.

- [x] **Step 5: Confirm the tree is clean**

Run:

```bash
git status --short
```

Expected: no entries under `crates/` and no entry for `Cargo.toml`. Pre-existing modifications to `.gitignore` and the untracked `docs/` directory are fine and must not be committed.

If a gate in Steps 1–4 fails, fix exactly the reported diagnostic (do not weaken or delete a test), then re-run Steps 1–4. If the fix changes a file under `crates/`, commit it with:

```bash
git add crates/sandbox-core/src/lib.rs crates/mcp-server/src/lib.rs
git commit -m "fix(wire-types): resolve verification gate finding"
```

- [x] **Step 6: Report completion**

Report the commands run and the observed results: `5 passed` (sandbox-core), `19 passed` (mcp-server), clippy exit 0, both `rg` checks with no matches. Also state that `crates/python-bindings` must be restored to `[workspace] members` in Phase 3.

---

## Out of scope (do not implement)

- `McpWireError`, JSON-RPC parsing, stdio transport, SSE/`axum` serving.
- Wasmtime engine, `Store`, `ResourceLimiter`, fuel metering, module cache.
- Dispatch, routing, version negotiation, `jsonrpc == "2.0"` enforcement, params validation.
- MCP `prompts`, `resources`, `sampling`, progress/cancellation payloads, `nextCursor`, `instructions`, `_meta`.
- Batch requests, borrowed/zero-copy wire types, authentication.
- New dependencies, new lints, `rust-toolchain.toml`, `#[non_exhaustive]`, `Eq`/`Default` derives, builders.
- Any edit to `crates/python-bindings/*`, `AGENTS.md`, `MEMORY.md`, or `.opencode/*`.
