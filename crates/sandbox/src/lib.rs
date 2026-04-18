#![deny(unsafe_code)]
#![warn(missing_docs)]

//! # nebula-sandbox — Process Sandboxing (Correctness Boundary)
//!
//! Provides two execution modes and the plugin discovery / capability model.
//!
//! **This is not a security boundary against malicious native code.**
//! Canon §12.6 is the normative statement: in-process execution provides
//! correctness and cooperative cancellation; child-process execution provides
//! OS-namespace separation via a duplex JSON envelope over UDS / Named Pipe
//! (ADR 0006). WASM / WASI is an explicit non-goal (§12.6).
//!
//! ## Key types
//!
//! - `InProcessSandbox` — trusted in-process dispatch; no isolation.
//! - `ProcessSandbox` — child-process execution via JSON envelope (ADR 0006).
//! - `ProcessSandboxHandler` — bridge into `ActionRegistry`.
//! - `SandboxRunner`, `ActionExecutor`, `SandboxedContext` — runner abstraction.
//! - `capabilities::PluginCapabilities` — iOS-style capability declarations. Currently unenforced
//!   (discovery TODO — see README Appendix).
//! - `discovery` — scan directories for plugin binaries via `plugin.toml`.
//! - `os_sandbox` — OS-level hardening primitives (best-effort, partial).
//! - `SandboxError` — typed error.
//!
//! ## Canon
//!
//! - §4.5 operational honesty: capability allowlist is a false capability until the discovery
//!   wiring TODO is closed.
//! - §7.1 plugin packaging: this crate is the host side of the duplex broker; `nebula-plugin-sdk`
//!   is the plugin side.
//! - §12.6 isolation honesty: correctness boundary, not attacker-grade.
//!
//! See `crates/sandbox/README.md` for the real isolation roadmap and ADR 0006
//! status.

pub mod capabilities;
pub mod discovery;
pub mod error;
mod handler;
mod in_process;
pub mod os_sandbox;
mod process;
mod runner;

pub use error::SandboxError;
pub use handler::ProcessSandboxHandler;
pub use in_process::InProcessSandbox;
pub use process::ProcessSandbox;
pub use runner::{ActionExecutor, ActionExecutorFuture, SandboxRunner, SandboxedContext};
