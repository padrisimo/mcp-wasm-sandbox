# Phase 5: Hardening, Observability and Supply Chain

**Date:** 2026-11-14
**Status:** Approved (design phase)
**Scope:** `crates/sandbox-core`, `crates/mcp-server`, `crates/python-bindings`, `.github/workflows`

## Goal

Make the server defensible before more agents use it: bound every execution in
wall-clock time, speak the MCP lifecycle correctly, let Python see the guest
output, leave a trace of every execution, and check the dependency tree for
known vulnerabilities.

## Non-Goals

- **Full `streamable-http` MCP transport.** No sessions, no server to client
  stream: the server never sends messages on its own, so both are optional in
  the specification.
- **Cancellation, progress notifications, streaming results.**
- **A tool registry.** There is one tool.
- **Deterministic clock and entropy for the guest.** Deferred: it needs a
  product decision about whether guests may observe the host time.
- **Auth on the HTTP transport.** The transport binds to loopback by default
  and warns when it does not.

## Context

The sandbox bounds the *work* a guest can do with fuel and with a linear memory
ceiling, but not the *wall-clock time* it can take: compilation is not fuel
metered, and the dispatcher handles one request at a time, so a slow guest
blocks every client, including other HTTP clients that share the handler.

Two protocol details are also loose. `initialize` echoes the version the client
asked for instead of the version the server supports, and the server accepts
requests before `initialize`. The Python bindings cannot see the guest output
even though WASI now captures it.

On the supply chain side, the workspace has grown three dependencies since
Phase 4 (`wasmtime-wasi`, `base64`, `tower`) and CI never checks for known
advisories.

## Proposed Architecture

### 1. Wall-clock timeout (`sandbox-core`)

`SandboxConfig` gains `timeout: Duration`, five seconds by default. The
`call_with_output` future is wrapped in `tokio::time::timeout`, so the bound
covers compilation, instantiation and execution. On expiry the call returns the
new `SandboxError::Timeout { after }`.

The blocking task cannot be cancelled once it started, so it keeps running until
the fuel budget ends it; the timeout stops the *caller* from waiting, not the
guest from running. The comment in the code says so.

### 2. MCP lifecycle (`mcp-server`)

- `SUPPORTED_PROTOCOL_VERSION` is a constant of the crate. `initialize`
  answers with that version, never with the version the client asked for: the
  specification requires the server to answer with a version it supports.
- The `Dispatcher` remembers whether it answered `initialize`. Requests other
  than `ping` are refused with `INVALID_REQUEST` until then, and a second
  `initialize` is refused with `INVALID_REQUEST` too. `ping` is always allowed.

### 3. Guest output from Python (`python-bindings`)

`WasmSandbox.run_with_output` returns the results and the captured stdout.
`run` keeps returning only the results.

### 4. Observability (`mcp-server`)

- Every tool call logs one line at `info`: the tool, the duration, the fuel
  spent and the outcome. To know the fuel spent, `SandboxOutput` gains
  `fuel_used`.
- The HTTP transport warns when it binds an address that is not loopback.

### 5. Supply chain (CI)

- `cargo audit` runs on every push, with the tracked `Cargo.lock`.
- A scheduled nightly job runs the criterion suite and uploads the report as an
  artifact, so a latency regression is visible even though the pull requests
  only compile the benchmarks.

## Files To Change

**Modified:** `crates/sandbox-core/src/sandbox.rs` (timeout, `SandboxError::Timeout`,
`fuel_used`), `crates/sandbox-core/src/lib.rs` (re-export), `crates/mcp-server/src/dispatcher.rs`
(lifecycle, version, log), `crates/mcp-server/src/lib.rs` (version constant),
`crates/mcp-server/src/http.rs` (bind warning), `crates/mcp-server/src/main.rs`
(timeout default), `crates/python-bindings/src/lib.rs` (`run_with_output`),
`.github/workflows/ci.yml` (audit, nightly benchmarks), `README.md`, `MEMORY.md`.

**New:** none.

## Testing Strategy

TDD, tests first, no `unwrap`, `expect` or `panic!` anywhere.

- `sandbox-core`: a module that never returns is answered with
  `SandboxError::Timeout`; a fast module is not; `fuel_used` is reported and
  is below the budget.
- `mcp-server`: `initialize` answers with the supported version even when the
  client asks for another one; a request before `initialize` is refused; a
  second `initialize` is refused; `ping` always works.
- `python-bindings`: `run_with_output` returns the printed text.
- The end-to-end suite keeps passing, and the CI gates stay green.

## Risks And Mitigations

1. **The timeout cannot stop a running guest.** It only stops the caller from
   waiting. The fuel budget is the real bound; the timeout is the net for the
   compilation phase, which fuel does not meter. Documented in the code.
2. **A five second timeout may be too tight for a legitimate heavy module.**
   It is configuration, and the fuel budget is the primary bound.
3. **`cargo audit` may fail on an advisory in a transitive dependency.** The
   gate is added only after running it locally; if it reports something, the
   dependency is upgraded or the advisory is documented before the gate lands.
4. **The lifecycle check may break clients that skip `initialize`.** That is
   the specification, and `ping` stays open for liveness checks.
5. **The nightly benchmark job needs a runner.** It is scheduled, not blocking,
   so a failure there does not hold the pipeline.

## Decision Summary

- **Every execution has a wall-clock bound** of five seconds by default, with a
  new `SandboxError::Timeout`.
- **`initialize` answers with the server's version** and the server enforces
  the lifecycle, with `ping` always allowed.
- **Python sees the guest output** through `run_with_output`.
- **Every tool call is logged** with duration, fuel spent and outcome; the HTTP
  transport warns on a non-loopback bind.
- **CI runs `cargo audit`** on every push and the benchmarks nightly.
- **No new dependencies**, no transport work, no tool registry.
