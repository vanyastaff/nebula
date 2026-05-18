//! Health check handlers
//!
//! - `GET /health` — liveness probe: the process is running and can answer HTTP. Never touches
//!   dependencies, so orchestrators can tell the process apart from its dependencies.
//! - `GET /ready` — readiness probe: the process is ready to serve traffic, which requires **every
//!   declared dependency** to be reachable. If any dependency is degraded the handler returns 503
//!   so orchestrators (k8s `readinessProbe`, load balancers) pull the pod out of rotation instead
//!   of routing traffic that would error (#291).

use std::time::Duration;

use axum::{Json, extract::State, http::StatusCode};
use chrono::Utc;

use crate::{
    domain::health::dto::{DependenciesStatus, HealthResponse, ReadinessResponse, VersionInfo},
    state::AppState,
};

/// Hard cap on how long a single dependency probe is allowed to take.
///
/// Readiness probes must stay cheap — orchestrators poll them every few
/// seconds. A slow or stuck dependency must flip readiness to `false` fast
/// enough that a circuit-breaker / load-shedder can act, rather than piling
/// up long-running readiness requests.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Liveness endpoint — reports that the process is up.
///
/// Does not consult any dependency. Kubernetes / operators use this to
/// decide whether the container should be restarted; a dependency outage
/// must not cause a restart loop.
#[utoipa::path(
    get,
    path = "/health",
    tag = "system",
    security(()),
    responses(
        (status = 200, description = "Process is up; HTTP listener is responsive.", body = HealthResponse),
    ),
)]
pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        timestamp: Utc::now().timestamp(),
    })
}

/// Version info endpoint.
///
/// Returns the application name and version. Unauthenticated.
#[utoipa::path(
    get,
    path = "/version",
    tag = "system",
    security(()),
    responses(
        (status = 200, description = "Application name and version.", body = VersionInfo),
    ),
)]
pub async fn version_info() -> Json<VersionInfo> {
    Json(VersionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: "nebula".to_string(),
    })
}

/// Readiness endpoint — reports whether every declared dependency is reachable.
///
/// Probes storage via a cheap `count()` query bounded by `PROBE_TIMEOUT`.
/// A failure or timeout flips `ready` to `false` **and** returns HTTP 503 so
/// orchestrators understand the process cannot currently serve traffic.
/// The body always carries the per-dependency breakdown for operator
/// visibility.
#[utoipa::path(
    get,
    path = "/ready",
    tag = "system",
    security(()),
    responses(
        (status = 200, description = "All declared dependencies are reachable.", body = ReadinessResponse),
        (status = 503, description = "At least one dependency is degraded; the body still carries the per-dependency breakdown.", body = ReadinessResponse),
    ),
)]
pub async fn readiness_check(
    State(state): State<AppState>,
) -> (StatusCode, Json<ReadinessResponse>) {
    let database_ok = probe_database(&state).await;

    let response = ReadinessResponse {
        ready: database_ok,
        dependencies: DependenciesStatus {
            database: database_ok,
            cache: None,
        },
    };

    let status = if database_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(response))
}

/// Probe the workflow store with a bounded timeout.
///
/// Uses [`WorkflowStore::is_reachable`] — the tenant-agnostic
/// infrastructure-liveness round-trip (`SELECT 1` on the SQL backends).
/// The readiness probe is unauthenticated infrastructure with no tenant,
/// so it deliberately does **not** construct a [`Scope`] or read tenant
/// rows: it only answers "is the store reachable?". A timeout OR an
/// error maps to "not ready" — readiness is a binary signal, callers do
/// not need to distinguish transport from timeout.
///
/// [`Scope`]: nebula_storage_port::Scope
async fn probe_database(state: &AppState) -> bool {
    match tokio::time::timeout(PROBE_TIMEOUT, state.workflow_store.is_reachable()).await {
        Ok(Ok(())) => true,
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "readiness probe: workflow store unreachable");
            false
        },
        Err(_) => {
            tracing::warn!(
                timeout_secs = PROBE_TIMEOUT.as_secs(),
                "readiness probe: workflow store reachability timed out",
            );
            false
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{ApiConfig, state::AppState};
    use async_trait::async_trait;
    use axum::http::StatusCode;
    use nebula_storage::inmem::{
        InMemoryControlQueue, InMemoryExecutionStore, InMemoryJournalReader,
        InMemoryNodeResultStore, InMemoryWorkflowStore, InMemoryWorkflowVersionStore,
    };
    use nebula_storage_port::{
        Scope, StorageError,
        dto::{WorkflowRecord, WorkflowVersionRecord},
        store::WorkflowStore,
    };

    /// `WorkflowStore` whose `is_reachable()` always fails — simulates a
    /// database outage in readiness probes (#291). The probe path is
    /// `probe_database` → `WorkflowStore::is_reachable`, so failing
    /// `is_reachable` is the exact surface the readiness check exercises.
    #[derive(Debug)]
    struct AlwaysFailWorkflowStore;

    #[async_trait]
    impl WorkflowStore for AlwaysFailWorkflowStore {
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn create(&self, _: &Scope, _: WorkflowRecord) -> Result<(), StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn get(&self, _: &Scope, _: &str) -> Result<Option<WorkflowRecord>, StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn get_by_slug(
            &self,
            _: &Scope,
            _: &str,
        ) -> Result<Option<WorkflowRecord>, StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn update(&self, _: &Scope, _: WorkflowRecord, _: u64) -> Result<(), StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn save_with_published_version(
            &self,
            _: &Scope,
            _: WorkflowRecord,
            _: WorkflowVersionRecord,
            _: Option<u64>,
        ) -> Result<(), StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn soft_delete(&self, _: &Scope, _: &str) -> Result<(), StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn list(&self, _: &Scope) -> Result<Vec<WorkflowRecord>, StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn count(&self, _: &Scope) -> Result<u64, StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        async fn is_reachable(&self) -> Result<(), StorageError> {
            Err(StorageError::Connection("db offline".to_string()))
        }
    }

    /// `WorkflowStore` whose `is_reachable()` sleeps for longer than
    /// `PROBE_TIMEOUT` — forces the `Err(_)` timeout branch in
    /// `probe_database` (#291 review). Pair with
    /// `#[tokio::test(start_paused = true)]` so the sleep is virtual and
    /// the test does not block for real seconds.
    #[derive(Debug)]
    struct SlowWorkflowStore;

    #[async_trait]
    impl WorkflowStore for SlowWorkflowStore {
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn create(&self, _: &Scope, _: WorkflowRecord) -> Result<(), StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn get(&self, _: &Scope, _: &str) -> Result<Option<WorkflowRecord>, StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn get_by_slug(
            &self,
            _: &Scope,
            _: &str,
        ) -> Result<Option<WorkflowRecord>, StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn update(&self, _: &Scope, _: WorkflowRecord, _: u64) -> Result<(), StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn save_with_published_version(
            &self,
            _: &Scope,
            _: WorkflowRecord,
            _: WorkflowVersionRecord,
            _: Option<u64>,
        ) -> Result<(), StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn soft_delete(&self, _: &Scope, _: &str) -> Result<(), StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn list(&self, _: &Scope) -> Result<Vec<WorkflowRecord>, StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        // guard-justified: readiness probe only calls `is_reachable`;
        // this op is unreachable on the probe path.
        async fn count(&self, _: &Scope) -> Result<u64, StorageError> {
            unimplemented!("not exercised by readiness tests")
        }
        async fn is_reachable(&self) -> Result<(), StorageError> {
            // Far longer than PROBE_TIMEOUT (2s). Under paused time, the
            // runtime auto-advances to whichever timer fires first — that's
            // the timeout, so this sleep gets cancelled and never elapses
            // in wall-clock time.
            tokio::time::sleep(Duration::from_mins(1)).await;
            Ok(())
        }
    }

    /// Build an `AppState` whose `WorkflowStore` is the supplied (possibly
    /// failing) port store, with real in-memory adapters for the rest —
    /// all stored raw (undecorated), the canonical composition-root wiring
    /// (mirrors `server::default_state`). The per-request tenant scope is
    /// applied by the `AppState` accessors, not baked in here.
    fn app_state_with_workflow_store(workflow_store: Arc<dyn WorkflowStore>) -> AppState {
        let config = ApiConfig::for_test();
        let exec_store = InMemoryExecutionStore::new();
        let control_queue = InMemoryControlQueue::new(&exec_store);
        let journal = InMemoryJournalReader::new(&exec_store);

        AppState::new(
            workflow_store,
            Arc::new(InMemoryWorkflowVersionStore::new()),
            Arc::new(exec_store),
            Arc::new(InMemoryNodeResultStore::new()),
            Arc::new(journal),
            Arc::new(control_queue),
            config.jwt_secret,
        )
    }

    #[tokio::test]
    async fn readiness_reports_ok_when_database_responds() {
        // `readiness_check` only probes `WorkflowStore::is_reachable()`
        // (the row store); it never reads the version store, so the
        // paired version map is immaterial here — construct the canonical
        // pair anyway so the store is wired exactly as production builds
        // it.
        let workflow_versions = InMemoryWorkflowVersionStore::new();
        let workflow_store = InMemoryWorkflowStore::new_with_versions(&workflow_versions);
        let state = app_state_with_workflow_store(Arc::new(workflow_store));
        let (status, Json(body)) = readiness_check(State(state)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.ready);
        assert!(body.dependencies.database);
    }

    #[tokio::test]
    async fn readiness_returns_503_when_database_probe_fails() {
        let state = app_state_with_workflow_store(Arc::new(AlwaysFailWorkflowStore));
        let (status, Json(body)) = readiness_check(State(state)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(!body.ready);
        assert!(!body.dependencies.database);
    }

    /// Covers the `Err(_)` timeout branch in `probe_database`: when
    /// `count()` exceeds `PROBE_TIMEOUT`, readiness must flip to `false`
    /// and the handler must respond with 503 so orchestrators drain the
    /// pod (#291 review). Uses paused time so the test completes in
    /// virtual microseconds rather than waiting 2 real seconds.
    #[tokio::test(start_paused = true)]
    async fn readiness_returns_503_when_database_probe_times_out() {
        let state = app_state_with_workflow_store(Arc::new(SlowWorkflowStore));
        let (status, Json(body)) = readiness_check(State(state)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(!body.ready);
        assert!(!body.dependencies.database);
    }
}
