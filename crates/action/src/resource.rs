//! [`ResourceAction`] — graph-level dependency injection for actions.
//!
//! A resource action runs `configure` before the downstream subtree and
//! `cleanup` when the scope ends. The produced resource is visible only
//! to the downstream branch, unlike `ctx.resource()` from the global
//! registry.

use std::future::Future;

use crate::action::Action;
use crate::context::Context;
use crate::error::ActionError;

/// Resource action: graph-level dependency injection.
///
/// The engine runs `configure` before downstream nodes; the resulting
/// resource is scoped to the branch. When the scope ends, the engine
/// calls `cleanup` with the same resource. Use for connection pools,
/// caches, or other resources visible only to the downstream subtree
/// (unlike `ctx.resource()` from the global registry).
///
/// A single associated type `Resource` is used for both the `configure`
/// return and the `cleanup` parameter. Earlier iterations split these
/// into `Config` (returned) and `Instance` (consumed by cleanup), but
/// the adapter could not safely bridge the two: it boxed `Config` and
/// downcast to `Instance`, so any impl where they differed failed at
/// runtime with `ActionError::Fatal`. Zero production impls ever used
/// distinct types, so the split was removed rather than patched.
pub trait ResourceAction: Action {
    /// Resource produced by `configure`, consumed by `cleanup`.
    type Resource: Send + Sync + 'static;

    /// Build the resource for this scope; engine runs this before downstream nodes.
    fn configure(
        &self,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<Self::Resource, ActionError>> + Send;

    /// Clean up the resource when the scope ends (e.g. drop pool, close connections).
    fn cleanup(
        &self,
        resource: Self::Resource,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<(), ActionError>> + Send;
}
