//! Latency benchmarks for the sandbox hot paths.
//!
//! Run them with `cargo bench -p sandbox-core`. Criterion writes the full
//! distributions, including p50, p95 and p99, under `target/criterion`.

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

criterion_group!(benches, bench_compile, bench_instantiate, bench_cached_call);
criterion_main!(benches);
