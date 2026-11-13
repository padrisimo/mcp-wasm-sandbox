//! Latency benchmarks for the sandbox hot paths and the sub-process baseline.
//!
//! Run them with `cargo bench -p sandbox-core`. Criterion writes the full
//! distributions, including p50, p95 and p99, under `target/criterion`.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use criterion::{Criterion, criterion_group, criterion_main};
use sandbox_core::{Sandbox, SandboxConfig};
use wasmtime::{Engine, Linker, Module, Store};

const ADD_MODULE: &str = r#"(module
    (func (export "add") (param i32 i32) (result i32)
        local.get 0
        local.get 1
        i32.add))"#;

/// Compiles the module from scratch: the cold path.
fn bench_compile(c: &mut Criterion) {
    let engine = Engine::default();
    assert!(Module::new(&engine, ADD_MODULE).is_ok());

    c.bench_function("compile_module", |b| {
        b.iter(|| Module::new(&engine, ADD_MODULE));
    });
}

/// Creates a store and instantiates an already compiled module.
fn bench_instantiate(c: &mut Criterion) {
    let engine = Engine::default();
    let Ok(module) = Module::new(&engine, ADD_MODULE) else {
        eprintln!("the benchmark module does not compile");
        return;
    };

    c.bench_function("instantiate", |b| {
        b.iter(|| {
            let mut store = Store::new(&engine, ());
            let linker = Linker::new(&engine);
            linker.instantiate(&mut store, &module)
        });
    });
}

/// Runs a cached module through the public API, offloaded to a blocking thread.
fn bench_cached_call(c: &mut Criterion) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        eprintln!("the benchmark runtime does not build");
        return;
    };
    let Ok(sandbox) = Sandbox::new(SandboxConfig::default()) else {
        eprintln!("the benchmark sandbox does not build");
        return;
    };

    c.bench_function("cached_call", |b| {
        b.iter(|| runtime.block_on(sandbox.call_i32(ADD_MODULE.as_bytes(), "add", &[2, 3])));
    });
}

/// Baseline: the floor of any sub-process approach, spawning a trivial process.
fn bench_subprocess_spawn(c: &mut Criterion) {
    c.bench_function("subprocess_spawn_floor", |b| {
        b.iter(|| Command::new("/bin/true").status().ok());
    });
}

/// Baseline: one execution behind a process boundary, through the MCP server.
fn bench_subprocess_round_trip(c: &mut Criterion) {
    let binary = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/mcp-server");
    if !binary.exists() {
        eprintln!(
            "skipping the sub-process baseline: build it with `cargo build --release -p mcp-server`"
        );
        return;
    }

    let request = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"run_wasm\",\
         \"arguments\":{{\"source\":\"{ADD_MODULE}\",\"export\":\"add\",\"args\":[2,3]}}}}}}\n"
    );

    c.bench_function("subprocess_round_trip", |b| {
        b.iter(|| {
            let mut child = match Command::new(&binary)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
            {
                Ok(child) => child,
                Err(err) => {
                    eprintln!("cannot spawn the server: {err}");
                    return None;
                }
            };

            if let Some(stdin) = child.stdin.as_mut()
                && let Err(err) = stdin.write_all(request.as_bytes())
            {
                eprintln!("cannot write the request: {err}");
            }
            drop(child.stdin.take());

            child.wait_with_output().ok().map(|out| out.stdout.len())
        });
    });
}

/// Baseline: one execution inside a container, through the same MCP server.
fn bench_container_round_trip(c: &mut Criterion) {
    const IMAGE: &str = "mcp-wasm-sandbox:bench";

    let status = Command::new("docker")
        .args(["image", "inspect", IMAGE])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !matches!(status, Ok(status) if status.success()) {
        eprintln!("skipping the container baseline: build it with `docker build -t {IMAGE} .`");
        return;
    }

    let request = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"run_wasm\",\
         \"arguments\":{{\"source\":\"{ADD_MODULE}\",\"export\":\"add\",\"args\":[2,3]}}}}}}\n"
    );

    c.bench_function("container_round_trip", |b| {
        b.iter(|| {
            let mut child = match Command::new("docker")
                .args(["run", "--rm", "-i", IMAGE])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => child,
                Err(err) => {
                    eprintln!("cannot start the container: {err}");
                    return None;
                }
            };

            if let Some(stdin) = child.stdin.as_mut()
                && let Err(err) = stdin.write_all(request.as_bytes())
            {
                eprintln!("cannot write the request: {err}");
            }
            drop(child.stdin.take());

            child.wait_with_output().ok().map(|out| out.stdout.len())
        });
    });
}

criterion_group!(
    benches,
    bench_compile,
    bench_instantiate,
    bench_cached_call,
    bench_subprocess_spawn,
    bench_subprocess_round_trip,
    bench_container_round_trip
);
criterion_main!(benches);
