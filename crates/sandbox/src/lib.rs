#![deny(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Sandbox
//!
//! Plugin isolation and sandboxing for the Nebula workflow engine.
//!
//! - [`InProcessSandbox`] — trusted in-process execution (built-in actions)
//! - [`ProcessSandbox`] — isolated child process execution (community plugins)
//! - [`ProcessSandboxHandler`] — bridges ProcessSandbox into ActionRegistry
//! - [`capabilities`] — iOS-style per-plugin capability model
//! - [`discovery`] — scan directories for plugin binaries

pub mod capabilities;
pub mod discovery;
mod handler;
mod in_process;
pub mod os_sandbox;
mod process;
mod runner;

pub use handler::ProcessSandboxHandler;
pub use in_process::InProcessSandbox;
pub use process::ProcessSandbox;
pub use runner::{ActionExecutor, ActionExecutorFuture, SandboxRunner, SandboxedContext};
