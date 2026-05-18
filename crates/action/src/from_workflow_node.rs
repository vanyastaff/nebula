//! `FromWorkflowNode` — async factory trait that resolves slot bindings
//! against a workflow node definition and an action context.
//!
//! Every concrete action implementing
//! [`Action`](crate::Action) ALSO implements [`FromWorkflowNode`]. The engine
//! calls [`from_workflow_node`](FromWorkflowNode::from_workflow_node) once at
//! dispatch time:
//!
//! 1. Read each declared slot field from `Self::dependencies()`.
//! 2. For each slot, look up the override in `node.slot_bindings` ( hybrid binding) —
//! falling back to the action's declared `default_id`.
//! 3. Resolve the resource / credential through [`ActionContext`](crate::ActionContext) typed
//! helpers ([`acquire_resource_by_id`](crate::context::ActionContextExt::acquire_resource_by_id),
//! [`resolve_credential_by_id`](crate::context::ActionContextExt::resolve_credential_by_id)).
//! 4. Assemble `Self` with the resolved guards.
//!
//! `#[derive(Action)]` (Phase 3 / Session 3) generates the body of
//! [`from_workflow_node`](FromWorkflowNode::from_workflow_node) automatically
//! so plugin authors never write it by hand.

use std::future::Future;

use nebula_workflow::NodeDefinition;

use crate::context::ActionContext;

/// Async factory trait — builds a typed action instance from a workflow
/// node definition + action context.
///
/// `Self::Error` is whatever the factory wants to surface; in practice the
/// derive-generated impl uses [`ActionError`](crate::ActionError) so failed
/// slot resolution flows through the existing retry/fatal classification.
///
/// # Object safety
///
/// **Not object-safe** — the trait carries `Sized` via the implementor
/// bound and an associated `Error` type. Engine dispatch calls a thin
/// non-generic wrapper ([`ActionFactory`](crate::ActionFactory) — Session 4)
/// that erases this typed surface to a `Box<dyn ErasedAction>`.
///
/// # Example (derive expansion sketch)
///
/// ```rust,ignore
/// # use nebula_action::{Action, FromWorkflowNode, ActionContext, ActionError};
/// # use nebula_workflow::NodeDefinition;
/// # use std::future::Future;
/// # struct SendTelegram { /* slot fields */ }
/// impl FromWorkflowNode for SendTelegram {
/// type Error = ActionError;
/// fn from_workflow_node<'a>(
/// node: &'a NodeDefinition,
/// ctx: &'a dyn ActionContext,
/// ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a {
/// async move {
/// // resolve slot fields against ctx + node.slot_bindings here
/// # unimplemented!()
/// }
/// }
/// }
/// ```
pub trait FromWorkflowNode: Sized + Send + 'static {
    /// Error returned on factory failure (typically [`ActionError`](crate::ActionError)).
    type Error: Send;

    /// Build a `Self` instance for the given node + context.
    ///
    /// Invoked once per dispatch by the engine (after registry lookup,
    /// before handler execution). Slot resolution costs (e.g., remote
    /// credential lookup) MUST be acceptable as a per-dispatch cost —
    /// the engine does not memoise the result.
    fn from_workflow_node<'a>(
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a;
}
