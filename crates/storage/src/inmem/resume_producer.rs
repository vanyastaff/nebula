//! In-memory [`ResumeProducer`] implementation (ADR-0099 W-S3d, Option B1).
//!
//! Shares the single [`super::execution::SharedState`] mutex that holds BOTH
//! the resume-token map and the control queue, so `consume_and_enqueue_resume`
//! removes the token and inserts the `Resume` row under ONE lock — the
//! in-memory analogue of the SQL backends' single transaction. There is no
//! window where a token is burned but the `Resume` is missing.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::resume_token::{ResumeTokenRow, TokenHash};
use nebula_storage_port::dto::{ControlCommand, ControlMsg};
use nebula_storage_port::store::ResumeProducer;
use subtle::ConstantTimeEq;

use super::execution::{QueuedMsg, SharedState};

/// In-memory resume producer.
///
/// Wraps the same `SharedState` as [`super::InMemoryExecutionStore`] and
/// [`super::InMemoryControlQueue`], so the token DELETE and control-queue
/// INSERT happen under one lock (single-lock atomicity invariant).
///
/// Wrap with `Arc` at the composition root.
#[derive(Debug, Clone)]
pub struct InMemoryResumeProducer {
    inner: SharedState,
}

impl InMemoryResumeProducer {
    /// Construct a producer backed by the given shared state.
    ///
    /// Pass the same `SharedState` used by the `InMemoryExecutionStore` so the
    /// token map (read/removed here) and the control queue (written here) are
    /// the ones the rest of the process observes.
    #[must_use]
    pub(super) fn new(inner: SharedState) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl ResumeProducer for InMemoryResumeProducer {
    async fn peek(&self, token_hash: &TokenHash) -> Result<Option<ResumeTokenRow>, StorageError> {
        let guard = self.inner.lock();
        let target = token_hash.as_bytes();
        // Best-effort constant-time key scan (defence-in-depth, parity with
        // `InMemoryResumeTokenStore::consume`); the map lookup is the
        // correctness primitive.
        let matching = guard
            .resume_tokens
            .iter()
            .find(|(key, _)| bool::from(key.as_slice().ct_eq(target)))
            .map(|(_, row)| row.clone());
        Ok(matching)
    }

    async fn consume_and_enqueue_resume(
        &self,
        token_hash: &TokenHash,
        resume_msg: &ControlMsg,
    ) -> Result<bool, StorageError> {
        // Fail-closed at the boundary: this producer enqueues ONLY `Resume`.
        // Checked BEFORE taking the lock or mutating, and release-enforced
        // (unlike a `debug_assert!`), so a misused command can never burn a token.
        if resume_msg.command != ControlCommand::Resume {
            return Err(StorageError::Internal(
                "ResumeProducer requires a Resume command".to_owned(),
            ));
        }

        let mut guard = self.inner.lock();
        let target = token_hash.as_bytes();
        let matching_key: Option<Vec<u8>> = guard
            .resume_tokens
            .keys()
            .find(|key| bool::from(key.as_slice().ct_eq(target)))
            .cloned();

        let Some(key) = matching_key else {
            // Zero rows removed — raced / replayed / absent. No enqueue.
            return Ok(false);
        };

        // Single-use gate: remove the token, then enqueue under the SAME lock.
        guard.resume_tokens.remove(&key);
        // Byte-identical to `InMemoryControlQueue::enqueue` so a `Resume`
        // produced here is indistinguishable from one minted via a commit.
        guard.queue.insert(
            resume_msg.id,
            QueuedMsg {
                msg: resume_msg.clone(),
                status: "Pending".to_string(),
                processed_by: None,
                processed_at: None,
                reclaim_count: 0,
                error_message: None,
            },
        );
        Ok(true)
    }
}
