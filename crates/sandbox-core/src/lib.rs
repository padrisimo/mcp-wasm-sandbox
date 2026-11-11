//! Core execution engine for the MCP WebAssembly sandbox.
//!
//! Phase 1 defines [`SandboxError`], the domain error type shared by the
//! workspace. Engine configuration, the module cache, and Wasmtime execution
//! arrive in a later phase.
