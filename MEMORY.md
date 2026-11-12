# Project Memory & Context: mcp-wasm-sandbox

## 1. Executive Summary & Intent
This repository houses `mcp-wasm-sandbox`, an enterprise-grade, ultra-low latency Model Context Protocol (MCP) server and runtime execution environment written in Rust. It specializes in running untrusted agent-generated code or dynamic tools inside tightly sandboxed WebAssembly (`Wasmtime`) instances.

The primary engineering goal is to displace traditional Docker/container-based tool execution, which suffers from massive cold-start penalties (~hundreds of milliseconds) and high memory overhead, replacing it with an infrastructure that guarantees sub-millisecond (< 1ms) instantiation and deterministic resource isolation.

## 2. Core Constraints & Guarantees
To ensure high-performance production readiness and pass strict technical reviews by US/EU architecture teams, the project enforces the following guardrails:
* **Zero Panic Strategy**: Absolute ban on `.unwrap()` or `.expect()` in non-test profiles. All failures must map to concrete `thiserror` variants and propagate safely.
* **Non-Blocking Runtime**: WebAssembly compilation and execution are highly CPU-bound. They must never occupy a Tokio async worker thread. All Wasm operations must run enclosed in `tokio::task::spawn_blocking`.
* **Hardened Sandboxing**: Every `wasmtime::Store` must deploy an explicit `ResourceLimiter` for linear memory allocation ceiling bounds and engine-level fuel metering via `Store::set_fuel` to kill infinite loops or abuse vectors.
* **GIL-Free Foreign Function Interface (FFI)**: When invoked natively from Python ecosystems via `PyO3`, the Python Global Interpreter Lock (GIL) must be explicitly dropped using `py.allow_threads` during the active Rust execution pipeline.

## 3. High-Level Modular Design
The project uses a Cargo Workspace to split concerns cleanly (YAGNI enforcement):
1. `crates/sandbox-core`: Independent library managing Wasmtime configurations, pre-compiled module caching, resource limiting, and execution.
2. `crates/mcp-server`: Network/I/O interface implementing JSON-RPC serialization (`serde_json`) over `stdio` and asynchronous `HTTP/SSE` streams (`Axum`).
3. `crates/python-bindings`: C-extension layer (`PyO3`) built using `Maturin` to expose the execution sandbox to popular agentic tools (LangGraph, PydanticAI).

## 4. Current State & Incremental Roadmap

### Phase 1: Core Definitions & Protocol Wire [DONE]
* Setup workspace architecture, global dependencies (`Cargo.toml`), and local crate wiring.
* Define `thiserror` domain models for sandbox anomalies (`SandboxError`, `McpWireError`).
* Model base MCP lifecycle messages (`initialize`, `tools/list`, `tools/call`) using serde derives.
* Implement asynchronous stream transport over standard input/output, with a minimal dispatcher and the `mcp-server` binary.

### Phase 2: Isolation Runtime [DONE]
* Initialize Wasmtime global configuration engine (`consume_fuel`, threads disabled).
* Design a pre-compiled `.wasm` module cache structure (in-memory `ModuleCache`, keyed by module bytes).
* Configure explicit memory caps and fuel restrictions (32 MiB ceiling, 10M fuel, hard `ResourceLimiter` denial).
* Validate thread offloading patterns with mock heavy computation modules (`spawn_blocking`, runtime stays responsive).

### Phase 3: Python Integration & Wheel Building [DONE]
* Create `WasmSandbox` PyO3 bindings (`crates/python-bindings`, module `mcp_wasm_sandbox`).
* Test safe GIL releasing procedures (`py.detach` around the sandbox run).
* Setup local `Maturin` dev builds (wheel built with `maturin build --release`).

### Phase 4: Benchmarking & CI/CD Pipeline [CURRENT]
* Author a performance testing suite using `criterion` (`cargo bench -p sandbox-core`: compile ~98 µs, instantiate ~1.3 µs, cached call ~6.6 µs).
* Graph p50/p95/p99 latency footprints vs sub-processes and containers. [PARTIAL: sub-process baseline ~2.5 ms per round trip and ~29 µs just to spawn a process; the container baseline needs a Docker daemon.]
* Configure automated GitHub workflows for workspace linting (`clippy`) and testing (`.github/workflows/ci.yml`).
