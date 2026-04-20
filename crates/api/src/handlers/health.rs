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
    models::{DependenciesStatus, HealthResponse, ReadinessResponse},
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
pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        timestamp: Utc::now().timestamp(),
    })
}

/// Readiness endpoint — reports whether every declared dependency is reachable.
///
/// Probes storage via a cheap `count()` query bounded by `PROBE_TIMEOUT`.
/// A failure or timeout flips `ready` to `false` **and** returns HTTP 503 so
/// orchestrators understand the process cannot currently serve traffic.
/// The body always carries the per-dependency breakdown for operator
/// visibility.
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

/// Probe the workflow repository with a bounded timeout.
///
/// `count()` is the cheapest read surface already on the trait, so adding
/// this probe does not widen the public API. A timeout OR an error from the
/// backend maps to "not ready" — readiness is a binary signal, callers do
/// not need to distinguish transport from timeout.
async fn probe_database(state: &AppState) -> bool {
    match tokio::time::timeout(PROBE_TIMEOUT, state.workflow_repo.count()).await {
        Ok(Ok(_)) => true,
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "readiness probe: workflow repo count() failed");
            false
        },
        Err(_) => {
            tracing::warn!(
                timeout_secs = PROBE_TIMEOUT.as_secs(),
                "readiness probe: workflow repo count() timed out",
            );
            false
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use axum::http::StatusCode;
    use nebula_core::WorkflowId;
    use nebula_storage::{
        InMemoryExecutionRepo, InMemoryWorkflowRepo, WorkflowRepo, WorkflowRepoError,
    };

    use super::*;
    use crate::{ApiConfig, state::AppState};

    /// Workflow repo that always fails `count()` — used to simulate a
    /// database outage in readiness probes (#291).
    struct AlwaysFailWorkflowRepo;

    #[async_trait]
    impl WorkflowRepo for AlwaysFailWorkflowRepo {
        async fn get_with_version(
            &self,
            _id: WorkflowId,
        ) -> Result<Option<(u64, serde_json::Value)>, WorkflowRepoError> {
            unimplemented!("not exercised by readiness tests")
        }

        async fn save(
            &self,
            _id: WorkflowId,
            _version: u64,
            _definition: serde_json::Value,
        ) -> Result<(), WorkflowRepoError> {
            unimplemented!("not exercised by readiness tests")
        }

        async fn delete(&self, _id: WorkflowId) -> Result<bool, WorkflowRepoError> {
            unimplemented!("not exercised by readiness tests")
        }

        async fn list(
            &self,
            _offset: usize,
            _limit: usize,
        ) -> Result<Vec<(WorkflowId, serde_json::Value)>, WorkflowRepoError> {
            unimplemented!("not exercised by readiness tests")
        }

        async fn count(&self) -> Result<usize, WorkflowRepoError> {
            Err(WorkflowRepoError::Connection("db offline".to_string()))
        }
    }

    /// Workflow repo whose `count()` sleeps for longer than `PROBE_TIMEOUT` —
    /// used to force the `Err(_)` timeout branch in `probe_database` (#291
    /// review). Pair with `#[tokio::test(start_paused = true)]` so the sleep
    /// is virtual and the test does not block for real seconds.
    struct SlowWorkflowRepo;

    #[async_trait]
    impl WorkflowRepo for SlowWorkflowRepo {
        async fn get_with_version(
            &self,
            _id: WorkflowId,
        ) -> Result<Option<(u64, serde_json::Value)>, WorkflowRepoError> {
            unimplemented!("not exercised by readiness tests")
        }

        async fn save(
            &self,
            _id: WorkflowId,
            _version: u64,
            _definition: serde_json::Value,
        ) -> Result<(), WorkflowRepoError> {
            unimplemented!("not exercised by readiness tests")
        }

        async fn delete(&self, _id: WorkflowId) -> Result<bool, WorkflowRepoError> {
            unimplemented!("not exercised by readiness tests")
        }

        async fn list(
            &self,
            _offset: usize,
            _limit: usize,
        ) -> Result<Vec<(WorkflowId, serde_json::Value)>, WorkflowRepoError> {
            unimplemented!("not exercised by readiness tests")
        }

        async fn count(&self) -> Result<usize, WorkflowRepoError> {
            // Far longer than PROBE_TIMEOUT (2s). Under paused time, the
            // runtime auto-advances to whichever timer fires first — that's
            // the timeout, so this sleep gets cancelled and never elapses
            // in wall-clock time.
            tokio::time::sleep(Duration::from_mins(1)).await;
            Ok(0)
        }
    }

    fn app_state_with_repo(repo: Arc<dyn WorkflowRepo>) -> AppState {
        let execution_repo = Arc::new(InMemoryExecutionRepo::new());
        let control_queue_repo: Arc<dyn nebula_storage::repos::ControlQueueRepo> =
            Arc::new(nebula_storage::repos::InMemoryControlQueueRepo::new());
        let config = ApiConfig::for_test();
        AppState::new(repo, execution_repo, control_queue_repo, config.jwt_secret)
    }

    #[tokio::test]
    async fn readiness_reports_ok_when_database_responds() {
        let state = app_state_with_repo(Arc::new(InMemoryWorkflowRepo::new()));
        let (status, Json(body)) = readiness_check(State(state)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.ready);
        assert!(body.dependencies.database);
    }

    #[tokio::test]
    async fn readiness_returns_503_when_database_probe_fails() {
        let state = app_state_with_repo(Arc::new(AlwaysFailWorkflowRepo));
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
        let state = app_state_with_repo(Arc::new(SlowWorkflowRepo));
        let (status, Json(body)) = readiness_check(State(state)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(!body.ready);
        assert!(!body.dependencies.database);
    }
}
