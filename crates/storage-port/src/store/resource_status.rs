//! Cross-process resource runtime-status store.
use std::time::Duration;

use crate::dto::{LiveResourceStatus, ResourceStatusSnapshot, StatusWorkerId};
use crate::error::StorageError;
use crate::scope::Scope;

/// Worker-published resource status, read back by the API process.
///
/// Liveness is the worker heartbeat: a snapshot is visible only while its
/// worker's heartbeat is unexpired, so a crashed worker's rows vanish from
/// reads without anyone deleting them. Every expiry is computed from the
/// store's own clock, never a caller's, so worker clock skew cannot keep a
/// dead worker alive or expire a live one early.
#[async_trait::async_trait]
pub trait ResourceStatusStore: Send + Sync + std::fmt::Debug {
    /// Marks `worker` live for `ttl` from now (store clock). Also prunes
    /// heartbeats, and their snapshots, that expired long ago.
    async fn heartbeat(&self, worker: &StatusWorkerId, ttl: Duration) -> Result<(), StorageError>;

    /// Records `worker`'s current view of one row in `scope`, replacing its
    /// previous view of that row.
    async fn publish(
        &self,
        scope: &Scope,
        worker: &StatusWorkerId,
        snapshot: &ResourceStatusSnapshot,
    ) -> Result<(), StorageError>;

    /// Removes `worker`'s view of one row (the row was retired there).
    async fn withdraw(
        &self,
        scope: &Scope,
        worker: &StatusWorkerId,
        resource_id: &str,
    ) -> Result<(), StorageError>;

    /// Removes every snapshot `worker` published and its heartbeat, for a
    /// graceful stop.
    async fn withdraw_worker(&self, worker: &StatusWorkerId) -> Result<(), StorageError>;

    /// Snapshots of one row in `scope` from workers whose heartbeat is live,
    /// ordered by worker id.
    async fn live_for(
        &self,
        scope: &Scope,
        resource_id: &str,
    ) -> Result<Vec<LiveResourceStatus>, StorageError>;
}
