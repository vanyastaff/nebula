//! Scope-enforcing [`WebhookActivationStore`] decorator.

use std::sync::Arc;

use nebula_storage_port::dto::WebhookActivationRecord;
use nebula_storage_port::store::WebhookActivationStore;
use nebula_storage_port::{Scope, StorageError};

/// Wraps a [`WebhookActivationStore`] and forces every call into the
/// bound [`Scope`].
///
/// `resolve` for a slug registered by another tenant comes back `None`:
/// an inbound `POST /hooks/{slug}` dispatches only within the request's
/// tenant, never fanning out to a colliding slug in a different tenant
/// (§6.1 confused-deputy).
#[derive(Clone)]
pub struct ScopedWebhookActivationStore {
    inner: Arc<dyn WebhookActivationStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedWebhookActivationStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedWebhookActivationStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedWebhookActivationStore {
    /// Bind `inner` to `scope`.
    #[must_use]
    pub fn new(inner: Arc<dyn WebhookActivationStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl WebhookActivationStore for ScopedWebhookActivationStore {
    async fn upsert(
        &self,
        _scope: &Scope,
        record: WebhookActivationRecord,
    ) -> Result<(), StorageError> {
        self.inner.upsert(&self.bound, record).await
    }

    async fn resolve(
        &self,
        _scope: &Scope,
        slug: &str,
    ) -> Result<Option<WebhookActivationRecord>, StorageError> {
        self.inner.resolve(&self.bound, slug).await
    }

    async fn deactivate(&self, _scope: &Scope, trigger_id: &str) -> Result<(), StorageError> {
        self.inner.deactivate(&self.bound, trigger_id).await
    }
}
