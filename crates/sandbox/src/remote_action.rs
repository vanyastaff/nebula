//! `RemoteAction` — wraps `ProcessSandboxHandler` as `impl nebula_action::Action`
//! and `impl nebula_action::StatelessHandler`.
//!
//! Discovered out-of-process actions need to register alongside built-in
//! actions in the engine's `ActionRegistry`. This wrapper carries the
//! host-side `ActionMetadata` (built from the wire descriptor during
//! discovery) and delegates execution to the long-lived `ProcessSandbox`
//! handle via the existing `ProcessSandboxHandler`.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::{
    Action, ActionContext, ActionError, ActionMetadata, ActionResult, DeclaresDependencies,
    StatelessHandler,
};

use crate::handler::ProcessSandboxHandler;

/// Host-side `impl Action` wrapper for an out-of-process action.
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
}

impl DeclaresDependencies for RemoteAction {}

impl Action for RemoteAction {
    fn metadata(&self) -> &ActionMetadata {
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
        input: serde_json::Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        self.handler.execute(input, ctx).await
    }
}
