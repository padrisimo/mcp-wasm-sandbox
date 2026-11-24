# Phase 1b: `McpWireError` and Async Stdio Transport

**Date:** 2026-11-11
**Status:** Approved (design phase)
**Scope:** `crates/mcp-server`

## Goal

Add the crate-level `McpWireError` (the parse/serialization/transport enum named in `architecture.md` §4) and a newline-delimited JSON-RPC 2.0 stdio transport in `crates/mcp-server`, without touching the existing wire types. `serve` reads one JSON message per line, routes requests through a caller-supplied `RequestHandler`, answers with one JSON line per request, and maps malformed input to JSON-RPC error frames. The request dispatcher, MCP method routing, and Wasm execution remain out of scope.

## Non-Goals

- **Dispatcher and routing.** No `main.rs`, no binary target, no `initialize`/`tools/*` handling, no `spawn_blocking`, no Wasmtime. `McpWireError` and the transport land now; routing is a later slice.
- **SSE / `axum`.** Stdio only; `axum` stays declared but unused.
- **Transport features beyond NDJSON.** No batch requests, timeouts, cancellation, graceful-shutdown signals (EOF is shutdown), TLS, sockets, or reconnect logic.
- **Logging.** No `tracing` calls; invalid notifications are dropped silently.
- **Wire-type changes.** `JsonRpcRequest`, `JsonRpcNotification`, `JsonRpcResponse`, `JsonRpcError`, constants, and MCP payloads stay as they are; only `lib.rs` module wiring changes.
- **Handler adapters.** No blanket `RequestHandler` impl for closures, no boxed/dyn handler, no `Send` bounds (nothing is spawned).
- **New dependencies or manifest edits.**

## Context

`crates/mcp-server/src/lib.rs` currently holds the Phase 1 wire vocabulary and 19 passing tests. Its crate doc still says the stdio transport "arrives in Phase 1b" — that sentence must be corrected as part of this slice. The crate has no other modules and no binary target. Workspace `tokio` enables `full`, so `rt`, `macros`, and `io-util` are available; `serde_json` and `thiserror` are already dependencies. No manifest changes are needed. Rust 1.99.1 is installed (edition 2024 needs ≥1.85); a fresh shell may require `source "$HOME/.cargo/env"`.

**Architecture alignment.** `architecture.md` §1 assigns stdio transport to `mcp-server`, and §4 names `McpWireError` for host-level parsing, serialization, and I/O failures. JSON-RPC, however, requires malformed frames to be *answered* rather than returned, so frame-level parse failures become `PARSE_ERROR` responses while `McpWireError` carries stream I/O and response-encoding failures. §2.1 defers CPU-bound work to `tokio::task::spawn_blocking`; this slice has no Wasm work, so the async read loop never blocks, and `RequestHandler` is the seam where a future dispatcher can offload. Phase 1's `JsonRpcError` remains the wire object; `McpWireError` is the Rust-level crate error that terminates the transport. Join semantics: a malformed frame is a wire-object response (session continues), while a serialization or I/O failure is `McpWireError` (session ends).

§3.2 of the architecture mentions a "controlled `McpError`"; this spec treats it as the same crate-level error named `McpWireError` in §4. If it was meant as a distinct type, the architecture doc needs a correction.

## Proposed Architecture

### 1. Module layout

```
crates/mcp-server/src/
  lib.rs        # existing wire types + `mod error; mod transport;` + re-exports
  error.rs      # McpWireError
  transport.rs  # RequestHandler, serve, serve_stdio
```

`error` and `transport` stay private modules; the public surface is `mcp_server::{McpWireError, RequestHandler, serve, serve_stdio}`. Tests stay inline in each module.

### 2. `McpWireError`

```rust
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
```

Only two variants: every failure the transport can raise is either JSON serialization or stream I/O, and `#[from]` lets both surface with `?`. Frame parse failures never become `Serde`: they are answered with `PARSE_ERROR`, so `Serde` is raised only when encoding a response (`to_vec`) fails. No `Clone`/`PartialEq` (`std::io::Error`), no `#[non_exhaustive]` (unpublished crate), no `Protocol` variant (malformed frames are answered, not returned). `Display` strings are contractual: `Io` is asserted exactly with `std::io::Error::other("boom")` → `transport i/o error: boom`; `Serde` is asserted by prefix because `serde_json`'s detail text is version-dependent (test 1 builds an instance from a `from_str` failure to exercise `Display`).

### 3. Request handling contract

```rust
/// Handles one JSON-RPC request and returns the response to write.
pub trait RequestHandler {
    /// Handles a request. The returned response must echo the request id.
    fn handle(&mut self, request: JsonRpcRequest) -> impl Future<Output = JsonRpcResponse>;
}

/// Serves JSON-RPC over `reader`/`writer` until EOF.
pub async fn serve<R, W, H>(reader: R, writer: W, handler: H) -> Result<(), McpWireError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    H: RequestHandler;

/// Serves JSON-RPC over process stdin/stdout.
pub async fn serve_stdio<H: RequestHandler>(handler: H) -> Result<(), McpWireError>;
```

- `-> impl Future` rather than `async fn` in the trait: a public `async fn` trait method triggers the warn-by-default `async_fn_in_trait` lint, which `-D warnings` turns into a failure. No `Send` bound is needed because the future is awaited inline, never spawned.
- `serve` takes the handler by value and the streams by value; `serve_stdio` is a three-line wrapper over `tokio::io::stdin()`/`stdout()`.
- The handler owns id echoing: the transport writes exactly the returned response and never rewrites ids or results.

### 4. Read loop

`BufReader::new(reader).lines()` yields one `String` per `\n`, strips a trailing `\r`, returns the final unterminated line at EOF, and keeps the stream usable after a decode error (this spec still ends the session in that case; see Risk 1). For each line, in order:

1. **Read error** (including invalid UTF-8): return `McpWireError::Io`. The session ends; a broken byte stream is not answered per line.
2. **Blank line** (`line.trim().is_empty()`): skip, no output.
3. **`serde_json::from_str::<Value>`**: on failure, answer `PARSE_ERROR` (`-32700`) with `id: null` and message `invalid JSON`, then continue.
4. **Non-object value** (e.g. `42`, `[]`): answer `INVALID_REQUEST` (`-32600`) with `id: null` and message `invalid request`, then continue.
5. **`id` key present** (`value.get("id")` returns `Some`, including `null`): deserialize `JsonRpcRequest`. On success, call `handler.handle(request).await` and write the response. On failure (missing `method`, fractional id, wrong types), answer `INVALID_REQUEST` with `id: null` and message `invalid request`, then continue.
6. **`id` key absent**: deserialize `JsonRpcNotification`. On success, do nothing (no response, handler not called); on failure, drop it silently.
7. **EOF**: return `Ok(())`.

Decision points:

- **`id` presence, not null-ness**, selects request vs notification, matching Phase 1: `{"jsonrpc": "2.0", "id": null, "method": "x"}` is a request and gets a response echoing `null`.
- **Error frames are `JsonRpcResponse::Error` with `jsonrpc: "2.0"`, `id: null`, `data: None`**, even when the incoming frame had a parseable id, because the frame was not accepted as a request and no id is trusted.
- **Message text is fixed** (`invalid JSON`, `invalid request`) so clients and tests are stable; serde details are not leaked. `data` stays `None`.
- **`method` and `params` are not inspected**; unknown methods are the dispatcher's concern, not the transport's.
- **A line with trailing content** (two JSON values) fails `from_str` and becomes `PARSE_ERROR`.

Response writing uses one private helper: `serde_json::to_vec(&response)?`, append `b'\n'`, `write_all`, then `flush`, **after every response**. Flushing only at EOF would deadlock an interactive client that waits for a response before sending its next line; per-response flushes make an EOF flush unnecessary. Serialization and I/O failures here propagate as `McpWireError`, ending the stream.

Handling is sequential: the next line is read only after the handler returns and the response is flushed. That is enough for Phase 1b and gives natural backpressure; pipelining is a later concern.

## Files To Change

**New:**

- `crates/mcp-server/src/error.rs` — `McpWireError` plus its Display tests.
- `crates/mcp-server/src/transport.rs` — `RequestHandler`, `serve`, `serve_stdio`, transport tests.

**Modified:**

- `crates/mcp-server/src/lib.rs` — add `mod error; mod transport;`, re-export `McpWireError`, `RequestHandler`, `serve`, `serve_stdio`, and fix the crate docs sentence that says the transport arrives in Phase 1b. No wire-type or test changes.

**Unchanged:** both `Cargo.toml` files, workspace `members`, `sandbox-core`, existing wire types and their 19 tests, `docs/architecture.md`, `AGENTS.md`, `python-bindings`.

## Testing Strategy

Tests come first: `error.rs` and `transport.rs` are created with tests only, `cargo test -p mcp-server` fails on unresolved items (red), then the minimum implementation lands and the same tests go green. Tests and implementation are committed together; a non-compiling tree is never committed.

Tests use `#[tokio::test]` (available via the workspace `full` tokio). Input is `&[u8]` and output is `&mut Vec<u8>`; both implement `AsyncRead`/`AsyncWrite` in tokio 1.54.1, so `tokio::io::duplex` is unnecessary. A test handler is a struct holding `Arc<AtomicUsize>`, incrementing on each call and returning `Success` with `result: {"ok": true}` and the request's id; the test keeps an `Arc` clone to assert the call count. Output is decoded as UTF-8, split on `\n`, and each line compared as `serde_json::Value`, never as a string. Tests return `Result<(), Box<dyn std::error::Error>>`, use `?`, and never use `unwrap`, `expect`, or `panic!`; constructing a `serde_json` error for test 1 uses a `match`/`let-else` that returns `Err`, not `expect_err` (which would trip the grep gate).

| # | Test | Asserts |
| --- | --- | --- |
| 1 | `wire_error_display` (`error.rs`) | `Io` exact: `transport i/o error: boom`; `Serde` starts with `json-rpc serialization error:` |
| 2 | `valid_request_writes_one_response_line` | handler called once; output is exactly one `\n`-terminated line; `Value` equals `{"jsonrpc":"2.0","id":7,"result":{"ok":true}}` |
| 3 | `notification_writes_nothing` | handler not called; output empty; `Ok(())` |
| 4 | `invalid_json_gets_parse_error` | input `{oops}` + newline → one frame with `code == PARSE_ERROR`, `id == null`, `message == "invalid JSON"`; handler not called |
| 5 | `invalid_request_gets_invalid_request` | input `42` and `{"jsonrpc":"2.0","id":1}` (missing `method`) → two frames, each `code == INVALID_REQUEST`, `id == null`, `message == "invalid request"`; handler not called |
| 6 | `blank_lines_ignored` | input `"\n\n"` → no output; `Ok(())` |
| 7 | `clean_eof_is_ok` | empty input → no output; `Ok(())` |

`serve_stdio` is not directly tested: it only wires stdin/stdout and contains no branching. Its behavior is covered through `serve`.

### Verification gates

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
rg -n "\.unwrap\(\)|\.expect\(" crates
```

Both cargo commands must exit 0; `rg` must produce no output (exit 1 is the expected no-match result). The grep covers tests.

## Risks And Mitigations

1. **Invalid UTF-8 ends the session.** Tokio reports a decode failure as `io::Error(InvalidData)`; `serve` propagates it, so one bad byte closes the stream instead of earning `PARSE_ERROR`. *Mitigation:* documented behavior; if untrusted clients appear, special-case `InvalidData` to answer `PARSE_ERROR` and continue. No test in this slice.
2. **Unbounded line length.** `lines()` allocates the whole line, so a huge frame can pressure memory. *Mitigation:* documented limit; add a byte cap before exposing the transport beyond local stdio.
3. **Sequential handling.** A slow handler blocks reads and all later requests. *Mitigation:* accepted for Phase 1b (natural backpressure); the dispatcher can pipeline or `spawn_blocking` later behind the same trait.
4. **Silent invalid notifications.** A malformed notification is dropped with no diagnostics, hiding client bugs. *Mitigation:* documented; add `tracing` when the dispatcher lands.
5. **Per-response flush is untestable with a `Vec` writer.** A missing flush would only appear against a real pipe, where an interactive client would hang. *Mitigation:* explicit flush in the single write helper; covered by review, not by the table above.
6. **Handler id contract is unenforced.** A handler that ignores the request id produces a mismatched response. *Mitigation:* documented on the trait; the future dispatcher is the only implementor and will echo.

## Decision Summary

- **New modules only:** `error.rs` (`McpWireError`) and `transport.rs` (`RequestHandler`, `serve`, `serve_stdio`); `lib.rs` gains `mod`/`pub use` and a corrected doc sentence.
- **`McpWireError` has exactly two variants**, `Serde(#[from] serde_json::Error)` and `Io(#[from] std::io::Error)`, with fixed Display text (exact for `Io`, prefix for `Serde`); no `Clone`, `PartialEq`, `#[non_exhaustive]`, or `Protocol` variant.
- **Malformed frames are answered; stream failures end the session.** `PARSE_ERROR`/`INVALID_REQUEST` responses use `id: null`, `data: None`, and fixed messages `invalid JSON` / `invalid request`.
- **Request vs notification is decided by `id` presence**, including `"id": null` as a request.
- **Non-object JSON is `INVALID_REQUEST`**, not a notification.
- **Blank lines are skipped; the final unterminated line is processed; clean EOF returns `Ok(())`.**
- **Read errors, including invalid UTF-8, propagate as `McpWireError::Io`.**
- **Every response is one `serde_json` line flushed immediately**; serialization and write failures propagate.
- **`RequestHandler::handle` returns `impl Future`**, avoiding the `async_fn_in_trait` lint and requiring no `Send`; handlers echo the request id.
- **Handling is sequential**; no spawning, no concurrency, no timeouts.
- **No dispatcher, no binary, no `spawn_blocking`, no Wasm, no `tracing`, no new dependencies.**
- **Tests:** 7 inline cases over in-memory `&[u8]`/`&mut Vec<u8>`, `#[tokio::test]`, `Arc<AtomicUsize>` call counting, `Value` comparisons; `serve_stdio` untested by design.
- **Gates:** `cargo test --workspace`, clippy with `-D warnings`, clean `rg` for `unwrap`/`expect`, on Rust 1.99.1.
