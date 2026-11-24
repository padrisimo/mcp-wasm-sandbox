# Phase 1c: Minimal MCP Dispatcher, Stdio Binary, and Transport Hardening

**Date:** 2026-11-11
**Status:** Approved (design phase)
**Scope:** `crates/mcp-server`

## Goal

Add the smallest usable MCP server: a stateless `Dispatcher` implementing `RequestHandler` for `initialize`, `tools/list`, and `tools/call`, plus a `src/main.rs` binary that serves it over stdio, and close the two transport risks Phase 1b deferred — invalid UTF-8 ending the session and unbounded line length — by replacing `BufReader::lines()` with a private, bounded byte-frame reader that answers `PARSE_ERROR` / `INVALID_REQUEST` and keeps reading. No Wasmtime, no tools, no execution.

## Non-Goals

Out of scope; nothing below is stubbed or scaffolded:

- **Wasmtime and real tools.** No engine, `Store`, `ResourceLimiter`, fuel metering, module cache, tool registry, or `tools/call` execution. `tools/list` returns `[]`; `tools/call` always answers "unknown tool".
- **`spawn_blocking`.** No CPU-bound work exists yet, so nothing is offloaded. `RequestHandler` remains the seam.
- **SSE / `axum`.** Stdio only; `axum` stays declared but unused.
- **Concurrency, pipelining, timeouts, cancellation.** Handling stays sequential; EOF is the only shutdown signal.
- **`initialize` state.** No handshake tracking, no version negotiation, no capability enforcement; the client `protocolVersion` is echoed verbatim.
- **New MCP schema.** No `prompts`, `resources`, `notifications/*` payload types, pagination, or `instructions`. Notifications stay generic and are never routed to the handler.
- **Logging configuration.** The one new `tracing::warn!` has no subscriber and no `tracing-subscriber` wiring; output is invisible unless the host installs one.
- **Predictable error `data`.** No structured error payloads; `data` stays `None`.
- **Wire-type and public API changes.** `JsonRpcRequest`, `JsonRpcNotification`, `JsonRpcResponse`, `JsonRpcError`, constants, MCP payloads, `McpWireError`, and the `RequestHandler` / `serve` / `serve_stdio` signatures are unchanged.
- **New dependencies or manifest edits.** `src/main.rs` is auto-detected; both `Cargo.toml` files stay untouched.

## Context

`crates/mcp-server/src/` currently contains `lib.rs` (Phase 1 wire types, 19 tests), `error.rs` (`McpWireError`), and `transport.rs` (`RequestHandler`, `serve`, `serve_stdio`, 7 tests). The transport reads with `BufReader::lines()`, so Phase 1b Risk 1 (invalid UTF-8 surfaces as `io::Error(InvalidData)` and kills the session) and Risk 2 (unbounded line length) are both live. There is no dispatcher and no binary, so the crate cannot answer a single real MCP request yet.

Phase 1b fixed the boundary this slice inherits: a frame the transport cannot accept is answered (`PARSE_ERROR` / `INVALID_REQUEST`, `id: null`, `data: None`, fixed message text) and the session continues; only stream I/O and response encoding failures are `McpWireError`, which ends the session. Everything here fits that split.

`tokio` enables `full` (so `rt-multi-thread`, `macros`, `io-util`), `serde`, `serde_json`, `thiserror`, and `tracing` are already dependencies. `tracing` is declared but unused; this slice adds exactly one `warn!`. Rust 1.99.1 is installed (edition 2024 needs ≥1.85); a fresh shell may need `source "$HOME/.cargo/env"`. `.gitignore` still ignores `docs/superpowers/`, so this and both prior specs cannot be committed until that line is removed — an external blocker, not part of this task.

**Architecture alignment.** `architecture.md` §1 gives `mcp-server` the JSON-RPC parser, stdio transport, and future SSE; a dispatcher is the "parser" half. §2.1 defers CPU-bound work to `tokio::task::spawn_blocking`; with no Wasm there is nothing to offload, and the read loop never blocks, so the Tokio rule is satisfied by construction and the dispatcher stays a pure async match. §4's `McpWireError` is unchanged and still terminates the stream; method-level faults are `JsonRpcError` values on the wire, not Rust errors. §3.2's "controlled `McpError`" is still read as the §4 `McpWireError` (Phase 1b's documented assumption); this slice adds nothing that depends on the distinction.

## Proposed Architecture

### 1. Module layout

```
crates/mcp-server/src/
  lib.rs        # wire types + `mod dispatcher; mod error; mod transport;` + re-exports
  error.rs      # McpWireError (unchanged)
  transport.rs  # RequestHandler, serve, serve_stdio, bounded frame reader (hardened)
  dispatcher.rs # Dispatcher
  main.rs       # stdio binary
```

`dispatcher` joins `error` and `transport` as a private module; the public surface becomes `mcp_server::{Dispatcher, McpWireError, RequestHandler, serve, serve_stdio}`. Tests stay inline in each module.

### 2. `Dispatcher`

```rust
/// Routes MCP methods to their handlers. Stateless: no initialization tracking.
pub struct Dispatcher;

impl RequestHandler for Dispatcher {
    async fn handle(&mut self, request: JsonRpcRequest) -> JsonRpcResponse;
}
```

`handle` is one `match` on `request.method.as_str()` with three arms and a fallback; no route table, no builder, no state, no `Default`. A stateless unit struct means `serve_stdio(Dispatcher)` needs no construction ceremony and `tools/list` works before `initialize` (MCP clients that skip the handshake still get a valid answer).

| Method | Params | Response |
| --- | --- | --- |
| `initialize` | `InitializeParams` | `InitializeResult { protocol_version: <client value>, capabilities: {"tools": {}}, server_info: {name: env!("CARGO_PKG_NAME"), version: env!("CARGO_PKG_VERSION")} }` |
| `tools/list` | ignored | `ToolsListResult { tools: [] }` |
| `tools/call` | `ToolsCallParams` | `INVALID_PARAMS`, `"unknown tool: <name>"` — no tools are registered |
| anything else | ignored | `METHOD_NOT_FOUND`, `"method not found: <method>"` |

Param and error decisions:

- `initialize` with absent, `null`, or non-conforming params → `INVALID_PARAMS`, `"invalid initialize params"`. Deserialization is `serde_json::from_value::<InitializeParams>`, so a missing `protocolVersion`, `capabilities`, or `clientInfo` is rejected rather than defaulted.
- `protocolVersion` is echoed verbatim: no negotiation, no supported-version list, no rejection of unknown versions. The server does not yet have a protocol-sensitive surface to gate.
- `capabilities: {"tools": {}}` is the only advertised capability, matching the empty `tools/list`. `server_info` uses `env!` rather than a literal so a package rename propagates; both expand at compile time in this crate, so tests assert `"mcp-server"` and `"0.1.0"` and must be updated deliberately if the package is renamed.
- `tools/call` parses `ToolsCallParams` (so `arguments` is validated as JSON but otherwise ignored) and always fails with `"unknown tool: <name>"` — a *params* fault, not `METHOD_NOT_FOUND`, because the method exists. When Phase 2 registers tools, this arm gains a lookup; nothing else changes.
- Unknown methods carry the method name in the message so a client can tell which call failed without correlating ids. No `data`.
- Every response echoes `request.id` — success and error frames alike. This is the only id the dispatcher sees; the transport's own `id: null` frames never reach it.
- `jsonrpc` is always `JSONRPC_VERSION`; the incoming value is not validated, consistent with Phase 1 (`jsonrpc` is an unvalidated `String`).

Two private helpers keep the arms to a few lines each:

- `transport::error_response` is promoted to `pub(crate)` and takes the id: `fn error_response(id: JsonRpcId, code: i64, message: &str) -> JsonRpcResponse`. Transport's three call sites pass `JsonRpcId::Null`; the dispatcher passes `request.id`. One constructor, two users, no duplicated six-line literal.
- `fn success<T: Serialize>(id: JsonRpcId, value: &T) -> JsonRpcResponse` serializes with `serde_json::to_value` and falls back to `error_response(id, INTERNAL_ERROR, "failed to serialize result")` on `Err`. A `match`, never a panic. The branch is unreachable for `InitializeResult` / `ToolsListResult` (no non-string map keys, no non-finite floats), so it is defensive; see Risk 3.

`method`, `params`, and `id` are moved/borrowed per arm; no cloning of the request is needed beyond the id the response must own.

### 3. Binary

```rust
// crates/mcp-server/src/main.rs
#[tokio::main]
async fn main() -> Result<(), mcp_server::McpWireError> {
    mcp_server::serve_stdio(mcp_server::Dispatcher).await
}
```

Cargo auto-detects `src/main.rs` and names the binary after the package (`mcp-server`), so no `[[bin]]` section is added. `main` returning `Result<(), McpWireError>` prints the `Debug` form and exits non-zero on a stream failure; a clean EOF exits 0. The binary is a thin shell: no argument parsing, no flags, no `tracing-subscriber` init.

### 4. Bounded frame reader

`transport.rs` replaces `BufReader::lines()` with one private reader and one private outcome type:

```rust
/// One frame read from the stream.
enum Frame {
    /// No more input.
    Eof,
    /// A line with the trailing `\n` and one optional `\r` removed.
    Line(Vec<u8>),
    /// A line longer than [`MAX_LINE_BYTES`]; its tail was consumed and discarded.
    TooLarge,
}

/// Hard cap on a single request line, excluding the terminating `\n`.
const MAX_LINE_BYTES: usize = 1024 * 1024;

async fn read_frame<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Frame, McpWireError>;
```

Both are private; `MAX_LINE_BYTES` is not configurable (no env var, no argument, no const generic) until a real caller needs a different bound.

Algorithm, per frame:

1. `fill_buf().await?`; an empty chunk means EOF. If nothing was accumulated → `Frame::Eof`, otherwise finish the final unterminated line as `Frame::Line`.
2. Locate `\n` in the chunk. Found at index `at`: the line is `chunk[..at]`. Not found: the whole chunk belongs to the line.
3. If `accumulated + take > MAX_LINE_BYTES` → switch to discard mode: stop extending, `consume` through the newline (or the whole chunk when no newline), keep consuming and discarding until a chunk contains `\n` (consume through it) or EOF, then return `Frame::TooLarge`. Accumulation never exceeds `MAX_LINE_BYTES`, so a hostile stream cannot grow the buffer without bound.
4. Otherwise extend the accumulator, `consume` the taken bytes, and loop until the newline or EOF.
5. Strip exactly one trailing `\r` (CRLF clients) and return `Frame::Line`.

Two invariants the implementation must respect, because both are easy to get wrong:

- **Copy before `consume`.** `fill_buf` borrows the reader, so the chunk's bytes are appended to the accumulator inside an inner scope; `consume` is called after that scope ends. Holding `&[u8]` from `fill_buf` across `consume` does not compile.
- **The cap counts bytes between newlines, including any `\r`.** A line of exactly `MAX_LINE_BYTES` bytes is accepted; `MAX_LINE_BYTES + 1` is not. The final unterminated line is capped identically.

`Frame::TooLarge` is the only reason the reader skips input without answering on the spot; the loop always writes its error frame before reading on.

### 5. Hardened read loop

Per frame, in order:

1. **I/O error from the stream** → return `McpWireError::Io`. Session ends; this is now strictly a transport failure, since invalid UTF-8 no longer lands here.
2. **`Frame::TooLarge`** → write `INVALID_REQUEST` (`-32600`), `id: null`, `"request too large"`; continue.
3. **`Frame::Eof`** → `Ok(())`.
4. **Blank line** (every byte ASCII whitespace) → skip.
5. **`std::str::from_utf8` failure** → write `PARSE_ERROR` (`-32700`), `id: null`, `"invalid JSON"`; continue. Non-UTF-8 is treated as unparseable JSON rather than a stream fault, so one bad byte no longer closes the server.
6. **`serde_json::from_str::<Value>` failure** → `PARSE_ERROR`, `"invalid JSON"`; continue.
7. **Non-object value** (`42`, `[]`) → `INVALID_REQUEST`, `"invalid request"`; continue.
8. **`id` key present** → deserialize `JsonRpcRequest`; on success call `handler.handle(request).await` and write the response, on failure `INVALID_REQUEST`, `"invalid request"`; continue.
9. **`id` key absent** → deserialize `JsonRpcNotification`; on success do nothing (valid notifications stay silent and unanswered), on failure emit `tracing::warn!(%err, "dropping invalid notification")` and drop it.

Unchanged from Phase 1b: request-vs-notification is decided by `id` *presence* (`"id": null` is a request), error frames always carry `id: null` and `data: None`, responses are one `serde_json` line flushed after every write, and handling is sequential with no spawning. The new `warn!` is the only logging call in the crate and has no subscriber (Risk 5).

Byte-level blank detection replaces `line.trim().is_empty()`: a line of only spaces or tabs is still skipped, while a line of non-ASCII whitespace now reaches the JSON parser and becomes `PARSE_ERROR` instead of being silently skipped. Both behaviors are acceptable; the byte check avoids a UTF-8 precondition for a line that is not JSON anyway.

## Files To Change

**New:**

- `crates/mcp-server/src/dispatcher.rs` — `Dispatcher` plus dispatcher tests.
- `crates/mcp-server/src/main.rs` — `#[tokio::main]` binary over `serve_stdio`.

**Modified:**

- `crates/mcp-server/src/transport.rs` — add `Frame`, `MAX_LINE_BYTES`, `read_frame`; rewrite the `serve` loop over frames; promote `error_response` to `pub(crate)` with an `id` parameter; add the notification `warn!`; add transport tests 1–3.
- `crates/mcp-server/src/lib.rs` — `mod dispatcher;`, `pub use dispatcher::Dispatcher;`, and crate docs that currently say "request dispatch and MCP method routing arrive later".

**Unchanged:** both `Cargo.toml` files, workspace `members`, `error.rs`, `sandbox-core`, all wire types and their 19 tests, the existing 7 transport tests (they must keep passing as-is), `docs/architecture.md`, `AGENTS.md`.

## Testing Strategy

Tests come first: `dispatcher.rs` and the new `transport.rs` tests are written against unresolved items, `cargo test -p mcp-server` fails (red, local only), then the minimum implementation lands and everything goes green. Tests and implementation are committed together; a non-compiling tree is never committed.

Tests use `#[tokio::test]`, `&[u8]` input and `&mut Vec<u8>` output (both implement the tokio traits; no `duplex`), return `Result<(), Box<dyn std::error::Error>>`, and never use `unwrap`, `expect`, `expect_err`, or `panic!`. Responses are compared as `serde_json::Value`, never as strings; multi-response outputs are split on `\n` and checked line by line, including the trailing newline count. Dispatcher tests call `Dispatcher.handle(request).await` directly with a hand-built `JsonRpcRequest` and assert the serialized `Value`; the integration test goes through `serve`.

| # | Test | Asserts |
| --- | --- | --- |
| 1 | `invalid_utf8_gets_parse_error` | input `b"\xff\n"` + one valid request → 2 frames: first `PARSE_ERROR`, `id: null`, `"invalid JSON"`; second the handler's `Success`; handler called once (session survived) |
| 2 | `oversized_line_gets_invalid_request` | `vec![b'x'; MAX_LINE_BYTES + 1]` + `\n` + one valid request → 2 frames: first `INVALID_REQUEST`, `id: null`, `"request too large"`; second the `Success`; handler called once; no unbounded allocation (the reader caps at `MAX_LINE_BYTES`) |
| 3 | `invalid_notification_is_dropped` | `{"jsonrpc":"2.0"}` (no `method`, no `id`) → no output, handler not called, `Ok(())`; the `warn!` is not asserted (no subscriber in tests) |
| 4 | existing 7 transport tests | unchanged and still green (`lines()`-specific behavior is not asserted anywhere) |
| 5 | `initialize_returns_echoed_version_and_info` | id echoed; `protocolVersion` equals the client's `"2024-11-05"`; `capabilities.tools == {}`; `serverInfo.name == "mcp-server"`; `serverInfo.version == "0.1.0"`; `jsonrpc == "2.0"` |
| 6 | `initialize_invalid_params` | missing params and `params: {"protocolVersion":"x"}` → 2 responses, each `INVALID_PARAMS`, `"invalid initialize params"`, id echoed, no `data` |
| 7 | `tools_list_returns_empty_array` | `{"tools":[]}`; id echoed; `params` present-but-garbage is ignored |
| 8 | `tools_call_fails_for_unknown_tool` | `{"name":"run_wasm","arguments":{}}` → `INVALID_PARAMS`, `"unknown tool: run_wasm"`; `params: {}` → `INVALID_PARAMS`, `"invalid tools/call params"`; both echo the id |
| 9 | `unknown_method_gets_method_not_found` | `{"jsonrpc":"2.0","id":99,"method":"foo/bar"}` → `METHOD_NOT_FOUND`, `"method not found: foo/bar"`, id `99` |
| 10 | `dispatcher_over_serve_round_trip` | `serve` with `Dispatcher` and one `initialize` line → exactly one newline-terminated line whose `Value` matches `{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"mcp-server","version":"0.1.0"}}}` |

Test 2 allocates 1 MiB of input, which is cheap and deliberate: the cap path is only exercised by crossing it. Tests 6 and 8 fold two inputs into one case to keep the table at ten while covering both fixed messages in each method.

`serve_stdio` and `main` are not unit-tested: they wire stdin/stdout and contain no branching. The binary smoke test below is their coverage.

### Verification gates

```sh
source "$HOME/.cargo/env"
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
rg -n "\.unwrap\(\)|\.expect\(" crates
rg -n 'panic!' crates
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}' | cargo run -q -p mcp-server
```

The three cargo commands must exit 0 and both `rg` runs must produce no output (exit 1 is the expected no-match result). The smoke test must print exactly one JSON line with `"id":1` and `"protocolVersion":"2024-11-05"`. Because `printf` closes stdin, the smoke test cannot distinguish per-response flushing from EOF-only flushing; interactive flushing stays a review concern (Phase 1b Risk 5).

## Risks And Mitigations

1. **`fill_buf`/`consume` borrow conflict.** Holding the chunk across `consume` does not compile, and the natural fix (cloning every chunk) would defeat the cap. *Mitigation:* copy into the accumulator inside an inner scope and `consume` afterwards; test 2 crosses the cap and test 1 crosses the UTF-8 boundary, so both paths execute.
2. **1 MiB cap vs. real tool arguments.** `tools/call` will eventually carry a whole `.wasm` module as arguments; a base64 module over 1 MiB is rejected with `"request too large"`, which a future client could hit. *Mitigation:* the cap is generous for control frames and documented; Phase 2 should pass a module path/reference rather than inline bytes, or raise the cap deliberately. The reader caps memory regardless, so the failure mode is a clean error frame, not an OOM.
3. **Untested `INTERNAL_ERROR` branch.** `success`'s serialization failure is unreachable for the current result types, so no test covers it and coverage will show a dead arm. *Mitigation:* a `match` that returns an error frame, never a panic; kept so the zero-panic rule holds if a future result type becomes fallible. Removing it would trade 4 lines for a latent `unwrap`.
4. **Empty tool list looks broken.** A client sees `tools: []` and may report "server has no tools" before Phase 2. *Mitigation:* correct and honest for this phase; the binary exists to prove the pipe end to end, and Phase 2 registers the first tool.
5. **The new `warn!` is silent.** Without a subscriber, "dropping invalid notification" goes nowhere, so client bugs stay invisible. *Mitigation:* documented; subscriber setup is a separate slice. Tests assert behavior, not logs.
6. **Package rename breaks assertions.** Tests and the smoke test hard-code `"mcp-server"` / `"0.1.0"`, while the code uses `env!`. A rename turns them red. *Mitigation:* that is the intended signal — the alternative, asserting `env!` in tests, would pass even if the wrong string shipped.
7. **Sequential handling remains.** A slow handler blocks reads and all later requests. *Mitigation:* accepted since Phase 1b; with no Wasm and no I/O in the dispatcher, every handler returns immediately. Pipelining stays deferred to the execution slice.
8. **Blank-line semantics shift for non-ASCII whitespace.** A line of only U+00A0 now becomes `PARSE_ERROR` instead of being skipped. *Mitigation:* such a line is not valid JSON, so answering it is more correct than silence; test 4 (`"\n\n"`) still covers the real case.

## Decision Summary

- **Scope:** `Dispatcher`, `src/main.rs`, and the two Phase 1b transport risks; nothing else.
- **`Dispatcher`** is a stateless unit struct implementing `RequestHandler` with one `match`: `initialize`, `tools/list`, `tools/call`, fallback.
- **`initialize`** echoes the client `protocolVersion`, advertises `capabilities: {"tools": {}}`, and reports `env!("CARGO_PKG_NAME")` / `env!("CARGO_PKG_VERSION")`; invalid params → `INVALID_PARAMS`, `"invalid initialize params"`.
- **`tools/list`** ignores params and returns `{"tools":[]}`. **`tools/call`** parses `ToolsCallParams` and always answers `INVALID_PARAMS`, `"unknown tool: <name>"`; malformed params → `"invalid tools/call params"`. **Unknown method** → `METHOD_NOT_FOUND`, `"method not found: <method>"`.
- **Every response echoes the request id**, including errors; `jsonrpc` is always `"2.0"`; `data` is always `None`.
- **Result serialization failure** → `INTERNAL_ERROR`, `"failed to serialize result"` via a `match`, never a panic; unreachable for the current result types and deliberately kept.
- **One shared error constructor:** `transport::error_response(id, code, message)` becomes `pub(crate)`; transport passes `JsonRpcId::Null`, the dispatcher passes the request id.
- **Binary:** `src/main.rs`, `#[tokio::main]`, `serve_stdio(Dispatcher)`, `Result<(), McpWireError>`, no args, no subscriber, no manifest edit (auto-detected `mcp-server` binary).
- **Bounded reader replaces `lines()`:** private `Frame { Eof, Line, TooLarge }`, private `const MAX_LINE_BYTES: usize = 1024 * 1024`, built on `fill_buf`/`consume`, accumulating at most `MAX_LINE_BYTES` and discarding the tail of an oversize line through its newline.
- **Invalid UTF-8 becomes `PARSE_ERROR` `"invalid JSON"`** and the session continues; oversize becomes `INVALID_REQUEST` `"request too large"` and the session continues; only real stream I/O ends the session.
- **Invalid notifications are dropped with `tracing::warn!(%err, "dropping invalid notification")`**; valid notifications stay silent and unanswered.
- **Unchanged contracts:** `RequestHandler`, `serve`, `serve_stdio`, `McpWireError`, all wire types, `id`-presence discrimination, `id: null` transport error frames, blank-line skipping, per-response flush, sequential handling.
- **Tests:** 10 inline cases — 3 new transport, the existing 7 re-run, 5 dispatcher, 1 integration — over in-memory streams with `Value` comparisons and no `unwrap`/`expect`/`panic!`.
- **Gates:** `cargo test --workspace`, clippy with `-D warnings`, `cargo fmt --all -- --check`, clean `rg` for `unwrap`/`expect`, clean `rg` for `panic!`, plus the piped `initialize` smoke test, all after `source "$HOME/.cargo/env"`.
