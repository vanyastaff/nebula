//! In-memory `ResumeTokenStore` implementation (W-S3c).
//!
//! Shares the same [`super::execution::SharedState`] mutex used by
//! [`super::execution::InMemoryExecutionStore`].  This ensures `consume`
//! and `revoke_on_terminal` observe the same token rows that `commit`
//! inserted atomically — there is no window where a committed token is
//! invisible to the read path.
//!
//! `consume` uses `subtle::ConstantTimeEq` for defence-in-depth.  This
//! is an in-process, single-binary store so network-level timing attacks
//! are not in scope; the constant-time comparison is best-effort
//! protection against any future refactor that introduces an early-exit
//! path, and costs nothing here.

use nebula_storage_port::Scope;
use nebula_storage_port::StorageError;
use nebula_storage_port::dto::resume_token::{ResumeTokenRow, TokenHash};
use nebula_storage_port::store::ResumeTokenStore;
use subtle::ConstantTimeEq;

use super::execution::{SharedState, State};

/// In-memory resume-token store.
///
/// Shares the `SharedState` mutex with `InMemoryExecutionStore` so
/// `commit` + `consume` are logically atomic (same in-memory state,
/// same lock order: always locked through `SharedState`).
///
/// Wrap with `Arc` at the composition root.
#[derive(Debug, Clone)]
pub struct InMemoryResumeTokenStore {
    inner: SharedState,
}

impl InMemoryResumeTokenStore {
    /// Construct a store backed by the given shared state.
    ///
    /// Pass the same `SharedState` used by the `InMemoryExecutionStore`
    /// so token inserts (from `commit`) and reads (from `consume`) share
    /// the same mutex-guarded map.
    #[must_use]
    pub(super) fn new(inner: SharedState) -> Self {
        Self { inner }
    }

    /// Construct a standalone store with its own independent shared state.
    ///
    /// Use at composition roots that hold an erased `Arc<dyn ExecutionStore>`
    /// and cannot call `InMemoryExecutionStore::resume_token_store()`. The
    /// token map is always empty because `commit` on the separate execution
    /// store will never insert into this state. Suitable for test harnesses
    /// that do not exercise the signal-park mint path.
    #[must_use]
    pub fn standalone() -> Self {
        use parking_lot::Mutex;
        use std::sync::Arc;
        Self {
            inner: Arc::new(Mutex::new(State::default())),
        }
    }
}

#[async_trait::async_trait]
impl ResumeTokenStore for InMemoryResumeTokenStore {
    async fn consume(
        &self,
        token_hash: &TokenHash,
    ) -> Result<Option<ResumeTokenRow>, StorageError> {
        let mut guard = self.inner.lock();
        let target = token_hash.as_bytes();
        // Constant-time equality scan for defence-in-depth; the map remove
        // by key is the correctness primitive.
        // Best-effort constant-time equality (defence-in-depth; see module doc).
        // The map remove by key is the correctness primitive.
        let matching_key: Option<Vec<u8>> = guard
            .resume_tokens
            .keys()
            .find(|k| bool::from(k.as_slice().ct_eq(target)))
            .cloned();
        Ok(matching_key.and_then(|key| guard.resume_tokens.remove(&key)))
    }

    async fn revoke_on_terminal(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<u64, StorageError> {
        let mut guard = self.inner.lock();
        let before = guard.resume_tokens.len();
        guard
            .resume_tokens
            .retain(|_, row| !(row.execution_id == execution_id && row.scope == *scope));
        Ok((before - guard.resume_tokens.len()) as u64)
    }
}
