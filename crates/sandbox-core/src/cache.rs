//! In-memory cache of compiled WebAssembly modules.

use std::collections::HashMap;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use wasmtime::{Engine, Module};

use crate::SandboxError;

/// Caches compiled modules so repeated executions skip compilation.
#[derive(Clone, Default)]
pub struct ModuleCache {
    modules: Arc<RwLock<HashMap<Vec<u8>, Module>>>,
}

impl ModuleCache {
    /// Creates an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the compiled module for `wasm`, compiling it on the first call.
    pub fn get_or_compile(&self, engine: &Engine, wasm: &[u8]) -> Result<Module, SandboxError> {
        if let Some(module) = self.read().get(wasm).cloned() {
            return Ok(module);
        }

        let module =
            Module::new(engine, wasm).map_err(|err| SandboxError::Compile(err.to_string()))?;
        self.write().insert(wasm.to_vec(), module.clone());
        Ok(module)
    }

    /// Number of compiled modules held by the cache.
    pub fn len(&self) -> usize {
        self.read().len()
    }

    /// Whether the cache holds no modules.
    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    fn read(&self) -> RwLockReadGuard<'_, HashMap<Vec<u8>, Module>> {
        self.modules
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, HashMap<Vec<u8>, Module>> {
        self.modules
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
}
