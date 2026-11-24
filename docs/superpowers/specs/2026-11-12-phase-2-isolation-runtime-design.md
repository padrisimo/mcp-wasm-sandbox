# Phase 2: Isolation Runtime

**Date:** 2026-11-12
**Status:** Implemented
**Scope:** `crates/sandbox-core`, `crates/mcp-server`

## Goal

Give the sandbox a real isolation runtime: a shared Wasmtime engine, an in-memory module cache, and execution under a hard fuel and linear-memory budget, offloaded from the async runtime. Expose it to MCP clients as the `run_wasm` tool.

## Non-Goals

- **WASI.** The linker exposes no host imports at all, which is stricter than a bare `WasiCtx`; `wasmtime-wasi` is not a dependency yet.
- **On-disk precompiled artifacts.** `Module::serialize`/`deserialize` and a file cache are not implemented; the cache is in-memory only.
- **Binary module input.** The `run_wasm` tool takes WAT text; base64-encoded `.wasm` is not supported yet.
- **Non-`i32` tool arguments.** `call_i32` covers the common case; floats and `i64` are reachable through the lower-level `call` API.
- **Concurrency.** One request at a time, as in Phase 1b.

## Context

`architecture.md` §2.1 requires CPU-bound Wasm work to leave the Tokio worker threads; §3.1 asks for a compiled-module cache; §3.2 for deterministic fuel metering; §3.3 for a hard linear-memory ceiling. Phase 1 delivered the wire protocol, the stdio transport, and a dispatcher with no tools.

## Proposed Architecture

### `sandbox-core`

- `cache.rs` — `ModuleCache`: `Arc<RwLock<HashMap<Vec<u8>, Module>>>`, keyed by module bytes. `get_or_compile` returns a cached `Module` or compiles and inserts it. `std::sync::RwLock` replaces the `DashMap` named in `architecture.md` §3.1: a poisoned lock is recovered with `into_inner`, so no `unwrap` is needed and no dependency is added.
- `sandbox.rs` — `Sandbox` holds the `Engine`, the cache, and a `SandboxConfig { max_memory_bytes, fuel }` (defaults: 32 MiB, 10M fuel). The engine sets `consume_fuel(true)` and `wasm_threads(false)`.
  - `call(wasm, export, args: &[Val])` clones the engine and cache into a `tokio::task::spawn_blocking` closure that compiles (cached), instantiates, and calls the export.
  - `call_i32(wasm, export, args: &[i32])` is the engine-agnostic entry point used by the MCP layer, so `mcp-server` never depends on `wasmtime`.
  - The store data implements `ResourceLimiter`: a growth request above the ceiling returns an error, which turns `memory.grow` into a trap and records the denial, so the failure is reported as `SandboxError::MemoryLimit` instead of a silent `-1`.
  - Failures are classified: a denied growth → `MemoryLimit`; `Trap::OutOfFuel` or an empty fuel gauge → `FuelExhausted`; anything else → `Execution`.
- `lib.rs` — `SandboxError` gains one variant, `Engine(String)`, because engine construction can now fail. The Phase 1 spec fixed nine variants; this is the only addition.

### `mcp-server`

- `Dispatcher` now owns a `Sandbox` and advertises one tool, `run_wasm`, with `source` (WAT text), `export`, and optional `args` (integers).
- `tools/call` with `run_wasm` compiles and runs the module; a guest failure is reported as an MCP tool result with `isError: true` and the error text, while protocol problems (unknown tool, invalid arguments) stay JSON-RPC errors.
- `main.rs` builds the sandbox with the default budget and serves stdio.

## Testing Strategy

TDD, tests before implementation, no `unwrap`/`expect`/`panic!`.

`sandbox-core`: `add` returns the sum; an invalid module is a `Compile` error; a missing export is an `Execution` error; a spin loop with a small budget is `FuelExhausted`; a memory growth above the ceiling is `MemoryLimit`; the cache compiles once; a heavy module leaves the async runtime free to run other tasks (multi-thread runtime, `AtomicBool` flag).

`mcp-server`: `tools/list` advertises `run_wasm`; `tools/call` returns the module output; a runaway module returns `isError: true`; invalid arguments are `INVALID_PARAMS`; an unknown tool is `INVALID_PARAMS`; an unknown method is `METHOD_NOT_FOUND`; `serve` with the real dispatcher answers an `initialize` request.

Gates: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, and clean `rg` for `unwrap`/`expect`/`panic!`. End-to-end: three requests piped into `cargo run -q -p mcp-server`.

## Risks And Mitigations

1. **The tool only accepts WAT text.** Binary modules need base64 support. *Mitigation:* documented in the tool description; the sandbox itself accepts raw bytes.
2. **Fuel accounting is per store, not per module.** A cached module can be instantiated repeatedly, each with a fresh budget. *Mitigation:* a new store per execution.
3. **The memory ceiling is per store.** Concurrent sandboxes each get their own ceiling. *Mitigation:* the budget is part of `SandboxConfig` and the server uses one sandbox.
4. **The cache grows without bound.** Every distinct module stays resident. *Mitigation:* documented; an LRU cap belongs with the on-disk cache slice.
5. **`call_i32` assumes `i32` exports.** Other signatures fail with a type error. *Mitigation:* the generic `call` API stays public for advanced callers.

## Decision Summary

- **`sandbox-core` gains `cache.rs` and `sandbox.rs`**, exporting `Sandbox`, `SandboxConfig`, `ModuleCache`, `DEFAULT_FUEL`, and `DEFAULT_MAX_MEMORY_BYTES`.
- **Execution always runs inside `spawn_blocking`**; the async runtime stays free.
- **Fuel is enabled at the engine**, the memory ceiling is a hard `ResourceLimiter` error, and both are reported as dedicated `SandboxError` variants.
- **The module cache is `std::sync::RwLock<HashMap<..>>`**, not `DashMap`.
- **`SandboxError::Engine` is added** for engine construction failures.
- **`mcp-server` exposes one tool, `run_wasm`**, and never depends on `wasmtime`.
- **No new dependencies, no WASI, no on-disk cache, no binary module input.**
