//! In-memory reference adapter for cross-process resource runtime status.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use nebula_core::accessor::Clock;
use nebula_storage_port::dto::{LiveResourceStatus, ResourceStatusSnapshot, StatusWorkerId};
use nebula_storage_port::store::ResourceStatusStore;
use nebula_storage_port::{Scope, StorageError};
use parking_lot::Mutex;

use crate::resource_status::{HEARTBEAT_RETENTION_MS, heartbeat_ttl_ms, row_version_to_stored};

type StatusKey = (Scope, String, StatusWorkerId);

#[derive(Default)]
struct State {
    /// Worker id → heartbeat expiry in store milliseconds.
    heartbeats: HashMap<StatusWorkerId, i64>,
    statuses: HashMap<StatusKey, ResourceStatusSnapshot>,
}

/// In-memory reference implementation of [`ResourceStatusStore`].
///
/// One mutex guards both maps, matching the transaction boundary required of
/// deployment adapters, and liveness is judged in the same epoch milliseconds
/// the SQL backends store. This adapter is for tests and conformance, not a
/// supported persistence backend.
#[derive(Clone)]
pub struct InMemoryResourceStatusStore {
    inner: Arc<Mutex<State>>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for InMemoryResourceStatusStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InMemoryResourceStatusStore")
            .finish_non_exhaustive()
    }
}

impl Default for InMemoryResourceStatusStore {
    fn default() -> Self {
        Self::with_clock(Arc::new(nebula_core::accessor::SystemClock))
    }
}

impl InMemoryResourceStatusStore {
    /// Create an empty adapter driven by the system clock.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an empty adapter driven by an injected authoritative clock.
    #[must_use]
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(State::default())),
            clock,
        }
    }

    fn now_ms(&self) -> i64 {
        self.clock.now().timestamp_millis()
    }
}

#[async_trait::async_trait]
impl ResourceStatusStore for InMemoryResourceStatusStore {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "heartbeat"))]
    async fn heartbeat(&self, worker: &StatusWorkerId, ttl: Duration) -> Result<(), StorageError> {
        let now = self.now_ms();
        let mut state = self.inner.lock();
        state
            .heartbeats
            .insert(worker.clone(), now.saturating_add(heartbeat_ttl_ms(ttl)));
        // Only workers whose heartbeat is pruned lose their snapshots; a
        // snapshot published before its worker's first heartbeat stays, as it
        // does in the SQL backends.
        let horizon = now.saturating_sub(HEARTBEAT_RETENTION_MS);
        let pruned: Vec<StatusWorkerId> = state
            .heartbeats
            .iter()
            .filter(|(_, expires_at)| **expires_at < horizon)
            .map(|(owner, _)| owner.clone())
            .collect();
        if !pruned.is_empty() {
            state.heartbeats.retain(|owner, _| !pruned.contains(owner));
            state
                .statuses
                .retain(|(_, _, owner), _| !pruned.contains(owner));
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "publish"))]
    async fn publish(
        &self,
        scope: &Scope,
        worker: &StatusWorkerId,
        snapshot: &ResourceStatusSnapshot,
    ) -> Result<(), StorageError> {
        row_version_to_stored(snapshot.row_version)?;
        self.inner.lock().statuses.insert(
            (scope.clone(), snapshot.resource_id.clone(), worker.clone()),
            snapshot.clone(),
        );
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "withdraw"))]
    async fn withdraw(
        &self,
        scope: &Scope,
        worker: &StatusWorkerId,
        resource_id: &str,
    ) -> Result<(), StorageError> {
        self.inner
            .lock()
            .statuses
            .remove(&(scope.clone(), resource_id.to_owned(), worker.clone()));
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "withdraw_worker"))]
    async fn withdraw_worker(&self, worker: &StatusWorkerId) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        state.statuses.retain(|(_, _, owner), _| owner != worker);
        state.heartbeats.remove(worker);
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "live_for"))]
    async fn live_for(
        &self,
        scope: &Scope,
        resource_id: &str,
    ) -> Result<Vec<LiveResourceStatus>, StorageError> {
        let now = self.now_ms();
        let state = self.inner.lock();
        let mut live: Vec<LiveResourceStatus> = state
            .statuses
            .iter()
            .filter(|((row_scope, row_resource, owner), _)| {
                row_scope == scope
                    && row_resource == resource_id
                    && state
                        .heartbeats
                        .get(owner)
                        .is_some_and(|expires_at| *expires_at > now)
            })
            .map(|((_, _, owner), snapshot)| LiveResourceStatus {
                worker_id: owner.clone(),
                snapshot: snapshot.clone(),
            })
            .collect();
        live.sort_by(|left, right| left.worker_id.cmp(&right.worker_id));
        Ok(live)
    }
}
