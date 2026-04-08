//! Sandbox interface — re-exported from [`nebula_sandbox`].
//!
//! This module re-exports sandbox types for backward compatibility.
//! New code should depend on `nebula-sandbox` directly.

pub use nebula_sandbox::{
    ActionExecutor, ActionExecutorFuture, InProcessSandbox, SandboxRunner, SandboxedContext,
};
