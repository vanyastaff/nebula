//! WASM-based sandbox for community plugins.
//!
//! Uses [extism](https://extism.org) (wasmtime-based) to load `.wasm` plugins.
//! Plugins export `metadata` and `execute` functions.
//! The host provides HTTP capability via host functions.
//!
//! ## Plugin contract
//!
//! A `.wasm` plugin must export:
//! - `metadata() -> JSON string` — plugin name, version, action descriptors
//! - `execute(JSON string) -> JSON string` — `{"action_key": "...", "input": {...}}`

mod loader;
mod sandbox;

pub use loader::WasmPluginLoader;
pub use sandbox::WasmSandbox;
