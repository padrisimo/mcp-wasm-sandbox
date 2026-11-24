# Phase 4: Benchmarks, CI and Container Baseline

**Date:** 2026-11-13
**Status:** Implemented
**Scope:** `crates/sandbox-core`, `.github/workflows`, `Dockerfile`

## Goal

Close the last roadmap slice: measure the sandbox latency with a real
benchmark suite, compare it against the sub-process and container alternatives,
and make every gate run automatically on every push.

## Non-Goals

- **Publishing the wheel.** The wheel is built and imported in CI, not uploaded
  to PyPI.
- **Cross-version or cross-platform latency tables.** The numbers are from one
  Apple Silicon laptop.
- **Container image publishing.** The image is a local benchmark artifact.

## Context

`MEMORY.md` asked for a `criterion` suite, for p50/p95/p99 latency footprints
against sub-processes and containers, and for GitHub workflows that lint and
test the workspace. The architecture targets sub-millisecond cold starts and
claims Wasmtime beats containers and sub-processes; nothing measured that yet.

## Proposed Architecture

### Benchmarks (`crates/sandbox-core/benches/execution.rs`)

Criterion 0.8, one `benches` group, `harness = false`:

- `compile_module`: `Module::new` on a WAT module, the cold path.
- `instantiate`: store plus instantiation of an already compiled module.
- `cached_call`: the public API, `Sandbox::call_i32`, offloaded to a blocking
  thread. This is the number the architecture cares about.
- `subprocess_spawn_floor`: spawning `/bin/true`, the floor of any sub-process
  approach.
- `subprocess_round_trip`: spawning `target/release/mcp-server` and running one
  `tools/call` through it, the same workload as `cached_call` behind a process
  boundary.
- `container_round_trip`: the same call inside `mcp-wasm-sandbox:bench`, the
  container baseline.

Every benchmark closure returns its `Result` instead of unwrapping it, so the
repository-wide grep gate stays meaningful. The container benchmark is skipped
when the image is missing.

### Container image (`Dockerfile`, `.dockerignore`)

A multi-stage build: `rust:1-slim` compiles the release server, and
`debian:stable-slim` carries only the binary. `.dockerignore` keeps `target`,
`.git` and the generated docs out of the build context.

### CI (`.github/workflows/ci.yml`)

Two jobs on `ubuntu-latest`:

- `rust`: format check, `clippy -D warnings`, `cargo test --workspace
  --exclude mcp-wasm-sandbox`, and `cargo bench --no-run`.
- `python`: the binding tests and the GIL test against the interpreter, the
  wheel build and import with maturin, the server build, and the end-to-end
  pytest suite that drives the server as an MCP client.

The bindings are excluded from the Rust job because an extension module cannot
be linked into a test binary on Linux; the Python job covers them instead, with
the extension-module feature turned off.

## Testing Strategy

The benchmark suite is verified by `cargo bench --no-run` in CI and by running
it locally. The end-to-end suite (`tests/test_mcp_stdio.py`) drives the server
over stdio exactly like an MCP client and asserts the `initialize`,
`tools/list`, `tools/call` and error responses.

## Risks And Mitigations

1. **The container baseline depends on a Docker daemon.** The benchmark skips
   itself when the image is not built, so the suite never fails for that.
2. **The sub-process baseline depends on the release binary.** Same treatment.
3. **Latency numbers are machine specific.** The README states the machine and
   the commit they come from.
4. **The image build is slow** because it compiles Wasmtime inside the
   container. It is a local artifact, not a CI step.

## Decision Summary

- **One Criterion group** with six benchmarks: three in-process, two
  sub-process baselines, one container baseline.
- **Benchmarks never unwrap**: the closures return `Result`.
- **The container image** is a two-stage build ending in `debian:stable-slim`.
- **CI has two jobs**, and the bindings are tested apart from the Rust job.
- **The end-to-end suite** lives in `tests/` and runs in CI.
