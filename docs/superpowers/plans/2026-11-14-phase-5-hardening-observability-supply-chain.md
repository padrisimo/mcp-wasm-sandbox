# Phase 5 Implementation Plan

**Spec:** `docs/superpowers/specs/2026-11-14-phase-5-hardening-observability-supply-chain-design.md`

TDD: tests first, red confirmed locally, tests and implementation committed together. No `unwrap`, `expect` or `panic!`. Run cargo with `source "$HOME/.cargo/env" &&`.

## Task 1: Wall-clock timeout in the sandbox

- [ ] **Step 1: Write the failing tests** in `crates/sandbox-core/src/sandbox.rs`
  - `a_module_that_never_returns_times_out`: a module with an infinite loop and a
    `SandboxConfig { timeout: Duration::from_millis(50), .. }` → `Err(SandboxError::Timeout { .. })`.
  - `a_fast_module_is_not_timed_out`: the `add` module with the default config → `Ok`.
  - `the_fuel_spent_is_reported`: `SandboxOutput.fuel_used` is below the budget and above zero for a module that burns fuel.
- [ ] **Step 2: Red check** — `cargo test -p sandbox-core` fails (no `Timeout` variant, no `timeout` field, no `fuel_used`).
- [ ] **Step 3: Implement**
  - `SandboxError::Timeout { after: Duration }` with the message `wasm execution timed out after {after:?}`.
  - `SandboxConfig { max_memory_bytes, fuel, timeout }`, default five seconds.
  - `call_with_output` wraps the blocking task in `tokio::time::timeout`; on expiry it returns the timeout error. The blocking task keeps running until the fuel ends it; say so in a comment.
  - `SandboxOutput { results, stdout, fuel_used }`, filled from `store.get_fuel()`.
- [ ] **Step 4: Green check** — `cargo test -p sandbox-core`, clippy and fmt clean.
- [ ] **Step 5: Commit** `feat(sandbox-core): bound executions with a wall-clock timeout`.

## Task 2: MCP lifecycle and version

- [ ] **Step 1: Write the failing tests** in `crates/mcp-server/src/dispatcher.rs`
  - `initialize_answers_with_the_supported_version`: the client asks for `"1999-01-01"`, the answer is `"2024-11-05"`.
  - `requests_before_initialize_are_refused`: `tools/list` first → `INVALID_REQUEST` with `"server not initialized"`.
  - `a_second_initialize_is_refused`: → `INVALID_REQUEST` with `"already initialized"`.
  - `ping_always_works`: `ping` before `initialize` → success.
- [ ] **Step 2: Red check** — `cargo test -p mcp-server` fails.
- [ ] **Step 3: Implement**
  - `SUPPORTED_PROTOCOL_VERSION` constant in `mcp-server`.
  - `Dispatcher { sandbox, initialized: bool }`; `initialize` sets it and answers with the supported version.
- [ ] **Step 4: Green check** — `cargo test -p mcp-server`, clippy and fmt clean.
- [ ] **Step 5: Commit** `feat(mcp-server): enforce the MCP lifecycle and answer with the supported version`.

## Task 3: Observability

- [ ] **Step 1: Write the failing test** in `crates/mcp-server/src/dispatcher.rs`: a tool call logs one line. Asserting a log line needs a subscriber; instead assert the data the log carries is available: skip the log assertion and rely on the review.
- [ ] **Step 2: Implement**
  - The dispatcher measures the call and logs `tracing::info!(tool, duration_ms, fuel_used, outcome, "tool call finished")`.
  - `serve_http` warns when the bound address is not loopback.
- [ ] **Step 3: Green check** — `cargo test --workspace`, clippy and fmt clean.
- [ ] **Step 4: Commit** `feat(mcp-server): log every tool call and warn on a public bind`.

## Task 4: Guest output from Python

- [ ] **Step 1: Write the failing test** in `crates/python-bindings/src/lib.rs`: a module that prints returns the text through `run_with_output`.
- [ ] **Step 2: Red check** — the method does not exist.
- [ ] **Step 3: Implement** `run_with_output` returning `(Vec<i32>, String)`.
- [ ] **Step 4: Green check** — `cargo test --workspace`, clippy and fmt clean.
- [ ] **Step 5: Commit** `feat(python-bindings): expose the guest output`.

## Task 5: Supply chain in CI

- [ ] **Step 1: Run `cargo audit` locally.** If it reports an advisory, upgrade or document it before adding the gate.
- [ ] **Step 2: Add to `.github/workflows/ci.yml`**
  - `rustsec/audit-check@v2` in the `rust` job.
  - A `benchmarks` job with `if: github.event_name == 'schedule'` that runs `cargo bench -p sandbox-core` and uploads `target/criterion`.
  - A `schedule` trigger with a nightly cron.
- [ ] **Step 3: Validate the YAML** with `python -c "import yaml; yaml.safe_load(...)"`.
- [ ] **Step 4: Commit** `ci: audit the dependency tree and run the benchmarks nightly`.

## Task 6: Docs and final gates

- [ ] **Step 1: Update `README.md`** with the timeout, the lifecycle, `run_with_output` and the audit gate.
- [ ] **Step 2: Update `MEMORY.md`** with the Phase 5 line.
- [ ] **Step 3: Final gates** — `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `rg -n "\.unwrap\(\)|\.expect\(" crates`, `rg -n 'panic!' crates`, `pytest tests -q`.
- [ ] **Step 4: Push** and wait for the CI run to be green.
