//! Scope-enforcing idempotency decorators (§6.1 replay-oracle).

use std::sync::Arc;
use std::time::Duration;

use nebula_storage_port::dto::CachedRecord;
use nebula_storage_port::store::{IdempotencyGuard, IdempotencyStore};
use nebula_storage_port::{Scope, StorageError};

/// Wraps an [`IdempotencyGuard`] and forces `check_and_mark` into the
/// bound [`Scope`].
#[derive(Clone)]
pub struct ScopedIdempotencyGuard {
    inner: Arc<dyn IdempotencyGuard>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedIdempotencyGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedIdempotencyGuard")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedIdempotencyGuard {
    /// Bind `inner` to `scope`.
    #[must_use]
    pub fn new(inner: Arc<dyn IdempotencyGuard>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl IdempotencyGuard for ScopedIdempotencyGuard {
    async fn check_and_mark(
        &self,
        _scope: &Scope,
        execution_id: &str,
        node_id: &str,
        attempt: u32,
    ) -> Result<bool, StorageError> {
        self.inner
            .check_and_mark(&self.bound, execution_id, node_id, attempt)
            .await
    }
}

/// Wraps an [`IdempotencyStore`] and forces every call into the bound
/// [`Scope`]. The port trait now takes `&Scope` explicitly and the
/// backend folds it into the stored key, so this decorator simply
/// substitutes its bound scope (mirroring every other `Scoped*`
/// decorator and `ScopedIdempotencyGuard`); a raw key from one tenant
/// can never collide with another tenant's because the scope differs.
#[derive(Clone)]
pub struct ScopedIdempotencyStore {
    inner: Arc<dyn IdempotencyStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedIdempotencyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedIdempotencyStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedIdempotencyStore {
    /// Bind `inner` to `scope`.
    #[must_use]
    pub fn new(inner: Arc<dyn IdempotencyStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl IdempotencyStore for ScopedIdempotencyStore {
    async fn get(
        &self,
        _scope: &Scope,
        cache_key: &str,
    ) -> Result<Option<CachedRecord>, StorageError> {
        self.inner.get(&self.bound, cache_key).await
    }

    async fn put(
        &self,
        _scope: &Scope,
        cache_key: String,
        record: CachedRecord,
        ttl: Duration,
    ) -> Result<(), StorageError> {
        self.inner.put(&self.bound, cache_key, record, ttl).await
    }

    async fn evict_expired(&self) -> Result<u64, StorageError> {
        self.inner.evict_expired().await
    }
}
