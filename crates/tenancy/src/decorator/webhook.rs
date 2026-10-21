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
///
/// [`resolve_by_token`](WebhookActivationStore::resolve_by_token) is keyed
/// by a secret token hash and returns that one row with its scope.
/// [`list_all_active`](WebhookActivationStore::list_all_active) lists every
/// tenant and is refused on a tenant-bound handle.
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
    /// The token hash is the capability: it names one row and never lists.
    async fn resolve_by_token(
        &self,
        token_hash: &[u8; 32],
    ) -> Result<Option<WebhookActivationRecord>, StorageError> {
        self.inner.resolve_by_token(token_hash).await
    }

    /// Worker-global: refused on a tenant-bound handle.
    ///
    /// Bootstrap map population reads every tenant's active rows, so it runs
    /// on the raw store held by the composition root. A tenant-bound handle
    /// refuses it with [`StorageError::ScopeViolation`] without reaching
    /// storage, rather than listing other tenants' activations.
    async fn list_all_active(&self) -> Result<Vec<WebhookActivationRecord>, StorageError> {
        Err(StorageError::ScopeViolation {
            entity: "webhook_activation",
        })
    }
}
