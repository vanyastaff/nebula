//! Resource provider for the engine.
//!
//! Bridges [`nebula_resource::Manager`] to the [`ResourceProvider`] port trait
//! so actions can acquire resources via `ActionContext::resource()`.

use std::sync::Arc;

use nebula_action::ActionError;
use nebula_action::provider::ResourceProvider;
use nebula_resource::{Context, Manager, ResourceHandle, Scope};
use tokio_util::sync::CancellationToken;

/// Engine-scoped resource provider.
///
/// Created per-execution with workflow/execution context and cancellation.
/// Implements [`ResourceProvider`] so it can be injected into [`ActionContext`].
pub(crate) struct Resources {
    manager: Arc<Manager>,
    scope: Scope,
    workflow_id: String,
    execution_id: String,
    cancellation: CancellationToken,
    /// Optional credential provider passed through to the resource Context.
    /// Gated behind the `credentials` feature.
    #[cfg(feature = "credentials")]
    credential_provider: Option<Arc<dyn nebula_resource::credentials::CredentialProvider>>,
}

impl Resources {
    pub(crate) fn new(
        manager: Arc<Manager>,
        workflow_id: impl Into<String>,
        execution_id: impl Into<String>,
        cancellation: CancellationToken,
    ) -> Self {
        let workflow_id = workflow_id.into();
        let execution_id = execution_id.into();
        // Build execution-level scope from available IDs.
        let scope = Scope::execution_in_workflow(&execution_id, &workflow_id, None);
        Self {
            manager,
            scope,
            workflow_id,
            execution_id,
            cancellation,
            #[cfg(feature = "credentials")]
            credential_provider: None,
        }
    }

    /// Attach a credential provider that will be passed through to the
    /// resource [`Context`] on every acquire call.
    #[cfg(feature = "credentials")]
    #[allow(dead_code)]
    pub(crate) fn with_credentials(
        mut self,
        provider: Arc<dyn nebula_resource::credentials::CredentialProvider>,
    ) -> Self {
        self.credential_provider = Some(provider);
        self
    }
}

#[async_trait::async_trait]
impl ResourceProvider for Resources {
    async fn acquire(&self, key: &str) -> Result<Box<dyn std::any::Any + Send>, ActionError> {
        let ctx = Context::new(self.scope.clone(), &self.workflow_id, &self.execution_id)
            .with_cancellation(self.cancellation.clone());

        #[cfg(feature = "credentials")]
        let ctx = match self.credential_provider {
            Some(ref provider) => ctx.with_credentials(Arc::clone(provider)),
            None => ctx,
        };

        let guard = tokio::select! {
            result = self.manager.acquire(key, &ctx) => {
                result.map_err(|e| {
                    if e.is_retryable() {
                        ActionError::retryable(format!("resource acquire failed: {e}"))
                    } else {
                        ActionError::fatal(format!("resource acquire failed: {e}"))
                    }
                })?
            }
            () = self.cancellation.cancelled() => {
                // Preserve cancellation semantics for upstream control flow/reporting.
                return Err(ActionError::Cancelled);
            }
        };

        Ok(Box::new(ResourceHandle::new(guard)))
    }
}
