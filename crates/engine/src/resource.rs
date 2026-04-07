//! Bridge between the resource [`Manager`] and the action execution layer.
//!
//! [`ManagedResourceAccessor`] implements [`ResourceAccessor`] from
//! `nebula-action` by delegating to [`Manager::acquire_erased`], providing
//! the glue between the resource subsystem and action contexts.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::capability::ResourceAccessor;
use nebula_action::ActionError;
use nebula_core::id::ExecutionId;
use nebula_core::ResourceKey;
use nebula_resource::ctx::BasicCtx;
use nebula_resource::options::AcquireOptions;
use nebula_resource::Manager;
use tokio_util::sync::CancellationToken;

/// [`ResourceAccessor`] backed by the resource [`Manager`].
///
/// Constructed per-execution with execution-scoped context (execution ID,
/// cancellation token). Delegates `acquire` calls to
/// [`Manager::acquire_erased`] which handles credential resolution, auth
/// injection, and topology-specific acquire dispatch.
pub(crate) struct ManagedResourceAccessor {
    manager: Arc<Manager>,
    execution_id: ExecutionId,
    cancel: CancellationToken,
}

impl ManagedResourceAccessor {
    /// Creates a new accessor for a single execution.
    pub(crate) fn new(
        manager: Arc<Manager>,
        execution_id: ExecutionId,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            manager,
            execution_id,
            cancel,
        }
    }
}

#[async_trait]
impl ResourceAccessor for ManagedResourceAccessor {
    async fn acquire(&self, key: &str) -> Result<Box<dyn Any + Send>, ActionError> {
        let resource_key = ResourceKey::new(key).map_err(|e| {
            ActionError::fatal(format!("invalid resource key `{key}`: {e}"))
        })?;

        let ctx = BasicCtx::new(self.execution_id)
            .with_cancel_token(self.cancel.clone());

        self.manager
            .acquire_erased(&resource_key, ctx, &AcquireOptions::default())
            .await
            .map_err(|e| ActionError::fatal(format!("resource `{key}`: {e}")))
    }

    async fn exists(&self, key: &str) -> bool {
        let Ok(resource_key) = ResourceKey::new(key) else {
            return false;
        };
        self.manager.contains(&resource_key)
    }
}
