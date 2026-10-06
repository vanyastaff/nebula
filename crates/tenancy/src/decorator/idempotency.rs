//! Scope-enforcing idempotency guard decorator (§6.1 replay-oracle).

use std::sync::Arc;

use nebula_storage_port::store::IdempotencyGuard;
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
