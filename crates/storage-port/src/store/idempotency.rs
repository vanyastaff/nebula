//! Per-attempt idempotency guard.

use crate::error::StorageError;
use crate::scope::Scope;

/// Per-attempt idempotency guard.
///
/// The key shape is unchanged — `{execution_id}:{node_id}:{attempt}` (attempt
/// index is `stored_attempts.len() + 1`). The decorator
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
