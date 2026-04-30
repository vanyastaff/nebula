//! `ActionFactory` — engine-side object-safe per-execution factory.
//!
//! Per ADR-0043 §6 / Phase 3 Session 4. The engine's
//! `ActionRegistry` keeps `Arc<dyn ActionFactory>` per `ActionKey`. On
//! each dispatch, the registry calls
//! [`instantiate`](ActionFactory::instantiate) with the current
//! [`NodeDefinition`](nebula_workflow::NodeDefinition) + an
//! [`ActionContext`](crate::ActionContext); the factory builds a fresh
//! [`ErasedAction`](crate::ErasedAction) ready for dispatch.
//!
//! The default `GenericActionFactory<A>` (planned for later in this
//! phase) wraps any `A: Action + FromWorkflowNode` into an
//! [`ActionFactory`] by routing through
//! [`FromWorkflowNode::from_workflow_node`](crate::FromWorkflowNode::from_workflow_node)
//! and then erasing to [`ErasedAction`](crate::ErasedAction).

use std::{future::Future, pin::Pin};

use nebula_workflow::NodeDefinition;

use crate::{
    context::ActionContext, erased::ErasedAction, error::ActionError, metadata::ActionMetadata,
};

/// Object-safe factory trait — engine registry stores `Arc<dyn ActionFactory>`.
///
/// `instantiate` returns a `Pin<Box<dyn Future<...>>>` so the trait remains
/// object-safe (vs `impl Future` which is not). The lifetime borrows
/// `node` and `ctx` for the duration of the future — typical engine
/// dispatch awaits the future to completion before moving on.
///
/// # Errors
///
/// Returns [`ActionError::Fatal`] if slot resolution fails or the factory
/// otherwise cannot construct an executable action for this dispatch.
pub trait ActionFactory: Send + Sync + 'static {
    /// Static metadata describing the action this factory produces.
    fn metadata(&self) -> &ActionMetadata;

    /// Build an [`ErasedAction`] for the given workflow node + context.
    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ErasedAction, ActionError>> + Send + 'a>>;
}
