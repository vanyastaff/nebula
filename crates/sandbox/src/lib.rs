#![deny(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Sandbox
//!
//! Plugin isolation and sandboxing for the Nebula workflow engine.
//!
//! - [`InProcessSandbox`] — trusted in-process execution (built-in actions)
//! - [`ProcessSandbox`] — isolated child process execution (community plugins)
//! - [`PluginPermissions`](permissions::PluginPermissions) — per-plugin access control

pub mod capabilities;
mod in_process;
mod process;
mod runner;

pub use in_process::InProcessSandbox;
pub use process::ProcessSandbox;
pub use runner::{ActionExecutor, ActionExecutorFuture, SandboxRunner, SandboxedContext};
