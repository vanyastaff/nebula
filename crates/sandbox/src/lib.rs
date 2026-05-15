#![deny(unsafe_code)]
#![warn(missing_docs)]

//! # nebula-sandbox — Process Sandboxing (Correctness Boundary)
//!
//! Two execution modes plus the plugin discovery path.
//!
//! **This is not a security boundary against malicious native code.**
//! Canon §12.6 is the normative statement: in-process execution provides
//! correctness and cooperative cancellation; child-process execution provides
//! OS-namespace separation via a duplex JSON envelope over UDS / Named Pipe
//! (ADR 0006). WASM / WASI is an explicit non-goal (§12.6). There is **no**
//! per-plugin capability/scope model here — egress, credential, and
//! filesystem mediation is the broker's responsibility (ADR-0025), not this
//! crate.
//!
//! ## Key types
//!
//! - `ProcessSandbox` — child-process execution via JSON envelope (ADR 0006).
//!   Transport methods return `SandboxError`; the `SandboxError` →
//!   `ActionError` mapping and the `SandboxRunner` runner abstraction live
//!   in `nebula-engine` (the dispatcher that owns them).
//! - `ProcessSandboxHandler` — bridge into `ActionRegistry`.
//! - `discovery` — scan directories for plugin binaries via `plugin.toml`.
//! - `os_sandbox` — Linux Landlock + rlimit child hardening (fixed system
//!   paths; no per-plugin grant).
//! - `SandboxError` — typed error.
//!
//! ## Canon
//!
//! - §7.1 plugin packaging: this crate is the host side of the duplex broker; `nebula-plugin-sdk`
//!   is the plugin side.
//! - §12.6 isolation honesty: correctness boundary, not attacker-grade; no
//!   false-capability surface is advertised.
//!
//! See `crates/sandbox/README.md` for the real isolation roadmap and ADR 0006
//! status.

pub mod discovered_plugin;
pub mod discovery;
pub mod error;
mod handler;
pub mod os_sandbox;
pub mod plugin_toml;
mod remote_action;

mod codec;
mod dispatch;
mod handshake;
mod spawn;

pub use discovered_plugin::DiscoveredPlugin;
pub use dispatch::ProcessSandbox;
pub use error::SandboxError;
pub use handler::ProcessSandboxHandler;
pub use remote_action::{RemoteAction, RemoteActionFactory};
