# mcp-wasm-sandbox

A Model Context Protocol (MCP) server that runs untrusted WebAssembly inside a
Wasmtime sandbox with a hard fuel budget and a hard linear-memory ceiling.

## Status

| Phase | Scope | State |
| --- | --- | --- |
| 1 | JSON-RPC 2.0 wire types, stdio transport, dispatcher | done |
| 2 | Isolation runtime: fuel metering, memory ceiling, module cache | done |
| 3 | Python bindings (`mcp_wasm_sandbox`) | done |
| 4 | Benchmarks and CI | pending |

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

## Known limitations

- The `run_wasm` tool only accepts modules in the WebAssembly text format;
  base64-encoded binaries are not supported yet.
- Guest modules get no host imports at all, so WASI is not available.
- The module cache grows without bound and is not shared between processes.
- One request is handled at a time; there is no pipelining.

## License

MIT, see [LICENSE.md](LICENSE.md).
