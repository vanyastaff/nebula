//! In-memory `IdempotencyStore` (ADR-0048 durable-replay cache) +
//! `WebhookActivationStore` (ADR-0049).
//!
//! Both share-nothing (their own `Arc<Mutex<…>>`): the cache is keyed by
//! `{workspace_id}:{org_id}:{cache_key}` (the store folds the scope in, so
//! tenant A can neither probe nor poison tenant B's dedup entry); webhook
//! activations are keyed by `(scope, slug)` so resolution never crosses a
//! tenant boundary.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use nebula_storage_port::dto::{CachedRecord, WebhookActivationRecord};
use nebula_storage_port::store::{IdempotencyStore, WebhookActivationStore};
use nebula_storage_port::{Scope, StorageError};
use parking_lot::Mutex;

/// In-memory durable idempotent-replay cache. First-writer-wins: a `put`
/// for an existing key is a no-op.
#[derive(Debug, Default, Clone)]
pub struct InMemoryIdempotencyStore {
    inner: Arc<Mutex<HashMap<String, CachedRecord>>>,
}

impl InMemoryIdempotencyStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Parse an RFC 3339 expiry into a comparable instant-ish epoch-ms value.
/// A malformed timestamp is treated as "already expired" (fail-closed:
/// never serve a record we cannot prove is still fresh).
fn expires_at_ms(rfc3339: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(i64::MIN)
}

/// Fold the scope into the cache key so two tenants' keyspaces are
/// disjoint: `{workspace_id}:{org_id}:{cache_key}`. A raw key from one
/// tenant can never collide with another's (§6.1 replay-oracle).
fn namespaced(scope: &Scope, cache_key: &str) -> String {
    format!("{}:{}:{}", scope.workspace_id, scope.org_id, cache_key)
}

#[async_trait::async_trait]
impl IdempotencyStore for InMemoryIdempotencyStore {
    async fn get(
        &self,
        scope: &Scope,
        cache_key: &str,
    ) -> Result<Option<CachedRecord>, StorageError> {
        let map = self.inner.lock();
        Ok(map.get(&namespaced(scope, cache_key)).cloned())
    }

    async fn put(
        &self,
        scope: &Scope,
        cache_key: String,
        record: CachedRecord,
        _ttl: Duration,
    ) -> Result<(), StorageError> {
        // First-writer-wins: only insert when absent (the existing record
        // — and its fingerprint — must survive a replay race).
        let mut map = self.inner.lock();
        map.entry(namespaced(scope, &cache_key)).or_insert(record);
        Ok(())
    }

    async fn evict_expired(&self) -> Result<u64, StorageError> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut map = self.inner.lock();
        let before = map.len();
        map.retain(|_, r| expires_at_ms(&r.expires_at) > now);
        Ok((before - map.len()) as u64)
    }
}

/// Activation map key: `(workspace_id, org_id, slug)`. A slug is unique
/// per tenant, so this composite key makes cross-tenant resolution
/// structurally impossible.
type ActivationKey = (String, String, String);

/// In-memory webhook-activation store. Keyed by [`ActivationKey`].
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
}
