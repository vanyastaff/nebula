//! Idempotency traits (spec-16 §11.3 + ADR-0048 hybrid backend).
use std::time::Duration;

use crate::dto::CachedRecord;
use crate::error::StorageError;
use crate::scope::Scope;

/// Per-attempt idempotency guard (spec-16 §11.3).
///
/// The key shape is unchanged — `{execution_id}:{node_id}:{attempt}` (the
/// ADR-0042 `attempts.len()+1` derivation is preserved). The decorator
/// namespaces it by tenant so tenant A cannot probe or poison tenant B's
/// dedup entry (replay-oracle mitigation, §6.1).
#[async_trait::async_trait]
pub trait IdempotencyGuard: Send + Sync + std::fmt::Debug {
    /// Atomically check whether `{execution_id}:{node_id}:{attempt}` is
    /// already marked, marking it if not. Returns `true` if this caller is
    /// the first to mark it (i.e. the work should proceed), `false` if it
    /// was already marked (skip — already done).
    async fn check_and_mark(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
        attempt: u32,
    ) -> Result<bool, StorageError>;
}

/// Durable idempotent-replay response store (ADR-0048 hybrid backend).
///
/// First-writer-wins: `put` for an existing key is a no-op (the original
/// record stays). `scope` is an explicit parameter (consistent with
/// [`IdempotencyGuard::check_and_mark`] and the rest of the port) — the
/// backend folds it into the stored key so tenant A can neither probe
/// nor poison tenant B's dedup entry (§6.1 replay-oracle), rather than
/// relying on the caller to hand in a pre-namespaced string. A
/// [`StorageError`] from `get` must NOT be treated as a cache miss —
/// silently dropping replay protection on corruption is rejected by
/// ADR-0048.
#[async_trait::async_trait]
pub trait IdempotencyStore: Send + Sync + std::fmt::Debug {
    /// Look up a cached record by `cache_key` within `scope`.
    async fn get(
        &self,
        scope: &Scope,
        cache_key: &str,
    ) -> Result<Option<CachedRecord>, StorageError>;

    /// Persist a record under `cache_key` within `scope` with `ttl`
    /// (first-writer-wins).
    async fn put(
        &self,
        scope: &Scope,
        cache_key: String,
        record: CachedRecord,
        ttl: Duration,
    ) -> Result<(), StorageError>;

    /// Drop expired rows; returns the number reclaimed (in-memory backends
    /// may report `0`).
    async fn evict_expired(&self) -> Result<u64, StorageError>;
}
