//! In-memory cache of compiled WebAssembly modules.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use wasmtime::{Engine, Module};

use crate::SandboxError;

/// Maximum number of compiled modules kept in memory.
pub const DEFAULT_CACHE_CAPACITY: usize = 64;

/// Caches compiled modules so repeated executions skip compilation.
#[derive(Clone)]
pub struct ModuleCache {
    inner: Arc<RwLock<CacheInner>>,
    capacity: usize,
}

/// Modules held in memory, with the insertion order used to evict them.
#[derive(Default)]
struct CacheInner {
    entries: HashMap<Vec<u8>, Module>,
    order: VecDeque<Vec<u8>>,
}

impl ModuleCache {
    /// Creates a cache with the default capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CACHE_CAPACITY)
    }

    /// Creates a cache that keeps at most `capacity` modules.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(CacheInner::default())),
            capacity,
        }
    }

    /// Returns the compiled module for `wasm`, compiling it on the first call.
    pub fn get_or_compile(&self, engine: &Engine, wasm: &[u8]) -> Result<Module, SandboxError> {
        if let Some(module) = self.read().entries.get(wasm).cloned() {
            return Ok(module);
        }

        let module =
            Module::new(engine, wasm).map_err(|err| SandboxError::Compile(err.to_string()))?;
        self.insert(wasm.to_vec(), module.clone());
        Ok(module)
    }

    /// Number of compiled modules held by the cache.
    pub fn len(&self) -> usize {
        self.read().entries.len()
    }

    /// Whether the cache holds no modules.
    pub fn is_empty(&self) -> bool {
        self.read().entries.is_empty()
    }

    /// Stores a module, dropping the oldest one when the cache is full.
    fn insert(&self, key: Vec<u8>, module: Module) {
        let mut inner = self.write();

        if inner.entries.len() >= self.capacity
            && let Some(oldest) = inner.order.pop_front()
        {
            inner.entries.remove(&oldest);
        }

        inner.order.push_back(key.clone());
        inner.entries.insert(key, module);
    }

    fn read(&self) -> RwLockReadGuard<'_, CacheInner> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, CacheInner> {
        self.inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for ModuleCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::ModuleCache;
    use crate::SandboxError;

    const ADD_MODULE: &str = r#"(module
        (func (export "add") (param i32 i32) (result i32)
            local.get 0
            local.get 1
            i32.add))"#;

    #[test]
    fn invalid_modules_are_compile_errors() -> Result<(), Box<dyn std::error::Error>> {
        let engine = wasmtime::Engine::default();
        let cache = ModuleCache::new();

        match cache.get_or_compile(&engine, b"not a wasm module") {
            Err(SandboxError::Compile(_)) => Ok(()),
            other => Err(std::io::Error::other(format!("unexpected outcome: {other:?}")).into()),
        }
    }

    #[test]
    fn modules_are_compiled_once() -> Result<(), Box<dyn std::error::Error>> {
        let engine = wasmtime::Engine::default();
        let cache = ModuleCache::new();

        cache.get_or_compile(&engine, ADD_MODULE.as_bytes())?;
        cache.get_or_compile(&engine, ADD_MODULE.as_bytes())?;

        assert_eq!(cache.len(), 1);
        assert!(!cache.is_empty());
        Ok(())
    }

    #[test]
    fn the_cache_never_grows_past_its_capacity() -> Result<(), Box<dyn std::error::Error>> {
        let engine = wasmtime::Engine::default();
        let cache = ModuleCache::with_capacity(2);

        for index in 0..4 {
            let module =
                format!("(module (func (export \"f{index}\") (result i32) i32.const {index}))");
            cache.get_or_compile(&engine, module.as_bytes())?;
        }

        assert_eq!(cache.len(), 2);
        Ok(())
    }
}
