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

    /// SYSTEM-SURFACE — delegates straight to `self.inner`, **ignoring the
    /// bound scope**.
    ///
    /// The scope is the *result* of this call (it comes out of the returned
    /// row), not an input: the caller supplies only a SHA-256 token hash and
    /// gets back the tenant identity alongside the activation record.  Binding
    /// the scope here would make it impossible to use the decorator for
    /// token-keyed resolution — the confused-deputy boundary is enforced by
    /// the fact that the caller must then construct a fresh
    /// `ScopedWebhookActivationStore::new(store, row.scope)` for any
    /// subsequent tenant-scoped operation, never by filtering the query.
    ///
    /// Mirrors the `reclaim_stuck` / `list_for_org` precedent on other stores.
    async fn resolve_by_token(
        &self,
        token_hash: &[u8; 32],
    ) -> Result<Option<WebhookActivationRecord>, StorageError> {
        self.inner.resolve_by_token(token_hash).await
    }

    /// SYSTEM-SURFACE — delegates straight to `self.inner`, **ignoring the
    /// bound scope**.
    ///
    /// Bootstrap map population requires the full cross-tenant set of active
    /// rows; filtering by the decorator's bound scope would return only one
    /// tenant's rows and leave the rest of the routing map dark.  Mirrors the
    /// `reclaim_stuck` / `list_for_org` precedent on other stores.
    async fn list_all_active(&self) -> Result<Vec<WebhookActivationRecord>, StorageError> {
        self.inner.list_all_active().await
    }
}
