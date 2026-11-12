# Architectural Specification: mcp-wasm-sandbox

## 1. Component Boundaries & Layout
The system is architected as a decoupled Cargo Workspace to guarantee strict separation of concerns, compile-time boundary enforcement, and seamless domain testability.

```
┌────────────────────────────────────────────────────────┐
│                  crates/mcp-server                     │
│  (JSON-RPC Parser, Stdio Transport, SSE Axum Streams)  │
└───────────────────────────┬────────────────────────────┘
                            │ (Uses)
                            ▼
┌────────────────────────────────────────────────────────┐
│                  crates/sandbox-core                   │
│  (Wasmtime Engine, Module Cache, Resource Guardrails)  │
└───────────────────────────▲────────────────────────────┘
                            │ (Uses)
                            │
┌───────────────────────────┴────────────────────────────┐
│                crates/python-bindings                  │
│       (PyO3 C-Extension Module, GIL Management)        │
└────────────────────────────────────────────────────────┘
```

## 2. Deep Dive: Memory & Threading Topology

### 2.1 Async vs. CPU-Bound Boundary (The Tokio Rule)
* **The Problem**: WebAssembly instantiation, linear memory allocation, and runtime execution (JIT-compiled code) are heavy CPU-bound tasks. Running them directly inside a `Tokio` asynchronous worker thread will steal cycles, spike latency, and induce event-loop starvation.
* **The Solution**: All communication layers (`mcp-server`) capture and deserialize incoming requests asynchronously. However, the payload is immediately handed over to `tokio::task::spawn_blocking`. This offloads the heavy `Wasmtime` execution pipeline to a dedicated thread pool managed by Tokio for blocking operations, keeping the async I/O loops fluid.

### 2.2 Foreign Function Interface (FFI) Isolation (The PyO3 Rule)
* **The Problem**: When running inside a Python application (e.g., LangGraph or PydanticAI), Python's Global Interpreter Lock (GIL) freezes multi-threaded execution. If Rust locks the thread while waiting for Wasm execution, it deadlocks or slows down the entire pipeline.
* **The Solution**: The `python-bindings` crate intercepts the call, extracts native primitives, and immediately invokes `py.allow_threads(|| { ... })`. This explicitly drops the GIL, allowing Python to continue running other async/threaded tasks while Rust executes the WebAssembly payload in a clean, parallel native thread.

## 3. Sandboxing & Security Determinism

Every dynamic execution request undergoes multi-layered restriction inside `crates/sandbox-core`:

1. **Pre-compiled Module Cache**: Guest `.wasm` binaries are compiled into native `Precompiled` modules once and cached in an in-memory `DashMap`. Instantiations fetch from cache, achieving sub-millisecond cold starts.
2. **Deterministic Fuel Metering**: To counter malicious infinite loops (`loop {}`), `wasmtime::Store::set_fuel` is injected into every execution. If an agent's tool runs out of the allocated fuel allocation, the engine immediately traps, terminates the execution, and returns a controlled `McpWireError`.
3. **Linear Memory Ceilings**: A custom `wasmtime::ResourceLimiter` is tied to each store context. It intercepts allocations dynamically, ensuring no runtime script can trigger a Host Out-Of-Memory (OOM) attack by exceeding the hard-coded RAM envelope (e.g., max 32MB per sandbox).
4. **Least-Privilege System Access (WASI)**: By default, the `wasmtime_wasi::WasiCtx` is stripped bare. No environment variables, no arbitrary filesystem access, and no raw socket creation are inherited from the host environment unless explicitly white-listed by tool definitions.

## 4. Error Mapping Strategy
No internal faults are allowed to bubble up as a thread panic. The system uses a strict unified mapping pattern:
* Host-level execution, parsing, or I/O failures -> Mapped via `thiserror` into domain-specific internal Enums (`SandboxError`, `McpWireError`).
* Guest-level execution anomalies (OOM, out of fuel, divide by zero) -> Captured as `wasmtime::Trap`, intercepted, and serialized into standard JSON-RPC error frames (`code: -32603` or custom runtime bounds).
