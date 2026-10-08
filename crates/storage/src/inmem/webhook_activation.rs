//! In-memory `WebhookActivationStore`: activations keyed by `(scope, slug)`
//! so resolution never crosses a tenant boundary.

use std::collections::HashMap;
use std::sync::Arc;

use nebula_storage_port::dto::WebhookActivationRecord;
use nebula_storage_port::store::WebhookActivationStore;
use nebula_storage_port::{Scope, StorageError};
use parking_lot::Mutex;

/// Activation map key: `(workspace_id, org_id, slug)`. A slug is unique
/// per tenant, so this composite key makes cross-tenant resolution
/// structurally impossible.
type ActivationKey = (String, String, String);

/// In-memory webhook-activation store keyed by the private activation-key tuple.
#[derive(Debug, Default, Clone)]
pub struct InMemoryWebhookActivationStore {
    inner: Arc<Mutex<HashMap<ActivationKey, WebhookActivationRecord>>>,
}

impl InMemoryWebhookActivationStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl WebhookActivationStore for InMemoryWebhookActivationStore {
    async fn upsert(
        &self,
        scope: &Scope,
        record: WebhookActivationRecord,
    ) -> Result<(), StorageError> {
        let key = (
            scope.workspace_id.clone(),
            scope.org_id.clone(),
            record.slug.clone(),
        );
        self.inner.lock().insert(key, record);
        Ok(())
    }

    async fn resolve(
        &self,
        scope: &Scope,
        slug: &str,
    ) -> Result<Option<WebhookActivationRecord>, StorageError> {
        let key = (
            scope.workspace_id.clone(),
            scope.org_id.clone(),
            slug.to_string(),
        );
        let map = self.inner.lock();
        // Only an active activation resolves; a deactivated row is a miss
        // (never route a paused webhook).
        Ok(map.get(&key).filter(|r| r.active).cloned())
    }

    async fn deactivate(&self, scope: &Scope, trigger_id: &str) -> Result<(), StorageError> {
        let mut map = self.inner.lock();
        for ((ws, org, _), rec) in &mut *map {
            if ws == &scope.workspace_id && org == &scope.org_id && rec.trigger_id == trigger_id {
                rec.active = false;
            }
        }
        Ok(())
    }

    /// SYSTEM-SURFACE: scope comes out of the returned row, not in.
    /// Rejects the all-zeros sentinel before scanning (see trait doc).
    async fn resolve_by_token(
        &self,
        token_hash: &[u8; 32],
    ) -> Result<Option<WebhookActivationRecord>, StorageError> {
        // Sentinel guard: all-zeros means "no token assigned"; never match.
        if token_hash == &[0u8; 32] {
            return Ok(None);
        }
        let map = self.inner.lock();
        // Only active rows are reachable by token; a deactivated row must
        // never be returned (mirrors the SQL `AND active = 1/TRUE` predicate).
        Ok(map
            .values()
            .find(|r| r.active && &r.token_hash == token_hash)
            .cloned())
    }

    /// SYSTEM-SURFACE: cross-tenant enumeration for bootstrap map population.
    async fn list_all_active(&self) -> Result<Vec<WebhookActivationRecord>, StorageError> {
        let map = self.inner.lock();
        Ok(map.values().filter(|r| r.active).cloned().collect())
    }
}
