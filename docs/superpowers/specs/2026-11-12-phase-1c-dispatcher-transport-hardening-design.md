# Phase 1c: Minimal Dispatcher, Stdio Binary, and Transport Hardening

**Date:** 2026-11-12
**Status:** Approved (design phase)
**Scope:** `crates/mcp-server`

## Goal

Finish Phase 1 with the two pieces still missing before the Wasm runtime: a minimal MCP dispatcher with a stdio binary, and the transport hardening that closes the Phase 1b risks (invalid UTF-8 ended the session, line length was unbounded, invalid notifications were dropped silently). No Wasm execution, no new dependencies.

## Non-Goals

- **Wasm execution.** No Wasmtime engine, no tool registry, no `tools/call` execution, no `spawn_blocking`. Phase 2.
- **Transport features.** No SSE/`axum`, no concurrency or pipelining, no timeouts, no auth.
- **Protocol depth.** No capability negotiation, no `initialize` version negotiation, no MCP notifications handling, no `_meta`.
- **Logging setup.** `tracing` events are emitted but no subscriber is configured in the binary.

## Context

`crates/mcp-server` currently holds the wire vocabulary (`lib.rs`), `McpWireError` (`error.rs`), and the NDJSON stdio transport (`transport.rs`), with 26 passing tests. `architecture.md` §1 assigns JSON-RPC parsing, stdio transport, and routing to `mcp-server`; §2.1 requires CPU-bound work to leave the async thread pool; §4 names `SandboxError` and `McpWireError` as the domain errors.

Phase 1b left three open risks: invalid UTF-8 terminated the session instead of being answered, `lines()` had no length bound, and malformed notifications were dropped without diagnostics. It also left the crate without a runnable server: the transport had no dispatcher and no binary.

## Proposed Architecture

### 1. `Dispatcher` (`src/dispatcher.rs`)

`pub struct Dispatcher;` implements `RequestHandler`. Routing is a `match` on `request.method`:

| Method | Behavior |
| --- | --- |
| `initialize` | Deserialize `InitializeParams`; return `InitializeResult` with `protocol_version` echoed from the client, `capabilities: {"tools": {}}`, `server_info: Implementation { name: env!("CARGO_PKG_NAME"), version: env!("CARGO_PKG_VERSION") }`. Invalid or missing params → `INVALID_PARAMS`, `"invalid initialize params"`. |
| `tools/list` | Return `ToolsListResult { tools: Vec::new() }`. Params ignored. |
| `tools/call` | Deserialize `ToolsCallParams`; no tool exists yet → `INVALID_PARAMS`, `"unknown tool: <name>"`. Invalid or missing params → `INVALID_PARAMS`, `"invalid tools/call params"`. |
| other | `METHOD_NOT_FOUND`, `"method not found: <method>"`. |

Every response echoes the request `id`. A result that fails to serialize becomes `INTERNAL_ERROR`, `"failed to serialize result"` — never a panic.

### 2. Binary (`src/main.rs`)

`#[tokio::main] async fn main() -> Result<(), McpWireError>` calls `serve_stdio(Dispatcher)`. No `Cargo.toml` change: `src/main.rs` is auto-detected.

### 3. Transport hardening (`src/transport.rs`)

`BufReader::lines()` is replaced by a private bounded byte-line reader built on `AsyncBufRead::fill_buf`/`consume`:

- `MAX_LINE_BYTES = 1 MiB` (private const). A longer line is discarded up to the next newline and answered with `INVALID_REQUEST`, `"request too large"`; the reader never allocates more than the cap.
- Invalid UTF-8 is answered with `PARSE_ERROR`, `"invalid JSON"`, and the session continues.
- Malformed notifications are dropped with `tracing::warn!`; valid notifications stay unanswered and silent.
- Blank-line skipping, EOF → `Ok(())`, per-response flush, `id`-presence request/notification discrimination, and the existing fixed error frames are unchanged. `RequestHandler`, `serve`, and `serve_stdio` keep their signatures.

## Files To Change

**New:** `crates/mcp-server/src/dispatcher.rs`, `crates/mcp-server/src/main.rs`.

**Modified:** `crates/mcp-server/src/lib.rs` (`mod dispatcher;`, `pub use dispatcher::Dispatcher;`, crate docs), `crates/mcp-server/src/transport.rs` (bounded reader, UTF-8 recovery, warning on invalid notifications, new tests).

**Unchanged:** both manifests, `sandbox-core`, the wire types and their 19 tests, `docs/architecture.md`, `AGENTS.md`.

## Testing Strategy

TDD, tests before implementation, tests and implementation committed together per unit. No `unwrap`, `expect`, `panic!`, or `expect_err` anywhere; `#[tokio::test]`; in-memory `&[u8]`/`&mut Vec<u8>`; `serde_json::Value` comparisons.

Transport: invalid UTF-8 → `PARSE_ERROR`; oversized line → `INVALID_REQUEST` with the following valid request still answered; invalid notification → no output. The 7 existing transport tests stay green.

Dispatcher: `initialize` success (id echoed, `protocolVersion` echoed, `capabilities.tools == {}`, `serverInfo.name == "mcp-server"`, `serverInfo.version == "0.1.0"`); `initialize` invalid params; `tools/list` empty; `tools/call` unknown tool; unknown method. Integration: `serve` with `Dispatcher` answers an `initialize` request with one response line.

Gates: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `rg -n "\.unwrap\(\)|\.expect\(" crates`, `rg -n 'panic!' crates`. Binary smoke test: pipe one `initialize` request into `cargo run -q -p mcp-server` and expect one response line echoing `"id":1`.

## Risks And Mitigations

1. **`tools/list` advertises no tools and `tools/call` always fails.** A client sees an empty tool set. *Mitigation:* honest placeholder; Phase 2 registers the real tool. The capability object still declares `tools`.
2. **The 1 MiB cap rejects legitimate large frames.** *Mitigation:* documented; raise the constant if a real client needs more.
3. **Handling is sequential.** A slow handler delays later requests. *Mitigation:* unchanged from Phase 1b; stdio clients are request/response, and Phase 2 offloads Wasm work with `spawn_blocking` behind the same trait.
4. **`serve_stdio` remains untested.** It only wires stdin/stdout. *Mitigation:* binary smoke test in the gates.
5. **No `initialize` version negotiation.** The client version is echoed. *Mitigation:* documented; negotiation belongs with the tool layer.

## Decision Summary

- **New:** `dispatcher.rs` (`Dispatcher`) and `main.rs`; `lib.rs` gains `mod dispatcher;` and the re-export.
- **Dispatcher routes** `initialize`, `tools/list`, `tools/call`, and answers everything else with `METHOD_NOT_FOUND`; ids are always echoed.
- **Transport** uses a private 1 MiB bounded byte-line reader; invalid UTF-8 → `PARSE_ERROR`; oversized lines → `INVALID_REQUEST`; invalid notifications are warned about and dropped.
- **Public API unchanged:** `RequestHandler`, `serve`, `serve_stdio`, `McpWireError`, and every wire type keep their signatures and behavior.
- **No new dependencies**, no Wasm, no SSE, no concurrency.
- **Tests:** 3 new transport cases, 5 dispatcher cases, 1 integration case, plus the existing 31.
