# Phase 1c Implementation Plan

**Spec:** `docs/superpowers/specs/2026-11-12-phase-1c-dispatcher-transport-hardening-design.md`
**Rules:** TDD (tests first, red locally, tests + implementation committed together), no `unwrap`/`expect`/`panic!`, English docs, no new dependencies. Run cargo commands with `source "$HOME/.cargo/env" &&`.

## Task 1: Transport hardening

- [x] **Step 1: Add the failing tests to `crates/mcp-server/src/transport.rs`**
  Append to the existing `#[cfg(test)] mod tests`:
  - `invalid_utf8_gets_parse_error`: input `b"\xff\n"` → one frame, `error.code == PARSE_ERROR`, `error.message == "invalid JSON"`, `id == null`, handler not called.
  - `oversized_line_gets_invalid_request`: input = `MAX_LINE_BYTES + 1` bytes of `a` + `\n` + a valid `ping` request line → two frames; first `error.code == INVALID_REQUEST`, `error.message == "request too large"`, `id == null`; second is the handler response; handler called once.
  - `invalid_notification_is_dropped`: input `{"jsonrpc":"2.0","method":42}` → no output, handler not called.
- [x] **Step 2: Run the red check**
  `cargo test -p mcp-server` → exit 101; failures in the three new tests (or unresolved `MAX_LINE_BYTES`).
- [x] **Step 3: Implement the bounded reader**
  In `crates/mcp-server/src/transport.rs`:
  - Add `const MAX_LINE_BYTES: usize = 1024 * 1024;`.
  - Replace `BufReader::lines()` with a private `enum IncomingLine { Eof, Line(Vec<u8>), TooLong }` and `async fn read_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<IncomingLine, std::io::Error>` built on `fill_buf`/`consume`; discard the rest of an oversized line without growing the buffer past the cap.
  - In `serve`: `TooLong` → `INVALID_REQUEST` `"request too large"`; invalid UTF-8 → `PARSE_ERROR` `"invalid JSON"`; invalid notification → `tracing::warn!(%err, "dropping invalid notification")` and drop.
  - Switch the import from `AsyncBufReadExt` to `AsyncBufRead`.
- [x] **Step 4: Run the green check**
  `cargo test -p mcp-server` → 29 passed (26 + 3 new); `cargo clippy --workspace --all-targets -- -D warnings` exit 0; `cargo fmt --all -- --check` exit 0.
- [x] **Step 5: Commit**
  `git add crates/mcp-server/src/transport.rs` → `feat(mcp-server): harden stdio transport against invalid utf-8 and oversized lines`.

## Task 2: Dispatcher and stdio binary

- [x] **Step 1: Create `crates/mcp-server/src/dispatcher.rs` with tests only**
  Tests: `initialize` success (id echoed, `protocolVersion` echoed, `capabilities == {"tools": {}}`, `serverInfo.name == "mcp-server"`, `serverInfo.version == "0.1.0"`), `initialize` invalid params → `INVALID_PARAMS` `"invalid initialize params"`, `tools/list` → `{"tools": []}`, `tools/call` unknown tool → `INVALID_PARAMS` `"unknown tool: run_wasm"`, unknown method → `METHOD_NOT_FOUND` `"method not found: nope"`. Helper `assert_error(response, id, code, message) -> Result<(), Box<dyn std::error::Error>>` avoids `panic!`. Add `mod dispatcher;` to `lib.rs`.
- [x] **Step 2: Run the red check**
  `cargo test -p mcp-server` → exit 101; `Dispatcher` unresolved.
- [x] **Step 3: Implement the dispatcher**
  `pub struct Dispatcher;` with `impl RequestHandler for Dispatcher` routing `initialize`, `tools/list`, `tools/call`, and `METHOD_NOT_FOUND`; private `success(id, result)` and `error(id, code, message)` helpers; `INTERNAL_ERROR` `"failed to serialize result"` when a result fails to serialize.
- [x] **Step 4: Run the green check**
  `cargo test -p mcp-server` → 34 passed; clippy and fmt clean.
- [x] **Step 5: Add the binary and the integration test**
  - `crates/mcp-server/src/main.rs` with `#[tokio::main] async fn main() -> Result<(), McpWireError>` calling `serve_stdio(Dispatcher)`.
  - `lib.rs`: `pub use dispatcher::Dispatcher;` and updated crate docs.
  - Integration test in `transport.rs` tests: `serve` with `Dispatcher` answers an `initialize` request with one response line.
- [x] **Step 6: Run the green check and the binary smoke test**
  `cargo test -p mcp-server` → 35 passed; `printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}' | cargo run -q -p mcp-server` → one line containing `"id":1` and `"protocolVersion":"2024-11-05"`.
- [x] **Step 7: Commit**
  `git add crates/mcp-server/src/dispatcher.rs crates/mcp-server/src/main.rs crates/mcp-server/src/lib.rs crates/mcp-server/src/transport.rs` → `feat(mcp-server): add MCP dispatcher and stdio binary`.

## Task 3: Final gates

- [x] **Step 1:** `cargo test --workspace` → 40 passed (5 sandbox-core + 35 mcp-server).
- [x] **Step 2:** `cargo clippy --workspace --all-targets -- -D warnings` → exit 0.
- [x] **Step 3:** `cargo fmt --all -- --check` → exit 0.
- [x] **Step 4:** `rg -n "\.unwrap\(\)|\.expect\(" crates` → no output; `rg -n 'panic!' crates` → no output.
- [x] **Step 5:** `git status --short` shows only `?? Cargo.lock`; `docs/`, `.gitignore` never staged.
- [x] **Step 6:** Push `main` to `origin`.
