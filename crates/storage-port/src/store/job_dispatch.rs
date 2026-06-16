//! Capability-routed job-dispatch queue port.
//!
//! The orchestrator pulls jobs by advertising the set of `CapabilityTag`s its
//! workers support; the queue delivers only rows whose `required_plugin_key`
//! is a member of that set.  The claim/fence/reclaim shape mirrors
//! `ControlQueue` — `ReclaimOutcome` is reused from that module.
use std::time::Duration;

use crate::dto::{CapabilityTag, JobDispatchMsg};
use crate::error::StorageError;
use crate::store::ReclaimOutcome;

/// Durable capability-routed job-dispatch queue.
///
/// The routing predicate is `required_plugin_key ∈ advertised_tags`.
/// Postgres uses `FOR UPDATE SKIP LOCKED` on `claim_pending`; SQLite uses a
/// single-consumer status flip.  Both are object-safe and Send+Sync.
#[async_trait::async_trait]
pub trait JobDispatchQueue: Send + Sync + std::fmt::Debug {
    /// Durably enqueue a job-dispatch message.
    async fn enqueue(&self, msg: &JobDispatchMsg) -> Result<(), StorageError>;

    /// Atomically claim up to `batch_size` pending jobs whose
    /// `required_plugin_key` is a member of `advertised_tags`.
    ///
    /// Postgres uses `FOR UPDATE SKIP LOCKED`; SQLite uses a single-consumer
    /// status flip with an explicit `AND status = 'Pending'` guard.
    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        advertised_tags: &[CapabilityTag],
    ) -> Result<Vec<JobDispatchMsg>, StorageError>;

    /// Mark a claimed job dispatched (terminal success).  Only the runner
    /// whose id matches the row's recorded processor may transition it
    /// (stale-worker fence).
    async fn mark_dispatched(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
    ) -> Result<(), StorageError>;

    /// Mark a claimed job failed (records `error`).  Same processor fence as
    /// [`Self::mark_dispatched`].
    async fn mark_failed(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
        error: &str,
    ) -> Result<(), StorageError>;

    /// Reclaim rows stuck in `Processing` past `reclaim_after`.  Rows under
    /// the `max_reclaim_count` budget go back to `Pending`; rows past it go
    /// to `Failed`.
    async fn reclaim_stuck(
        &self,
        reclaim_after: Duration,
        max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError>;

    /// Delete terminal rows older than `retention`; returns the count deleted.
    async fn cleanup(&self, retention: Duration) -> Result<u64, StorageError>;
}
