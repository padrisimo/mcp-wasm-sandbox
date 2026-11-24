# Phase 1: Domain Errors and JSON-RPC 2.0 / MCP Wire Types

**Date:** 2026-11-11
**Status:** Approved (design phase)
**Scope:** `crates/sandbox-core`, `crates/mcp-server`

## Goal

Define the Phase 1 wire foundation: `SandboxError`, a `thiserror` domain error in `crates/sandbox-core/src/lib.rs`, and in `crates/mcp-server/src/lib.rs` the transport-agnostic JSON-RPC 2.0 envelope (`JsonRpcId`, `JsonRpcRequest`, `JsonRpcNotification`, `JsonRpcError`, `JsonRpcResponse`) plus typed MCP payloads for `initialize`, `tools/list`, and `tools/call`. Tests come first (red confirmed locally, then the minimum implementation), with no `unwrap`/`expect` and no new dependencies.

## Non-Goals

Out of scope; nothing below is stubbed or scaffolded:

- **Transport and `McpWireError`.** No stdio loop, SSE/`axum` server, or JSON-RPC parser; `McpWireError` (architecture §4) arrives with the parser in Phase 1b. `axum` and `tokio` stay declared but unused.
- **Wasmtime execution engine.** No engine configuration, `Store`, `ResourceLimiter`, fuel metering, module cache, or `spawn_blocking`. The error variants exist; only tests construct them.
- **Full MCP schema.** No `prompts`, `resources`, `sampling`, logging, progress, cancellation, `nextCursor`, `instructions`, `_meta`, capability sub-types, or typed notification payloads. The generic notification envelope may carry a `notifications/*` method; no payload type is defined.
- **JSON-RPC batch requests.**
- **Zero-copy / borrowed wire types.** No `Cow`, no `&str` fields.
- **Authentication and authorization.**
- **Request dispatch or validation.** No handler, router, version negotiation, `jsonrpc == "2.0"` enforcement, or `params` schema checks.
- **Python bindings.** `crates/python-bindings` stays empty.
- **Crate-wide lints and toolchain pinning.** No new `deny` attributes and no `rust-toolchain.toml`.

## Context

The workspace declares `sandbox-core`, `mcp-server`, and `python-bindings`; the two target crates contain only `Cargo.toml` with no `src/`, so both `lib.rs` files are new. `sandbox-core` declares `wasmtime`, `thiserror`, `tracing`, `tokio` and deliberately has **no `serde`/`serde_json`**. `mcp-server` already declares `sandbox-core`, `serde` (derive), `serde_json`, `thiserror`, `tokio`, tracing crates, and `axum`. No dependency changes are required.

**Architecture alignment.** `architecture.md` §1 assigns JSON-RPC parsing, stdio transport, and SSE/`axum` to `mcp-server`; §4 names two error enums, `SandboxError` and `McpWireError`. This spec is the wire-vocabulary slice: `JsonRpcError` here is the JSON-RPC error **object** that goes on the wire, while `McpWireError` is the future crate-level error for parse/serialization/transport failures. It is intentionally not defined in Phase 1; transport and the parser are Phase 1b. `MEMORY.md` lists that stdio work under Phase 1, which is why the split is stated here.

Project rules from `AGENTS.md` and `MEMORY.md`: zero panic strategy, `thiserror` domain errors, English documentation and messages, and no `.unwrap()`/`.expect()` anywhere (tests included).

**Workspace prerequisite (user-approved).** `crates/python-bindings` is a workspace member with only `pyproject.toml`, so cargo refuses to load the workspace (`failed to load manifest for workspace member`) and no gate can run. The user approved temporarily removing its line from `[workspace] members`; restore it in Phase 3. This is the only `Cargo.toml` edit in Phase 1.

**Environment.** Rust stable 1.99.1 is installed via rustup and `cargo`, `rustc`, and `cargo clippy` work. `edition = "2024"` requires ≥1.85, which 1.99.1 satisfies. A fresh shell may need `source "$HOME/.cargo/env"` (or `$HOME/.cargo/bin` on `PATH`).

## Proposed Architecture

### 1. One module per crate

`SandboxError` sits at the root of `sandbox-core`'s `lib.rs` (`sandbox_core::SandboxError`). All wire types sit at the root of `mcp-server`'s `lib.rs`; submodules are deferred until a second consumer exists. Both crate roots get `//!` docs and every `pub` item a `///` doc comment; `missing_docs` is not enabled.

### 2. `SandboxError`

```rust
/// Domain errors raised by the Wasm sandbox.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("wasm compilation failed: {0}")]
    Compile(String),
    #[error("wasm instantiation failed: {0}")]
    Instantiate(String),
    #[error("wasm execution failed: {0}")]
    Execution(String),
    #[error("wasm execution exhausted its fuel budget")]
    FuelExhausted,
    #[error("wasm memory limit exceeded: {limit_bytes} bytes")]
    MemoryLimit { limit_bytes: usize },
    #[error("invalid wasm module: {0}")]
    InvalidModule(String),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("unsupported operation: {0}")]
    Unsupported(String),
}
```

The variants give the guest anomalies in `architecture.md` §4 a home: `Execution` carries a `wasmtime::Trap` (divide-by-zero, unreachable, and the other traps), `FuelExhausted` is out-of-fuel, and `MemoryLimit` is an OOM denied by the `ResourceLimiter`. `Serialization(String)` exists because `sandbox-core` has no `serde_json`: there is no `From<serde_json::Error>`, callers pass `err.to_string()`. Only `Io` uses `#[from]`; `Debug` and `thiserror::Error` are the only derives because `std::io::Error` prevents `Clone`/`PartialEq`. No `#[non_exhaustive]` (unpublished crate).

Exact `Display` strings are contractual:

| Variant | `to_string()` |
| --- | --- |
| `Compile("bad")` | `wasm compilation failed: bad` |
| `Instantiate("bad")` | `wasm instantiation failed: bad` |
| `Execution("trap")` | `wasm execution failed: trap` |
| `FuelExhausted` | `wasm execution exhausted its fuel budget` |
| `MemoryLimit { limit_bytes: 4096 }` | `wasm memory limit exceeded: 4096 bytes` |
| `InvalidModule("bad")` | `invalid wasm module: bad` |
| `Io(err)` where `err` is `std::io::Error::other("boom")` | `i/o error: boom` |
| `Serialization("bad")` | `serialization error: bad` |
| `Unsupported("x")` | `unsupported operation: x` |

### 3. JSON-RPC 2.0 envelope

```rust
pub const JSONRPC_VERSION: &str = "2.0";

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(i64),
    String(String),
    Null,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcResponse {
    Success {
        jsonrpc: String,
        id: JsonRpcId,
        result: serde_json::Value,
    },
    Error {
        jsonrpc: String,
        id: JsonRpcId,
        error: JsonRpcError,
    },
}
```

- **`JsonRpcId`** is an enum so `true`, arrays, and objects cannot be ids. Variant order is `Number`, `String`, `Null`; numeric ids are `i64` only, so fractional or oversized numbers are rejected rather than coerced.
- **`id` and `method` are required** on `JsonRpcRequest`; `JsonRpcNotification` requires `method` and has no `id` field. The future dispatcher distinguishes them by the **presence or absence of `id`**, not by whether the value is `null`; `"id": null` is still a request. `method` is an unvalidated `String`.
- **`params` and `result` are `serde_json::Value`**; typed MCP payloads are converted by the future dispatcher. A generic `JsonRpcResponse<T>` buys nothing here.
- **`JsonRpcResponse` is an enum**, so `result` and `error` cannot coexist on output. `Success` is first so untagged deserialization prefers it when a malformed frame carries both.
- **`jsonrpc` is a plain `String`**, not a validated newtype: no handler exists to enforce `"2.0"` in Phase 1.
- **Optional fields are omitted when `None`**; explicit `null` deserializes to `None` and re-serializes as omitted.
- **No `rename_all`** on these types: the field names are already the wire names.

Wire examples the tests reproduce (compared as `serde_json::Value`):

```json
{"jsonrpc":"2.0","id":1,"method":"tools/list"}
{"jsonrpc":"2.0","id":"req-1","method":"tools/call","params":{"name":"run_wasm","arguments":{"module":"a.wasm"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}
{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"wasm execution failed: trap"}}
```

These are full JSON-RPC messages; the section 4 examples are payload-level JSON (the contents of `params` or `result`).

### 4. MCP payloads

All payload structs use `#[serde(rename_all = "camelCase")]`; optional fields are `Option<T>` with `skip_serializing_if = "Option::is_none"`.

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Implementation {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: serde_json::Value,
    pub client_info: Implementation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: serde_json::Value,
    pub server_info: Implementation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsListResult {
    pub tools: Vec<Tool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCallParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Content {
    Text { text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCallResult {
    pub content: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}
```

- **`capabilities`** is required but opaque (`Value`): Phase 1 does not negotiate capabilities.
- **`input_schema` and `arguments`** are opaque and unvalidated; the tool layer (Phase 2+) enforces object/schema shape. The camelCase key is the contract here.
- **`Content`** is a single-variant internally tagged enum (`{"type":"text","text":"42"}`); later variants are additive.
- **`is_error`** is `Option<bool>`; omitted when `None`, which the future dispatcher treats as `false`.
- **`protocol_version`** is a passthrough `String`; no negotiation or version constant.
- **`instructions`, `_meta`, `nextCursor`** are omitted entirely, per Non-Goals.

```json
{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test-client","version":"1.0.0"}}
{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"mcp-wasm-sandbox","version":"0.1.0"}}
{"tools":[{"name":"run_wasm","description":"Runs wasm","inputSchema":{"type":"object"}}]}
{"tools":[{"name":"run_wasm","inputSchema":{}}]}
{"name":"run_wasm","arguments":{"module":"a.wasm"}}
{"content":[{"type":"text","text":"42"}],"isError":false}
{"content":[{"type":"text","text":"boom"}],"isError":true}
```

### 5. Error bridging

```rust
impl From<SandboxError> for JsonRpcError {
    fn from(err: SandboxError) -> Self {
        Self {
            code: INTERNAL_ERROR,
            message: err.to_string(),
            data: None,
        }
    }
}
```

Every `SandboxError` maps to `INTERNAL_ERROR` (`-32603`), matching `architecture.md` §4, which allows `-32603` **or** custom runtime-bound codes; Phase 1 defines no custom codes, so all sandbox faults use `-32603`. The `Display` text is copied verbatim into `message`, and `data` stays `None` (no variant names or debug formatting leak to clients). The impl is orphan-rule legal because `JsonRpcError` is local; no reverse conversion exists.

### 6. Conventions

- Wire types derive `Debug, Clone, PartialEq, Serialize, Deserialize` (no `Eq`, `Default`, builders).
- `rename_all = "camelCase"` on payload structs; `untagged` on both enums; `tag = "type"` + `lowercase` on `Content`.
- No `deny_unknown_fields` anywhere: MCP carries `_meta` and future fields.
- Tests live in `#[cfg(test)] mod tests` at the bottom of each `lib.rs`.

## Files To Change

**New files:**

- `crates/sandbox-core/src/lib.rs` — crate docs, `SandboxError`, tests.
- `crates/mcp-server/src/lib.rs` — crate docs, JSON-RPC constants and types, MCP payloads, `From<SandboxError> for JsonRpcError`, tests.

**Modified:**

- `Cargo.toml` (workspace root) — remove `"crates/python-bindings"` from `members` (user-approved, restore in Phase 3). Only manifest edit.

**Unchanged:** both crate manifests (dependencies already declared), workspace dependency versions, `crates/python-bindings/*`, `AGENTS.md`, `MEMORY.md`, `.opencode/*`.

## Testing Strategy

Tests are written before the implementation. The first step creates each `lib.rs` locally (not committed) with crate docs and the test module only; `cargo test -p <crate>` fails on unresolved types — the expected red signal — and green means the same target compiles and passes.

Test functions return `Result<(), Box<dyn std::error::Error>>` and use `?`; assertions use `assert_eq!`, `assert!`, and `matches!`. No `unwrap`, `expect`, or `panic!`, so the grep gate stays clean.

**Comparisons are order-insensitive:** round-trips compare `serde_json::Value` against `serde_json::json!` literals, never raw strings. Raw-key checks (`value.get("protocolVersion")`, and absence of `protocol_version`) appear only in field-naming tests.

**Commit strategy:** a non-compiling tree is never committed. Red is local evidence; tests and minimum implementation land together per logical unit once green.

### `sandbox-core`

| # | Test | Asserts |
| --- | --- | --- |
| 1 | `fuel_exhausted_display` | `wasm execution exhausted its fuel budget` |
| 2 | `memory_limit_display` | `wasm memory limit exceeded: 4096 bytes` |
| 3 | `payload_variant_display` | `Compile`, `Instantiate`, `Execution`, `InvalidModule`, `Serialization`, `Unsupported` render the table strings |
| 4 | `io_error_conversion` | `SandboxError::from(std::io::Error::other("boom"))` is `Io` with `Display` = `i/o error: boom` |
| 5 | `variant_matching` | `matches!` on `FuelExhausted`, `MemoryLimit { limit_bytes: 4096 }`, and an `Io` value confirms public matchability |

### `mcp-server` — JSON-RPC

| # | Test | Asserts |
| --- | --- | --- |
| 1 | `constants` | `JSONRPC_VERSION == "2.0"`; codes `-32700`, `-32600`, `-32601`, `-32602`, `-32603` |
| 2 | `request_round_trip_number_id` | `Value` equals `json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})`; `method` and `id` round-trip |
| 3 | `request_round_trip_string_id` | `JsonRpcId::String("req-1")` survives |
| 4 | `request_round_trip_null_id` | `JsonRpcId::Null` serializes as `null` and deserializes back |
| 5 | `request_params_omitted_when_none` | no `params` key |
| 6 | `notification_round_trip` | with/without `params`; `method` round-trips; no `id` key |
| 7 | `response_success_round_trip` | `result` survives; deserializes as `Success` |
| 8 | `response_error_round_trip` | `error` survives; deserializes as `Error`; `data` omitted |
| 9 | `response_with_data` | `data: Some(json!({"detail": 1}))` round-trips |
| 10 | `fractional_id_rejected` | `{"jsonrpc":"2.0","id":1.5,"method":"x"}` fails to deserialize |

### `mcp-server` — MCP payloads

| # | Test | Asserts |
| --- | --- | --- |
| 11 | `initialize_params_camel_case` | raw keys `protocolVersion`, `clientInfo` present; snake_case absent; round-trips |
| 12 | `initialize_result_camel_case` | `protocolVersion`, `serverInfo`; round-trips |
| 13 | `tool_camel_case_and_optional_description` | `inputSchema` present; `description` omitted/present by `None`/`Some` |
| 14 | `tools_list_round_trip` | `Value` equals the documented JSON |
| 15 | `tools_call_params_optional_arguments` | `arguments` omitted when `None`, intact when `Some` |
| 16 | `content_text_shape` | `Value` equals `{"type":"text","text":"42"}`; round-trips |
| 17 | `tools_call_result_camel_case` | `isError` present, omitted when `None`, `false`/`true` round-trip |

### `mcp-server` — error bridging

| # | Test | Asserts |
| --- | --- | --- |
| 18 | `sandbox_error_maps_to_internal_error` | `FuelExhausted` → `code == INTERNAL_ERROR`, `data.is_none()`, message as specified |
| 19 | `sandbox_error_message_preserved` | `Execution("trap")` → `"wasm execution failed: trap"` |

### Verification gates

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
rg -n "\.unwrap\(\)|\.expect\(" crates
```

Both cargo commands must exit 0; `rg` must produce no output (exit 1 is the expected no-match result). The grep covers tests, so the test convention above applies there too, and doc comments must not quote the forbidden call syntax.

## Risks And Mitigations

1. **Untagged `JsonRpcId`.** Resolution relies on variant order; reordering or adding a variant silently reclassifies ids, and fractional/oversized numbers are rejected. *Mitigation:* fixed order, one test per id kind plus the rejection test, and the `i64`-only rule documented.
2. **MCP field-name drift.** A wrong `rename_all` breaks clients while struct-level tests still pass. *Mitigation:* camelCase on every payload struct and raw-key assertions for `protocolVersion`, `clientInfo`, `inputSchema`, `isError`.
3. **String error payloads lose source chains.** `Compile`/`Instantiate`/`Execution`/`Serialization`/`Unsupported` flatten causes; `Serialization` cannot expose line/column. *Mitigation:* `Io` keeps `#[from]` source semantics; the upgrade (typed `#[source]`, `wasmtime::Error`) is a Phase 2 decision that may add `serde_json` to `sandbox-core`.
4. **Untagged `JsonRpcResponse`.** A frame with both keys deserializes as `Success`. *Mitigation:* separate tests; the enum prevents emitting both; the dispatcher must not hand-build response JSON.
5. **Request/notification ambiguity on input.** Unknown fields are allowed, so a request also deserializes as a notification. *Mitigation:* the dispatcher decides by `id` presence (not null-ness); `deny_unknown_fields` is deliberately absent for `_meta`.
6. **Workspace cannot load.** The `python-bindings` member lacks a manifest, failing every cargo command. *Mitigation:* the user-approved one-line `members` removal; restore in Phase 3.
7. **Toolchain not visible in a fresh shell.** `cargo` not on `PATH` looks like "no toolchain" and invites skipped gates. *Mitigation:* `source "$HOME/.cargo/env"`, confirm `cargo --version` reports 1.99.1 (≥1.85 for edition 2024), and do not claim completion without the three gates.

## Decision Summary

- **Scope:** wire vocabulary only — `SandboxError`, JSON-RPC 2.0 envelope, MCP `initialize`/`tools/list`/`tools/call` payloads. Transport and `McpWireError` are Phase 1b.
- **`SandboxError`** derives `Debug` + `thiserror::Error` with nine variants; `Execution` covers `wasmtime::Trap`, `FuelExhausted` covers out-of-fuel, `MemoryLimit` covers OOM.
- **Exact English `Display` strings** per the table; `Io` is the only `#[from]`; `Serialization(String)` avoids adding `serde_json` to `sandbox-core`.
- **No `Clone`/`PartialEq`/`#[non_exhaustive]`** on `SandboxError`; no new dependencies.
- **Owned wire types only** (`String`, `serde_json::Value`); no `Cow` or generics.
- **Constants:** `JSONRPC_VERSION = "2.0"` and the five standard `i64` error codes.
- **`JsonRpcId`:** untagged `Number(i64) | String(String) | Null`, tried in that order; fractional/oversized numbers rejected.
- **Requests** require `id` and `method`; **notifications** require `method`, have no `id`; dispatcher uses `id` presence, not null-ness. Unknown fields allowed.
- **`JsonRpcResponse`** is untagged with `Success` before `Error`, so `result`/`error` cannot coexist on output.
- **`JsonRpcError`** is `{ code, message, data? }`; optional fields are omitted when `None`, and **`jsonrpc`** is an unvalidated `String`.
- **MCP payloads** use `rename_all = "camelCase"`; `capabilities`, `input_schema`, `arguments` are opaque `Value`s; `Content` has only `Text`; `is_error` is `Option<bool>`.
- **`From<SandboxError> for JsonRpcError`** always uses `INTERNAL_ERROR` (`-32603`) and copies the `Display` text into `message`; `data` is `None`.
- **TDD:** tests first, red local only, tests + minimum implementation committed together once green; `Value` comparisons, not strings.
- **One manifest change:** user-approved temporary removal of `crates/python-bindings` from `members`.
- **Gates:** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, clean `rg` for `unwrap`/`expect`, on Rust 1.99.1 (≥1.85).
