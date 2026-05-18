//! Durable control-signal outbox trait (cancel/terminate/pause + lease reclaim).
use std::time::Duration;

use crate::dto::ControlMsg;
use crate::error::StorageError;

/// Summary of a single `reclaim_stuck` sweep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReclaimOutcome {
    /// Rows moved `Processing → Pending` for redelivery.
    pub reclaimed: u64,
    /// Rows moved `Processing → Failed` because the reclaim budget ran out.
    pub exhausted: u64,
}

/// Durable control-command outbox.
///
/// `enqueue` is scoped so a low-privilege tenant cannot enqueue a
/// Cancel/Terminate for another tenant's execution (§6.1 confused-deputy
/// mitigation). Ids are typed 16-byte ULIDs (raw bytes on [`ControlMsg`]) —
/// the legacy "UTF-8 of the ULID string" encoding is gone. The processor
/// fencing on `mark_completed`/`mark_failed` is preserved (a stale runner
/// whose row was reclaimed cannot overwrite the new claim).
#[async_trait::async_trait]
pub trait ControlQueue: Send + Sync + std::fmt::Debug {
    /// Enqueue a control command (scoped).
    async fn enqueue(&self, msg: &ControlMsg) -> Result<(), StorageError>;

    /// Atomically claim up to `batch_size` pending commands for
    /// `processor`. Postgres uses `FOR UPDATE SKIP LOCKED`; SQLite uses a
    /// single-consumer status flip.
    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
    ) -> Result<Vec<ControlMsg>, StorageError>;

    /// Mark a claimed command completed. Only the runner whose id matches
    /// the row's recorded processor may transition it (stale-worker fence).
    async fn mark_completed(&self, id: &[u8; 16], processor: &[u8; 16])
    -> Result<(), StorageError>;

    /// Mark a claimed command failed (records `error`). Same processor
    /// fence as [`Self::mark_completed`].
    async fn mark_failed(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
        error: &str,
    ) -> Result<(), StorageError>;

    /// Reclaim rows stuck in `Processing` past `reclaim_after`. Rows under
    /// the `max_reclaim_count` budget go back to `Pending`; rows past it go
    /// to `Failed`.
    async fn reclaim_stuck(
        &self,
        reclaim_after: Duration,
        max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError>;

    /// Delete rows older than `retention`; returns the count deleted.
    async fn cleanup(&self, retention: Duration) -> Result<u64, StorageError>;
}
