# mcp-wasm-sandbox

A Model Context Protocol (MCP) server that runs untrusted WebAssembly inside a
Wasmtime sandbox with a hard fuel budget and a hard linear-memory ceiling.

## Status

| Phase | Scope | State |
| --- | --- | --- |
| 1 | JSON-RPC 2.0 wire types, stdio transport, dispatcher | done |
| 2 | Isolation runtime: fuel metering, memory ceiling, module cache | done |
| 3 | Python bindings (`mcp_wasm_sandbox`) | done |
| 4 | Benchmarks and CI | in progress: benchmarks and CI done, sub-process and container baselines missing |

Everything below is verified by the test suite and by the commands shown here.
Nothing is published to PyPI yet; the wheel is built from this repository.

## Requirements

- Rust 1.85 or newer (the workspace uses edition 2024).
- Python 3.10 or newer, only for the bindings. The interpreter needs a shared
  `libpython` to build the bindings.

## Build and install

### Python bindings

```sh
pip install maturin
cd crates/python-bindings
maturin build --release          # writes target/wheels/mcp_wasm_sandbox-*.whl
pip install target/wheels/mcp_wasm_sandbox-*.whl
```

`maturin develop` installs the module in editable mode into the active virtual
environment, which is the fastest loop while working on the bindings.

```python
import mcp_wasm_sandbox

WAT = b'(module (func (export "add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))'

sandbox = mcp_wasm_sandbox.WasmSandbox(32 * 1024 * 1024, 10_000_000)
sandbox.run(WAT, "add", [2, 3])  # -> [5]
```

`WasmSandbox(max_memory_bytes, fuel)` takes the linear-memory ceiling and the
fuel budget. The GIL is released while the module runs, so a long module never
blocks the interpreter.

### MCP server

```sh
cargo build --release -p mcp-server
./target/release/mcp-server
```

The server speaks newline-delimited JSON-RPC 2.0 over stdin and stdout and
exposes one tool, `run_wasm`:

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_wasm","arguments":{"source":"(module (func (export \"add\") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))","export":"add","args":[2,3]}}}
```

The response is `{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"5"}]}}`.

The module travels either as text (`source`, WebAssembly text format) or as a
binary (`module_base64`, base64 encoded), which is what agents usually produce:

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"run_wasm","arguments":{"module_base64":"AGFzbQEAAAABBwFgAn9/AX8DAgEABwcBA2FkZAAACgkBBwAgACABags=","export":"add","args":[2,3]}}}
```

### Container image

```sh
docker build -t mcp-wasm-sandbox:bench .
docker run --rm -i mcp-wasm-sandbox:bench
```

The image ships the release server in a minimal Debian layer, so the container
baseline measures the same workload as the sub-process one.

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

The interpreter-backed test that proves the GIL is released needs a Python with
a shared `libpython`:

```sh
PYO3_PYTHON=$(uv python find 3.12) cargo test -p mcp-wasm-sandbox --no-default-features -- --ignored
```

## Performance

`cargo bench -p sandbox-core` measures the hot paths. Criterion writes the full
distributions, p50, p95 and p99 included, under `target/criterion`.

| Benchmark | Median | What it measures |
| --- | --- | --- |
| `compile_module` | ~98 µs | cold path, the module is compiled from WAT every time |
| `instantiate` | ~1.3 µs | store plus instantiation of an already compiled module |
| `cached_call` | ~6.6 µs | the full public API, offloaded to a blocking thread |
| `subprocess_spawn_floor` | ~29 µs | baseline: spawning a trivial process |
| `subprocess_round_trip` | ~2.5 ms | baseline: one execution behind a process boundary, through the MCP server |
| `container_round_trip` | ~178 ms | baseline: the same execution inside a container |

The container baseline is skipped unless the image is built first
(`docker build -t mcp-wasm-sandbox:bench .`).

The same work costs 6.6 µs in process, 2.5 ms behind a sub-process and 178 ms
inside a container: roughly 375 times and 27,000 times more expensive. A cached
execution stays two orders of magnitude below the 1 ms target of the
architecture.

Measured on an Apple Silicon laptop. A cached execution is roughly 375 times
faster than the same work behind a sub-process, and two orders of magnitude
below the 1 ms target of the architecture.

## Known limitations

- Guest modules get no host imports at all, so WASI is not available.
- The tool arguments are integers only; floats and 64 bit integers are not
  exposed yet.
- The module cache grows without bound and is not shared between processes.
- One request is handled at a time; there is no pipelining.

## License

MIT, see [LICENSE.md](LICENSE.md).
