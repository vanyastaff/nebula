#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Sandbox
//!
//! Plugin isolation and sandboxing for the Nebula workflow engine.
//!
//! This crate provides:
//! - [`SandboxRunner`] trait — common interface for action execution within isolation
//! - [`InProcessSandbox`] — trusted in-process execution (for built-in actions)
//! - [`SandboxedContext`] — wrapped action context with capability checks
//!
//! Future (behind feature flags):
//! - `WasmSandbox` — WASM-based sandbox via wasmtime (for community plugins)

mod in_process;
mod runner;

pub use in_process::InProcessSandbox;
pub use runner::{ActionExecutor, ActionExecutorFuture, SandboxRunner, SandboxedContext};
