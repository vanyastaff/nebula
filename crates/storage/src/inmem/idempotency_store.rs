//! In-memory `IdempotencyStore` (durable idempotent-replay cache) +
//! `WebhookActivationStore` (webhook activation specs).
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

#[cfg(test)]
mod namespaced_collision_tests {
    //! FIX 2 regression: two (workspace_id, org_id, cache_key) tuples that
    //! produce the same raw-`:` string must produce DISTINCT keys after the
    //! length-prefix fix. The classic confused-deputy collision is:
    //!   ("a:b", "c", "key")  vs  ("a", "b:c", "key")
    //! which both serialised to "a:b:c:key" under the old scheme.

    use nebula_storage_port::Scope;

    use super::namespaced;

    #[test]
    fn distinct_scope_tuples_that_collided_with_raw_colon_now_produce_distinct_keys() {
        let scope_ab_c = Scope::new("a:b", "c");
        let scope_a_bc = Scope::new("a", "b:c");
        let cache_key = "my-cache-key";

        let key1 = namespaced(&scope_ab_c, cache_key);
        let key2 = namespaced(&scope_a_bc, cache_key);

        assert_ne!(
            key1, key2,
            "scopes ('a:b','c') and ('a','b:c') must produce distinct namespaced keys \
             (previously both mapped to 'a:b:c:my-cache-key' under the raw-colon scheme)"
        );
    }

    #[test]
    fn equal_scopes_produce_equal_keys() {
        let scope1 = Scope::new("ws-123", "org-456");
        let scope2 = Scope::new("ws-123", "org-456");
        assert_eq!(namespaced(&scope1, "k"), namespaced(&scope2, "k"));
    }

    #[test]
    fn different_cache_keys_on_same_scope_produce_distinct_keys() {
        let scope = Scope::new("ws", "org");
        assert_ne!(namespaced(&scope, "key-a"), namespaced(&scope, "key-b"));
    }

    #[test]
    fn workspace_id_containing_record_separator_does_not_collide() {
        // Even if a workspace_id contains the RS sentinel (\x1e), distinctness
        // must hold because the length pin still prevents boundary ambiguity.
        let scope_with_rs = Scope::new("ws\x1e", "org");
        let scope_plain = Scope::new("ws", "\x1eorg");
        assert_ne!(
            namespaced(&scope_with_rs, "k"),
            namespaced(&scope_plain, "k"),
            "a workspace_id containing the RS sentinel must not collide with a plain scope"
        );
    }
}

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
/// disjoint. Uses the same length-prefix scheme as [`Scope::credential_owner_id`]:
/// `{ws_len}\x1e{workspace_id}\x1e{org_len}\x1e{org_id}\x1e{cache_key}`.
///
/// A raw `:` or `/` delimiter is **not safe** because `workspace_id` and
/// `org_id` are unvalidated free strings (see `scope.rs` doc), so a component
/// containing the delimiter could forge a different namespace (confused-deputy
/// cross-tenant collision). Length-prefixing makes the boundary unforgeable:
/// the leading `{ws_len}` pins exactly where `workspace_id` ends regardless of
/// its content, and the same applies to `org_id`. The ASCII Record Separator
/// (`\x1e`) is a readable sentinel between the length and the value; the full
/// key is an opaque internal lookup value, never wire-exposed.
fn namespaced(scope: &Scope, cache_key: &str) -> String {
    format!(
        "{}\x1e{}\x1e{}\x1e{}\x1e{}",
        scope.workspace_id.len(),
        scope.workspace_id,
        scope.org_id.len(),
        scope.org_id,
        cache_key,
    )
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
