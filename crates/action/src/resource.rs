//! [`ResourceAction`] — graph-level dependency injection for actions.
//!
//! A resource action runs `configure` before the downstream subtree and
//! `cleanup` when the scope ends. The produced config/instance is visible
//! only to the downstream branch, unlike `ctx.resource()` from the global
//! registry.

use std::future::Future;

use crate::action::Action;
use crate::context::Context;
use crate::error::ActionError;

/// Resource action: graph-level dependency injection.
///
/// The engine runs `configure` before downstream nodes; the resulting config
/// (or instance) is scoped to the branch. When the scope ends, the engine
/// calls `cleanup`. Use for connection pools, caches, or other resources
/// visible only to the downstream subtree (unlike `ctx.resource()` from the
/// global registry).
pub trait ResourceAction: Action {
    /// Configuration or instance type produced by `configure` and passed to downstream.
    type Config: Send + Sync;
    /// Instance type to clean up (often the same as `Config`, e.g. a pool handle).
    type Instance: Send + Sync;

    /// Build the resource for this scope; engine runs this before downstream nodes.
    fn configure(
        &self,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<Self::Config, ActionError>> + Send;

    /// Clean up the instance when the scope ends (e.g. drop pool, close connections).
    fn cleanup(
        &self,
        instance: Self::Instance,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<(), ActionError>> + Send;
}
