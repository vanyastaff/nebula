//! Bounded graceful-shutdown gate for the HTTP surface.
//!
//! `axum::serve(...).with_graceful_shutdown(...)` stops accepting new
//! connections and waits for in-flight requests **without a bound**: one
//! handler parked on a dependency that never answers keeps the process
//! alive until the orchestrator's SIGKILL, so telemetry flush, credential
//! runtime shutdown, and the reservation sweep join never run.
//!
//! [`ShutdownGate`] wraps a router in a [`nebula_resilience::Gate`]: every
//! request enters the gate and holds an RAII guard until its response is
//! produced. On shutdown the composition root closes the gate with a
//! caller-chosen budget — new requests are rejected with `503`, and the
//! drain either completes or is abandoned on the budget with the active
//! request count reported.
//!
//! `/health` and `/ready` stay admitted while draining, mirroring
//! [`rate_limit`](crate::middleware::rate_limit): a probe that turned 503
//! during shutdown would invite the orchestrator to kill the process
//! early, which is exactly what the bounded drain exists to prevent.

use std::time::Duration;

use axum::{
    Router,
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::IntoResponse,
};
use nebula_resilience::{Gate, GateCloseTimeout, GateClosed, GateGuard};

/// Paths admitted (but still tracked) while the gate is closing.
const PROBE_PATHS: &[&str] = &["/health", "/ready"];

/// Tracks in-flight HTTP requests for bounded graceful shutdown.
///
/// Cheap to clone; all clones share the same gate. Construct one per
/// serving process, install it on the router, and close it when a shutdown
/// signal arrives.
///
/// # Examples
///
/// ```rust,no_run
/// use std::time::Duration;
///
/// use axum::{Router, routing::get};
/// use nebula_api::middleware::ShutdownGate;
///
/// # async fn run() {
/// let gate = ShutdownGate::new();
/// let app: Router = gate.install(Router::new().route("/", get(|| async { "ok" })));
/// # let _ = (app, gate.close(Duration::from_secs(1)).await);
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct ShutdownGate {
    gate: Gate,
}

impl ShutdownGate {
    /// Create an open gate with no in-flight requests.
    #[must_use]
    pub fn new() -> Self {
        Self { gate: Gate::new() }
    }

    /// Wrap `router` so every request holds a shutdown guard for its
    /// response's lifetime.
    ///
    /// Apply this to the fully built router so the gate sees every request,
    /// including those that will be rejected by inner middleware.
    pub fn install(&self, router: Router) -> Router {
        let gate = self.gate.clone();
        router.layer(middleware::from_fn(move |request: Request, next: Next| {
            let gate = gate.clone();
            async move {
                if PROBE_PATHS.contains(&request.uri().path()) {
                    return next.run(request).await;
                }
                match gate.enter() {
                    // The guard is bound (not `_`) so it lives until the
                    // response is produced; an unbound temporary would drop
                    // immediately and the gate would never see the request.
                    Ok(_guard) => next.run(request).await,
                    Err(GateClosed { .. }) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
                }
            }
        }))
    }

    /// Enter the gate outside request handling.
    ///
    /// Lets composition roots put their own bounded background work under
    /// the same drain barrier. Returns [`GateClosed`] once the gate is
    /// closing.
    ///
    /// # Errors
    ///
    /// Returns [`GateClosed`] when the gate is closing or already closed.
    pub fn enter(&self) -> Result<GateGuard, GateClosed> {
        self.gate.enter()
    }

    /// Whether the gate is closing or closed.
    #[must_use]
    pub fn is_closing(&self) -> bool {
        self.gate.is_closed()
    }

    /// Best-effort count of in-flight requests and entered background work.
    #[must_use]
    pub fn active_requests(&self) -> u32 {
        self.gate.active_count()
    }

    /// Close the gate and wait — for at most `budget` — for every guard to
    /// drop.
    ///
    /// # Errors
    ///
    /// Returns [`GateCloseTimeout`] when guards are still active after
    /// `budget`; the gate stays closed, so the caller can grant more time
    /// or abandon the remaining work.
    pub async fn close(&self, budget: Duration) -> Result<(), GateCloseTimeout> {
        self.gate.close(budget).await
    }
}

impl Default for ShutdownGate {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{body::Body, http::Request, routing::get};
    use tokio::sync::{Mutex, oneshot};
    use tower::ServiceExt;

    use super::*;

    fn request(path: &str) -> Request<Body> {
        Request::builder()
            .uri(path)
            .body(Body::empty())
            .expect("request builds")
    }

    /// Fire the one-shot that tells the test the handler has entered.
    async fn signal_started(tx: Arc<Mutex<Option<oneshot::Sender<()>>>>) {
        if let Some(tx) = tx.lock().await.take() {
            let _ = tx.send(());
        }
    }

    /// Park the handler until the test releases it.
    async fn await_release(rx: Arc<Mutex<Option<oneshot::Receiver<()>>>>) {
        if let Some(rx) = rx.lock().await.take() {
            let _ = rx.await;
        }
    }

    async fn park_then_finish(
        started_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
        release_rx: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
    ) -> &'static str {
        signal_started(started_tx).await;
        await_release(release_rx).await;
        "done"
    }

    #[tokio::test]
    async fn open_gate_admits_requests_and_tracks_none_after_completion() {
        let gate = ShutdownGate::new();
        let app = gate.install(Router::new().route("/", get(|| async { "ok" })));

        let response = app.oneshot(request("/")).await.expect("router serves");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(gate.active_requests(), 0);
    }

    #[tokio::test]
    async fn closed_gate_rejects_new_requests_with_503() {
        let gate = ShutdownGate::new();
        let app = gate.install(Router::new().route("/", get(|| async { "ok" })));
        gate.close(Duration::ZERO).await.expect("idle gate drains");

        let response = app.oneshot(request("/")).await.expect("router serves");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn close_waits_for_in_flight_request_then_admits_nothing() {
        let gate = ShutdownGate::new();
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let release_rx = Arc::new(Mutex::new(Some(release_rx)));
        let (started_tx, started_rx) = oneshot::channel::<()>();
        let started_tx = Arc::new(Mutex::new(Some(started_tx)));

        let handler = {
            let release_rx = Arc::clone(&release_rx);
            let started_tx = Arc::clone(&started_tx);
            move || park_then_finish(Arc::clone(&started_tx), Arc::clone(&release_rx))
        };
        let app = gate.install(Router::new().route("/work", get(handler)));

        let request_task = tokio::spawn(app.clone().oneshot(request("/work")));
        started_rx.await.expect("handler started");
        assert_eq!(gate.active_requests(), 1);

        let close_task = tokio::spawn({
            let gate = gate.clone();
            async move { gate.close(Duration::from_secs(5)).await }
        });

        // The close must not complete while the request is parked.
        tokio::task::yield_now().await;
        assert!(!close_task.is_finished());

        release_tx.send(()).expect("release the handler");
        request_task
            .await
            .expect("request task joins")
            .expect("router serves");
        close_task
            .await
            .expect("close task joins")
            .expect("drain completes once the guard drops");

        assert!(gate.is_closing());
        assert_eq!(gate.active_requests(), 0);
        let response = app.oneshot(request("/work")).await.expect("router serves");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn probes_stay_admitted_while_draining() {
        let gate = ShutdownGate::new();
        let app = gate.install(
            Router::new()
                .route("/health", get(|| async { "ok" }))
                .route("/", get(|| async { "ok" })),
        );
        gate.close(Duration::ZERO).await.expect("idle gate drains");

        let health = app.clone().oneshot(request("/health")).await;
        assert_eq!(
            health.expect("router serves").status(),
            StatusCode::OK,
            "a failing liveness probe during shutdown invites an early kill"
        );

        let root = app.oneshot(request("/")).await.expect("router serves");
        assert_eq!(root.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn enter_tracks_background_work_through_the_same_drain() {
        let gate = ShutdownGate::new();
        let guard = gate.enter().expect("gate starts open");
        assert_eq!(gate.active_requests(), 1);

        let err = gate
            .close(Duration::from_millis(20))
            .await
            .expect_err("a held guard must exhaust the budget");
        assert_eq!(err.active_guards, 1);

        drop(guard);
        gate.close(Duration::from_secs(1))
            .await
            .expect("drain completes once the background guard drops");
    }
}
