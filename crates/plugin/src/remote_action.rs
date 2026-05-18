//! `RemoteAction` — wraps `ProcessSandboxHandler` as `impl nebula_action::StatelessHandler`.
//!
//! Discovered out-of-process actions need to register alongside built-in
//! actions in the engine's `ActionRegistry`. This wrapper carries the
//! host-side `ActionMetadata` (built from the wire descriptor during
//! discovery) and delegates execution to the long-lived `ProcessSandbox`
//! handle via the existing `ProcessSandboxHandler`.
//!
//! The typed [`nebula_action::Action`] trait is
//! `Sized`/object-unsafe and requires static metadata — incompatible with
//! the dynamic per-instance metadata `RemoteAction` carries. So the wrapper
//! only provides the dyn-erased [`StatelessHandler`] surface; engine
//! registration happens through a `RemoteActionFactory` adapter (post-S4)
//! that produces an [`nebula_action::ErasedAction::Stateless`] from this
//! handler at dispatch time.

use std::{future::Future, pin::Pin, sync::Arc};

use async_trait::async_trait;
use nebula_action::{
    ActionContext, ActionError, ActionFactory, ActionMetadata, ActionResult, ErasedAction,
    ErasedStateless, StatelessHandler,
};
use nebula_workflow::NodeDefinition;
use serde_json::Value;

use crate::handler::ProcessSandboxHandler;

/// Host-side `dyn StatelessHandler` wrapper for an out-of-process action.
///
/// Created by discovery when building actions from an out-of-process plugin's
/// wire `ActionDescriptor` list. Holds the resolved `ActionMetadata`
/// (including the schema round-tripped from the wire) and an `Arc` to the
/// shared `ProcessSandboxHandler` that dispatches invocations through the
/// plugin's long-lived process.
pub struct RemoteAction {
    metadata: ActionMetadata,
    handler: Arc<ProcessSandboxHandler>,
}

impl std::fmt::Debug for RemoteAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteAction")
            .field("key", &self.metadata.base.key)
            .finish_non_exhaustive()
    }
}

impl RemoteAction {
    /// Create a new `RemoteAction`.
    pub fn new(metadata: ActionMetadata, handler: Arc<ProcessSandboxHandler>) -> Self {
        Self { metadata, handler }
    }

    /// The underlying sandbox handler.
    pub fn handler(&self) -> &Arc<ProcessSandboxHandler> {
        &self.handler
    }

    /// Access the host-side metadata.
    pub fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

#[async_trait]
impl StatelessHandler for RemoteAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }

    async fn execute(
        &self,
        input: Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        self.handler.execute(input, ctx).await
    }
}

/// `ActionFactory` adapter for an out-of-process [`RemoteAction`].
///
/// The host registry stores `Arc<dyn ActionFactory>` ;
/// at dispatch time the engine calls
/// [`instantiate`](ActionFactory::instantiate) and gets an
/// [`ErasedAction::Stateless`] wrapping the underlying `RemoteAction`'s
/// long-lived `ProcessSandboxHandler`.
pub struct RemoteActionFactory {
    inner: Arc<RemoteAction>,
}

impl RemoteActionFactory {
    /// Wrap a shared [`RemoteAction`] as a factory.
    pub fn new(inner: Arc<RemoteAction>) -> Self {
        Self { inner }
    }
}

impl std::fmt::Debug for RemoteActionFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteActionFactory")
            .field("key", &self.inner.metadata().base.key)
            .finish_non_exhaustive()
    }
}

impl ActionFactory for RemoteActionFactory {
    fn metadata(&self) -> &ActionMetadata {
        self.inner.metadata()
    }

    fn instantiate<'a>(
        &'a self,
        _node: &'a NodeDefinition,
        _ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ErasedAction, ActionError>> + Send + 'a>> {
        // Out-of-process actions do not carry slot fields (their inputs
        // travel as JSON across the protocol boundary). The factory just
        // wraps the long-lived RemoteAction in an ErasedStateless.
        let stateless: Box<dyn ErasedStateless> = Box::new(RemoteErasedStateless {
            inner: Arc::clone(&self.inner),
        });
        Box::pin(async move { Ok(ErasedAction::Stateless(stateless)) })
    }
}

/// Internal `ErasedStateless` wrapper bridging a `RemoteAction` (which
/// already implements [`StatelessHandler`]) to the `dyn ErasedStateless`
/// engine surface.
struct RemoteErasedStateless {
    inner: Arc<RemoteAction>,
}

#[async_trait]
impl ErasedStateless for RemoteErasedStateless {
    fn metadata(&self) -> &ActionMetadata {
        self.inner.metadata()
    }

    async fn dispatch(
        &self,
        input: Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        StatelessHandler::execute(&*self.inner, input, ctx).await
    }
}
