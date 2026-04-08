#![deny(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Sandbox
//!
//! Plugin isolation and sandboxing for the Nebula workflow engine.
//!
//! This crate provides:
//! - [`SandboxRunner`] trait — common interface for action execution within isolation
//! - [`InProcessSandbox`] — trusted in-process execution (for built-in actions)
//!
//! With the `wasm` feature:
//! - [`WasmSandbox`](wasm::WasmSandbox) — WASM-based sandbox via wasmtime (for community plugins)
//! - [`WasmPluginLoader`](wasm::WasmPluginLoader) — loads and caches `.wasm` components

mod in_process;
mod runner;
#[cfg(feature = "wasm")]
pub mod wasm;

pub use in_process::InProcessSandbox;
pub use runner::{ActionExecutor, ActionExecutorFuture, SandboxRunner, SandboxedContext};
