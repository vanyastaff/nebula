//! Engine-side read-only resource runtime-status seam.
//!
//! [`EngineResourceStatus`] is the **read-only** projection of a stored
//! resource row's live runtime, exposed in api-safe types so a consumer
//! that must not depend on `nebula-resource` (the public API tier —
//! `deny.toml` `[[wrappers]]` forbids `nebula-api → nebula-resource`) can
//! still report runtime status.
//!
//! # Across processes
//!
//! A stored row is activated lazily inside the worker that drives an
//! execution binding it, so its runtime lives in that worker's `Manager`,
//! not in the API process asking for its status. Workers therefore publish
//! a per-row snapshot through a
//! [`ResourceStatusStore`]
//! ([`ResourceStatusPublisher`]) and the API reads it back
//! ([`StoredResourceStatus`]). A snapshot counts only while its worker's
//! heartbeat is live, so a crashed worker's rows fall out of the status on
//! their own.
//!
//! # No lifecycle mutation
//!
//! The seam exposes **only** a status read. There is no acquire / release /
//! drain / reload entry point: resource lifecycle is owned by the engine
//! and is not reachable through this seam.
//!
//! # Tenant isolation
//!
//! Reads are keyed by the row's own `(workspace, org)` scope and id, and the
//! API consults this seam only after its owned-row check. A snapshot carries
//! lifecycle state only — never configuration or credential material.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use nebula_storage_port::{
    Scope, StorageError,
    dto::{LiveResourceStatus, ResourceStatusPhase, ResourceStatusSnapshot, StatusWorkerId},
    store::ResourceStatusStore,
};
use tokio_util::sync::CancellationToken;

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Stable, non-secret runtime status of one stored resource row, aggregated
/// over every live worker that currently serves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRuntimeStatus {
    /// Lifecycle phase as a stable lowercase token: the least healthy phase
    /// any serving worker reports (`failed` outranks `ready`).
    pub phase: &'static str,
    /// `true` iff every serving worker reports the row healthy (`ready`).
    pub healthy: bool,
    /// `true` iff at least one serving worker can accept new acquires.
    pub accepting: bool,
    /// Number of live workers serving the row.
    pub instances: u32,
}

/// Why a status read could not be answered.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResourceStatusError {
    /// The status store could not be read.
    #[error("resource status store unavailable")]
    Unavailable(#[source] StorageError),
}

/// Read-only resource runtime-status port.
///
/// `Ok(None)` means no live worker serves the row — it exists as a
/// definition but is not currently active — distinct from an unanswerable
/// read (`Err`).
pub trait EngineResourceStatus: Send + Sync {
    /// Aggregated runtime status of version `row_version` of stored row
    /// `resource_id` in `scope`.
    ///
    /// Only workers running that version count: a worker still serving an
    /// earlier version heartbeats its old snapshot until it next activates
    /// the row, and must not make an updated definition look ready.
    fn runtime_status<'a>(
        &'a self,
        scope: &'a Scope,
        resource_id: &'a str,
        row_version: u64,
    ) -> BoxFut<'a, Result<Option<ResourceRuntimeStatus>, ResourceStatusError>>;
}

/// [`EngineResourceStatus`] reading worker-published snapshots.
pub struct StoredResourceStatus {
    store: Arc<dyn ResourceStatusStore>,
}

impl StoredResourceStatus {
    /// Reads status published into `store`.
    #[must_use]
    pub fn new(store: Arc<dyn ResourceStatusStore>) -> Self {
        Self { store }
    }
}

impl fmt::Debug for StoredResourceStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredResourceStatus")
            .finish_non_exhaustive()
    }
}

impl EngineResourceStatus for StoredResourceStatus {
    fn runtime_status<'a>(
        &'a self,
        scope: &'a Scope,
        resource_id: &'a str,
        row_version: u64,
    ) -> BoxFut<'a, Result<Option<ResourceRuntimeStatus>, ResourceStatusError>> {
        Box::pin(async move {
            let mut live = self
                .store
                .live_for(scope, resource_id)
                .await
                .map_err(ResourceStatusError::Unavailable)?;
            live.retain(|status| status.snapshot.row_version == row_version);
            Ok(aggregate(&live))
        })
    }
}

/// Severity order for aggregation, least healthy first.
const SEVERITY: [ResourceStatusPhase; 7] = [
    ResourceStatusPhase::Failed,
    ResourceStatusPhase::ShuttingDown,
    ResourceStatusPhase::Draining,
    ResourceStatusPhase::Unknown,
    ResourceStatusPhase::Initializing,
    ResourceStatusPhase::Reloading,
    ResourceStatusPhase::Ready,
];

fn severity(phase: ResourceStatusPhase) -> usize {
    SEVERITY
        .iter()
        .position(|known| *known == phase)
        .unwrap_or(0)
}

/// Folds per-worker snapshots into one status; `None` when no worker serves
/// the row.
fn aggregate(live: &[LiveResourceStatus]) -> Option<ResourceRuntimeStatus> {
    let worst = live
        .iter()
        .map(|status| status.snapshot.phase)
        .min_by_key(|phase| severity(*phase))?;
    Some(ResourceRuntimeStatus {
        phase: worst.as_str(),
        healthy: live.iter().all(|status| status.snapshot.healthy),
        accepting: live.iter().any(|status| status.snapshot.accepting),
        instances: u32::try_from(live.len()).unwrap_or(u32::MAX),
    })
}

/// Maps a `nebula_resource` lifecycle phase to the persisted vocabulary and
/// its `healthy` / `accepting` predicates, in one place.
///
/// `ResourcePhase` is `#[non_exhaustive]`: a variant this build does not
/// name maps to `unknown` and is conservatively reported not healthy.
pub(crate) fn project(
    phase: nebula_resource::state::ResourcePhase,
) -> (ResourceStatusPhase, bool, bool) {
    use nebula_resource::state::ResourcePhase;
    let persisted = match phase {
        ResourcePhase::Initializing => ResourceStatusPhase::Initializing,
        ResourcePhase::Ready => ResourceStatusPhase::Ready,
        ResourcePhase::Reloading => ResourceStatusPhase::Reloading,
        ResourcePhase::Draining => ResourceStatusPhase::Draining,
        ResourcePhase::ShuttingDown => ResourceStatusPhase::ShuttingDown,
        ResourcePhase::Failed => ResourceStatusPhase::Failed,
        _ => ResourceStatusPhase::Unknown,
    };
    (
        persisted,
        matches!(phase, ResourcePhase::Ready),
        phase.is_accepting(),
    )
}

/// Default interval between heartbeats and status diffs.
pub const DEFAULT_STATUS_PUBLISH_INTERVAL: Duration = Duration::from_secs(10);

/// How long a withdrawal may take on graceful stop before it is abandoned
/// (the heartbeat then expires on its own).
const WITHDRAW_BUDGET: Duration = Duration::from_secs(5);

/// Publishes this worker's stored-resource status for [`StoredResourceStatus`]
/// readers in other processes.
///
/// Each tick renews the worker heartbeat (TTL = three intervals, so one
/// missed tick does not blank the status) and writes only rows whose status
/// changed since the last successful write. A graceful stop withdraws
/// everything this worker published.
pub struct ResourceStatusPublisher {
    store: Arc<dyn ResourceStatusStore>,
    worker: StatusWorkerId,
    interval: Duration,
}

impl fmt::Debug for ResourceStatusPublisher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceStatusPublisher")
            .field("worker", &self.worker)
            .field("interval", &self.interval)
            .finish_non_exhaustive()
    }
}

pub(crate) type PublishedKey = (Scope, String);

/// This worker's stored-resource status at one instant, for
/// [`ResourceStatusPublisher`].
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ResourceStatusView {
    /// Rows registered here, with their current status.
    pub live: Vec<(Scope, ResourceStatusSnapshot)>,
    /// Rows an activation holds right now. Their last published status
    /// stays: the registration they had may still be serving.
    pub busy: Vec<PublishedKey>,
}

impl ResourceStatusPublisher {
    /// Publishes as `worker` into `store` every
    /// [`DEFAULT_STATUS_PUBLISH_INTERVAL`].
    #[must_use]
    pub fn new(store: Arc<dyn ResourceStatusStore>, worker: StatusWorkerId) -> Self {
        Self {
            store,
            worker,
            interval: DEFAULT_STATUS_PUBLISH_INTERVAL,
        }
    }

    /// Overrides the publish interval (clamped to at least 100 ms).
    #[must_use]
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval.max(Duration::from_millis(100));
        self
    }

    /// Runs until `shutdown`, then withdraws this worker's status.
    ///
    /// Store failures are logged and retried on the next tick; they never
    /// end the loop, because status is diagnostic and must not take a
    /// worker down.
    pub async fn run(self, engine: Arc<crate::WorkflowEngine>, shutdown: CancellationToken) {
        let mut published: HashMap<PublishedKey, ResourceStatusSnapshot> = HashMap::new();
        let mut ticker = tokio::time::interval(self.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => break,
                _ = ticker.tick() => {},
            }
            self.tick(&engine, &mut published).await;
        }
        let withdraw = self.store.withdraw_worker(&self.worker);
        match tokio::time::timeout(WITHDRAW_BUDGET, withdraw).await {
            Ok(Ok(())) => {},
            Ok(Err(error)) => tracing::warn!(
                target: "nebula_engine::resource_status",
                %error,
                "resource status withdrawal failed; it expires with the heartbeat"
            ),
            Err(_) => tracing::warn!(
                target: "nebula_engine::resource_status",
                "resource status withdrawal timed out; it expires with the heartbeat"
            ),
        }
    }

    pub(crate) async fn tick(
        &self,
        engine: &crate::WorkflowEngine,
        published: &mut HashMap<PublishedKey, ResourceStatusSnapshot>,
    ) {
        if let Err(error) = self
            .store
            .heartbeat(&self.worker, self.interval.saturating_mul(3))
            .await
        {
            tracing::warn!(
                target: "nebula_engine::resource_status",
                %error,
                "resource status heartbeat failed"
            );
            return;
        }
        // Deleted rows go first, so this tick already withdraws their status.
        engine.retire_deleted_resources().await;
        let view = engine.resource_status_snapshot();
        let mut seen: HashSet<PublishedKey> =
            HashSet::with_capacity(view.live.len() + view.busy.len());
        seen.extend(view.busy);
        for (scope, snapshot) in view.live {
            let key = (scope, snapshot.resource_id.clone());
            seen.insert(key.clone());
            if published.get(&key) == Some(&snapshot) {
                continue;
            }
            match self.store.publish(&key.0, &self.worker, &snapshot).await {
                Ok(()) => {
                    published.insert(key, snapshot);
                },
                Err(error) => {
                    published.remove(&key);
                    tracing::warn!(
                        target: "nebula_engine::resource_status",
                        %error,
                        "resource status publish failed; retrying next tick"
                    );
                },
            }
        }
        let retired: Vec<PublishedKey> = published
            .keys()
            .filter(|key| !seen.contains(*key))
            .cloned()
            .collect();
        for key in retired {
            match self.store.withdraw(&key.0, &self.worker, &key.1).await {
                Ok(()) => {
                    published.remove(&key);
                },
                Err(error) => tracing::warn!(
                    target: "nebula_engine::resource_status",
                    %error,
                    "resource status withdraw failed; retrying next tick"
                ),
            }
        }
    }
}

#[cfg(test)]
#[path = "resource_status_tests.rs"]
mod tests;
