# Phase 1b: `McpWireError` and Async Stdio Transport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers-subagent-driven-development (recommended) or superpowers-executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `McpWireError` and a newline-delimited JSON-RPC 2.0 stdio transport (`RequestHandler`, `serve`, `serve_stdio`) to `mcp-server`, leaving the Phase 1 wire types and their 19 tests untouched.

**Architecture:** Two new private modules: `error.rs` holds the two-variant crate-level error, `transport.rs` holds the handler trait and the async read/write loop. `lib.rs` gains `mod error; mod transport;` plus public re-exports and a corrected crate doc. The transport answers malformed frames with JSON-RPC error responses (`PARSE_ERROR` / `INVALID_REQUEST`, `id: null`) and reserves `McpWireError` for stream I/O and response-encoding failures.

**Tech Stack:** Rust 1.99.1 (edition 2024), `tokio` 1.54.1 with workspace feature `full` (`AsyncBufReadExt::lines`, `#[tokio::test]`), `serde_json` 1.0, `thiserror` 1.0. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-11-phase-1b-mcp-wire-error-stdio-transport-design.md` (approved).

---

## Context and contract

Current state: `crates/mcp-server/src/lib.rs` contains the Phase 1 wire vocabulary and 19 passing tests. It is the only file in `src/`. `crates/mcp-server/Cargo.toml` already declares `tokio` (workspace feature `full`), `serde`, `serde_json`, and `thiserror`; no manifest change is required. `Cargo.lock` is untracked in this repository; it must not be committed.

Run every command from the repository root (the directory containing the workspace `Cargo.toml`). A fresh shell may need `source "$HOME/.cargo/env"`.

The transport contract, fixed by the spec:

- **Read loop:** one JSON value per line via `BufReader::lines()`. A read error (including invalid UTF-8) returns `McpWireError::Io` and ends the session. Blank/whitespace-only lines are skipped.
- **Frame handling:** invalid JSON → `PARSE_ERROR` (`-32700`) with message `invalid JSON`; valid JSON that is not an object → `INVALID_REQUEST` (`-32600`) with message `invalid request`; an object with an `id` key (including `"id": null`) is a request, deserialized and handed to the handler; an object without `id` is a notification and produces no output.
- **Error frames:** `jsonrpc: "2.0"`, `id: JsonRpcId::Null`, `data: None`. The incoming id is never trusted or echoed. Fixed messages are contractual.
- **Response writing:** `serde_json::to_vec`, append `b'\n'`, `write_all`, then `flush` after **every** response (also error frames). Serialization and write failures propagate as `McpWireError`.
- **Handler:** owns id echoing; the transport writes exactly the returned `JsonRpcResponse`.
- **`serve_stdio`** is a three-line wrapper over `serve` (process stdin/stdout); it is not directly tested because it contains no branching.
- **`RequestHandler::handle` returns `impl Future`**, not `async fn`, so the warn-by-default `async_fn_in_trait` lint does not fire under `-D warnings`.

Constraints for every task:

- No `unwrap`, `expect`, `expect_err`, or `panic!` anywhere, tests included. Use `?`, `match`, `let-else`, and `assert!`.
- Tests compare `serde_json::Value` values, never raw response strings.
- Tests first: create the two modules with tests only, confirm the red signal locally, then implement. A red tree is never committed; tests and implementation land in one commit.
- Do not change any existing wire type, constant, `From` impl, or the 19 existing tests in `lib.rs`.

---

## File structure

| File | Action | Responsibility |
| --- | --- | --- |
| `crates/mcp-server/src/error.rs` | Create (Task 1 tests, Task 2 implementation) | `McpWireError` |
| `crates/mcp-server/src/transport.rs` | Create (Task 1 tests, Task 2 implementation) | `RequestHandler`, `serve`, `serve_stdio`, transport tests |
| `crates/mcp-server/src/lib.rs` | Modify twice | Task 1: `mod error; mod transport;`. Task 2: crate doc + re-exports. No other changes. |

---

### Task 1: Create the failing module tests and wire the modules

**Files:**
- Create: `crates/mcp-server/src/error.rs`
- Create: `crates/mcp-server/src/transport.rs`
- Modify: `crates/mcp-server/src/lib.rs`

- [x] **Step 1: Create `crates/mcp-server/src/error.rs` with the test only**

Content:

```rust
//! Crate-level errors for the MCP wire layer.

#[cfg(test)]
mod tests {
    use super::McpWireError;

    #[test]
    fn wire_error_display() -> Result<(), Box<dyn std::error::Error>> {
        let io = McpWireError::from(std::io::Error::other("boom"));
        assert_eq!(io.to_string(), "transport i/o error: boom");

        let Err(error) = serde_json::from_str::<serde_json::Value>("{oops}") else {
            return Err("invalid JSON unexpectedly parsed".into());
        };
        let serde_error = McpWireError::from(error);
        assert!(
            serde_error
                .to_string()
                .starts_with("json-rpc serialization error:")
        );

        Ok(())
    }
}
```

- [x] **Step 2: Create `crates/mcp-server/src/transport.rs` with the tests only**

Content:

```rust
//! Newline-delimited JSON-RPC 2.0 over async byte streams.

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;

    struct CountingHandler {
        calls: Arc<AtomicUsize>,
    }

    impl RequestHandler for CountingHandler {
        async fn handle(&mut self, request: JsonRpcRequest) -> JsonRpcResponse {
            self.calls.fetch_add(1, Ordering::SeqCst);
            JsonRpcResponse::Success {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id: request.id,
                result: json!({"ok": true}),
            }
        }
    }

    fn counting_handler() -> (CountingHandler, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            CountingHandler {
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }

    fn response_values(
        output: &[u8],
    ) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
        let text = std::str::from_utf8(output)?;
        let mut values = Vec::new();
        for line in text.lines() {
            values.push(serde_json::from_str(line)?);
        }
        Ok(values)
    }

    #[tokio::test]
    async fn valid_request_writes_one_response_line() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let input = concat!(r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#, "\n").as_bytes();
        let mut output = Vec::new();

        serve(input, &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(output.iter().filter(|byte| **byte == b'\n').count(), 1);
        assert_eq!(output.last(), Some(&b'\n'));

        let values = response_values(&output)?;
        assert_eq!(
            values,
            vec![json!({"jsonrpc": "2.0", "id": 7, "result": {"ok": true}})]
        );
        Ok(())
    }

    #[tokio::test]
    async fn notification_writes_nothing() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let input = concat!(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n"
        )
        .as_bytes();
        let mut output = Vec::new();

        serve(input, &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(output.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn invalid_json_gets_parse_error() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let mut output = Vec::new();

        serve("{oops}\n".as_bytes(), &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let values = response_values(&output)?;
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["jsonrpc"], json!("2.0"));
        assert_eq!(values[0]["id"], json!(null));
        assert_eq!(values[0]["error"]["code"], json!(PARSE_ERROR));
        assert_eq!(values[0]["error"]["message"], json!("invalid JSON"));
        assert!(values[0]["error"].get("data").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn invalid_request_gets_invalid_request() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let input = concat!("42\n", r#"{"jsonrpc":"2.0","id":1}"#, "\n").as_bytes();
        let mut output = Vec::new();

        serve(input, &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let values = response_values(&output)?;
        assert_eq!(values.len(), 2);
        for value in &values {
            assert_eq!(value["jsonrpc"], json!("2.0"));
            assert_eq!(value["id"], json!(null));
            assert_eq!(value["error"]["code"], json!(INVALID_REQUEST));
            assert_eq!(value["error"]["message"], json!("invalid request"));
            assert!(value["error"].get("data").is_none());
        }
        Ok(())
    }

    #[tokio::test]
    async fn blank_lines_ignored() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let mut output = Vec::new();

        serve("\n\n".as_bytes(), &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(output.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn clean_eof_is_ok() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let input: &[u8] = b"";
        let mut output = Vec::new();

        serve(input, &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(output.is_empty());
        Ok(())
    }
}
```

- [x] **Step 3: Wire both modules into `crates/mcp-server/src/lib.rs`**

Replace exactly:

```rust
use sandbox_core::SandboxError;
use serde::{Deserialize, Serialize};
```

with exactly:

```rust
mod error;
mod transport;

use sandbox_core::SandboxError;
use serde::{Deserialize, Serialize};
```

Change nothing else in `lib.rs` in this task (no re-exports, no doc edit yet).

- [x] **Step 4: Confirm the red signal**

Run:

```bash
cargo test -p mcp-server
```

Expected: non-zero exit. The library compiles but the test binary does not; the first errors are:

```
error[E0432]: unresolved import `super::McpWireError`
error[E0405]: cannot find trait `RequestHandler` in this scope
error[E0425]: cannot find function `serve` in this scope
```

More `cannot find type/value` errors for `JsonRpcRequest`, `JsonRpcResponse`, `JSONRPC_VERSION`, `PARSE_ERROR`, and `INVALID_REQUEST` follow. **Do not commit.** The tree does not compile under `cfg(test)`.

---

### Task 2: Implement both modules, update `lib.rs`, and commit

**Files:**
- Modify (overwrite): `crates/mcp-server/src/error.rs`
- Modify (overwrite): `crates/mcp-server/src/transport.rs`
- Modify: `crates/mcp-server/src/lib.rs`

- [x] **Step 1: Overwrite `crates/mcp-server/src/error.rs` with `McpWireError` plus the test**

Content:

```rust
//! Crate-level errors for the MCP wire layer.

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

#[cfg(test)]
mod tests {
    use super::McpWireError;

    #[test]
    fn wire_error_display() -> Result<(), Box<dyn std::error::Error>> {
        let io = McpWireError::from(std::io::Error::other("boom"));
        assert_eq!(io.to_string(), "transport i/o error: boom");

        let Err(error) = serde_json::from_str::<serde_json::Value>("{oops}") else {
            return Err("invalid JSON unexpectedly parsed".into());
        };
        let serde_error = McpWireError::from(error);
        assert!(
            serde_error
                .to_string()
                .starts_with("json-rpc serialization error:")
        );

        Ok(())
    }
}
```

- [x] **Step 2: Overwrite `crates/mcp-server/src/transport.rs` with the implementation plus the unchanged tests**

Content:

```rust
//! Newline-delimited JSON-RPC 2.0 over async byte streams.

use std::future::Future;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::error::McpWireError;
use crate::{
    INVALID_REQUEST, JSONRPC_VERSION, JsonRpcError, JsonRpcId, JsonRpcNotification, JsonRpcRequest,
    JsonRpcResponse, PARSE_ERROR,
};

/// Handles one JSON-RPC request and returns the response to write.
pub trait RequestHandler {
    /// Handles a request. The returned response must echo the request id.
    fn handle(&mut self, request: JsonRpcRequest) -> impl Future<Output = JsonRpcResponse>;
}

/// Serves JSON-RPC over `reader` and `writer` until EOF.
pub async fn serve<R, W, H>(reader: R, mut writer: W, mut handler: H) -> Result<(), McpWireError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    H: RequestHandler,
{
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                write_response(&mut writer, error_response(PARSE_ERROR, "invalid JSON")).await?;
                continue;
            }
        };

        if !value.is_object() {
            write_response(
                &mut writer,
                error_response(INVALID_REQUEST, "invalid request"),
            )
            .await?;
            continue;
        }

        if value.get("id").is_some() {
            let request: JsonRpcRequest = match serde_json::from_value(value) {
                Ok(request) => request,
                Err(_) => {
                    write_response(
                        &mut writer,
                        error_response(INVALID_REQUEST, "invalid request"),
                    )
                    .await?;
                    continue;
                }
            };
            let response = handler.handle(request).await;
            write_response(&mut writer, response).await?;
        } else {
            // Notifications are never answered; invalid ones are dropped silently.
            let _ = serde_json::from_value::<JsonRpcNotification>(value);
        }
    }

    Ok(())
}

/// Serves JSON-RPC over process stdin and stdout until EOF.
pub async fn serve_stdio<H: RequestHandler>(handler: H) -> Result<(), McpWireError> {
    serve(tokio::io::stdin(), tokio::io::stdout(), handler).await
}

fn error_response(code: i64, message: &str) -> JsonRpcResponse {
    JsonRpcResponse::Error {
        jsonrpc: JSONRPC_VERSION.to_string(),
        id: JsonRpcId::Null,
        error: JsonRpcError {
            code,
            message: message.to_string(),
            data: None,
        },
    }
}

async fn write_response<W: AsyncWrite + Unpin>(
    writer: &mut W,
    response: JsonRpcResponse,
) -> Result<(), McpWireError> {
    let mut line = serde_json::to_vec(&response)?;
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;

    struct CountingHandler {
        calls: Arc<AtomicUsize>,
    }

    impl RequestHandler for CountingHandler {
        async fn handle(&mut self, request: JsonRpcRequest) -> JsonRpcResponse {
            self.calls.fetch_add(1, Ordering::SeqCst);
            JsonRpcResponse::Success {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id: request.id,
                result: json!({"ok": true}),
            }
        }
    }

    fn counting_handler() -> (CountingHandler, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            CountingHandler {
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }

    fn response_values(
        output: &[u8],
    ) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
        let text = std::str::from_utf8(output)?;
        let mut values = Vec::new();
        for line in text.lines() {
            values.push(serde_json::from_str(line)?);
        }
        Ok(values)
    }

    #[tokio::test]
    async fn valid_request_writes_one_response_line() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let input = concat!(r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#, "\n").as_bytes();
        let mut output = Vec::new();

        serve(input, &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(output.iter().filter(|byte| **byte == b'\n').count(), 1);
        assert_eq!(output.last(), Some(&b'\n'));

        let values = response_values(&output)?;
        assert_eq!(
            values,
            vec![json!({"jsonrpc": "2.0", "id": 7, "result": {"ok": true}})]
        );
        Ok(())
    }

    #[tokio::test]
    async fn notification_writes_nothing() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let input = concat!(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n"
        )
        .as_bytes();
        let mut output = Vec::new();

        serve(input, &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(output.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn invalid_json_gets_parse_error() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let mut output = Vec::new();

        serve("{oops}\n".as_bytes(), &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let values = response_values(&output)?;
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["jsonrpc"], json!("2.0"));
        assert_eq!(values[0]["id"], json!(null));
        assert_eq!(values[0]["error"]["code"], json!(PARSE_ERROR));
        assert_eq!(values[0]["error"]["message"], json!("invalid JSON"));
        assert!(values[0]["error"].get("data").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn invalid_request_gets_invalid_request() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let input = concat!("42\n", r#"{"jsonrpc":"2.0","id":1}"#, "\n").as_bytes();
        let mut output = Vec::new();

        serve(input, &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let values = response_values(&output)?;
        assert_eq!(values.len(), 2);
        for value in &values {
            assert_eq!(value["jsonrpc"], json!("2.0"));
            assert_eq!(value["id"], json!(null));
            assert_eq!(value["error"]["code"], json!(INVALID_REQUEST));
            assert_eq!(value["error"]["message"], json!("invalid request"));
            assert!(value["error"].get("data").is_none());
        }
        Ok(())
    }

    #[tokio::test]
    async fn blank_lines_ignored() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let mut output = Vec::new();

        serve("\n\n".as_bytes(), &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(output.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn clean_eof_is_ok() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let input: &[u8] = b"";
        let mut output = Vec::new();

        serve(input, &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(output.is_empty());
        Ok(())
    }
}
```

- [x] **Step 3: Update the crate docs and add re-exports in `crates/mcp-server/src/lib.rs`**

Replace exactly:

```rust
//! Transport-agnostic JSON-RPC 2.0 envelope and MCP wire payloads.
//!
//! Phase 1 defines the message vocabulary only: the JSON-RPC constants and
//! envelope types, the `initialize`, `tools/list`, and `tools/call` payloads,
//! and the bridge from [`sandbox_core::SandboxError`] to [`JsonRpcError`].
//! The stdio transport and the request dispatcher arrive in Phase 1b.

mod error;
mod transport;
```

with exactly:

```rust
//! JSON-RPC 2.0 envelope, MCP wire payloads, and newline-delimited stdio transport.
//!
//! The JSON-RPC constants and envelope types, the `initialize`, `tools/list`,
//! and `tools/call` payloads, and the bridge from [`sandbox_core::SandboxError`]
//! to [`JsonRpcError`] come from Phase 1. Phase 1b adds [`McpWireError`] and
//! the newline-delimited transport ([`serve`] and [`serve_stdio`]); request
//! dispatch and MCP method routing arrive later.

mod error;
mod transport;

pub use error::McpWireError;
pub use transport::{RequestHandler, serve, serve_stdio};
```

Change nothing else in `lib.rs`: the constants, wire types, `From<SandboxError>` impl, and `#[cfg(test)] mod tests` stay byte-identical.

- [x] **Step 4: Run the tests and confirm green**

Run:

```bash
cargo test -p mcp-server
```

Expected: exit 0 and

```
running 26 tests
test error::tests::wire_error_display ... ok
test transport::tests::blank_lines_ignored ... ok
test transport::tests::clean_eof_is_ok ... ok
test transport::tests::invalid_json_gets_parse_error ... ok
test transport::tests::invalid_request_gets_invalid_request ... ok
test transport::tests::notification_writes_nothing ... ok
test transport::tests::valid_request_writes_one_response_line ... ok
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

That is the 19 Phase 1 tests plus 7 new ones. Test order and the position of `test result` are not fixed; the required outcome is 26 total tests and `test result: ok. 26 passed; 0 failed`.

- [x] **Step 5: Commit**

Run:

```bash
git add crates/mcp-server/src/lib.rs crates/mcp-server/src/error.rs crates/mcp-server/src/transport.rs
git commit -m "feat(mcp-server): add McpWireError and stdio transport"
```

Expected: a new commit is created. `git status --short` afterwards shows only `?? Cargo.lock` (untracked, never committed).

---

### Task 3: Final verification gates

**Files:** none (verification only; a formatting-only fix may touch the three source files)

- [x] **Step 1: Run the whole test suite**

Run:

```bash
cargo test --workspace
```

Expected: exit 0 and two passing unit-test binaries:

```
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
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

This is the gate that would fail if `handle` were declared `async fn` in the trait (`async_fn_in_trait`); the `impl Future` signature above avoids it.

- [x] **Step 3: Check formatting**

Run:

```bash
cargo fmt --all -- --check
```

Expected: exit 0 and no output. If it prints a diff, apply the formatting and commit it:

```bash
cargo fmt --all
git add crates/mcp-server/src/lib.rs crates/mcp-server/src/error.rs crates/mcp-server/src/transport.rs
git commit -m "style(mcp-server): format phase 1b sources"
```

- [x] **Step 4: Check for forbidden panic calls**

Run:

```bash
rg -n "\.unwrap\(\)|\.expect\(" crates; echo "exit: $?"
```

Expected: no output before the exit line, and `exit: 1` (ripgrep's no-match status). This pattern catches `unwrap` and `expect` calls; it does not match `expect_err`, which the tests avoid by using `let-else` (see `wire_error_display`).

- [x] **Step 5: Check for `panic!`**

Run:

```bash
rg -n 'panic!' crates; echo "exit: $?"
```

Expected: no output before the exit line, and `exit: 1`.

- [x] **Step 6: Confirm the tree is clean**

Run:

```bash
git status --short
```

Expected: no entries under `crates/` and no changes to any `Cargo.toml`. `?? Cargo.lock` is acceptable and must not be committed; commit nothing else.

- [x] **Step 7: Report completion**

Report the observed results: `5 passed` (sandbox-core), `26 passed` (mcp-server), clippy exit 0, `cargo fmt --check` exit 0, and both `rg` checks with no matches.

---

## Out of scope (do not implement)

- Request dispatcher, `main.rs`/binary target, MCP method routing (`initialize`, `tools/*`), `spawn_blocking`, Wasmtime.
- `McpWireError::Protocol` or any third variant; batch requests; SSE/`axum`; timeouts; cancellation; shutdown signals; TLS/sockets.
- `tracing` calls (invalid notifications stay silent), `tracing`/`axum` removal or changes.
- `RequestHandler` adapters for closures, boxed/dyn handlers, `Send` bounds, concurrency/pipelining.
- Read caps, invalid-UTF-8 recovery, or any wire-type/constant/`From`-impl/test changes.
- Manifest edits, new dependencies, edits to `docs/`, `.gitignore`, or committing `Cargo.lock`.
