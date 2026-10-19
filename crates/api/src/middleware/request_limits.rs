//! Process-wide bounds on HTTP work: a request deadline and a concurrency cap.
//!
//! The per-IP [`rate_limit`](crate::middleware::rate_limit) bounds how fast one
//! client may send. It does not bound how much work the process holds at once:
//! many clients, or handlers parked on a slow dependency, can still pile up
//! requests until memory or the database pool runs out. [`RequestLimits`]
//! adds the two missing bounds.
//!
//! - **Deadline.** A request whose response is not ready within
//!   `API_REQUEST_TIMEOUT` is answered with `503` and its handler future is
//!   dropped. The deadline covers producing the response, not streaming its
//!   body.
//! - **Concurrency cap.** At most `API_MAX_CONCURRENT_REQUESTS` requests run
//!   at once. A request over the cap is refused with `503` and `Retry-After`
//!   instead of queueing, so overload surfaces at once rather than as
//!   latency.
//!
//! `/health` and `/ready` bypass the cap, as they bypass the rate limiter: a
//! probe refused under load would invite the orchestrator to restart a process
//! that is only busy.

use std::{sync::Arc, time::Duration};

use axum::{
    extract::Request,
    http::{HeaderValue, header::RETRY_AFTER},
    middleware::Next,
    response::{IntoResponse, Response},
};
use tokio::sync::Semaphore;

use crate::{error::ApiError, middleware::rate_limit::EXCLUDED_PATHS};

/// Deadline and concurrency cap shared by every request of one router.
///
/// Cheap to clone; clones share the same permits.
#[derive(Debug, Clone)]
pub struct RequestLimits {
    permits: Arc<Semaphore>,
    timeout: Duration,
}

impl RequestLimits {
    /// Allow `max_concurrent` requests at once, each answered within `timeout`.
    #[must_use]
    pub fn new(max_concurrent: usize, timeout: Duration) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max_concurrent)),
            timeout,
        }
    }

    /// Apply both bounds to one request.
    pub async fn handle(&self, request: Request, next: Next) -> Response {
        let _permit = if EXCLUDED_PATHS.contains(&request.uri().path()) {
            None
        } else if let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() {
            Some(permit)
        } else {
            tracing::warn!("request refused: concurrency limit reached");
            let mut response = ApiError::ServiceUnavailable(
                "The server is at its concurrent request limit; retry shortly.".into(),
            )
            .into_response();
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from_static("1"));
            return response;
        };
        match tokio::time::timeout(self.timeout, next.run(request)).await {
            Ok(response) => response,
            Err(_) => {
                tracing::warn!(
                    timeout_ms = u64::try_from(self.timeout.as_millis()).unwrap_or(u64::MAX),
                    "request abandoned: deadline exceeded"
                );
                ApiError::ServiceUnavailable(
                    "The request did not complete within the server's time limit.".into(),
                )
                .into_response()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::get,
    };
    use tokio::sync::oneshot;
    use tower::ServiceExt;

    use super::RequestLimits;

    fn router(limits: RequestLimits, release: Option<oneshot::Receiver<()>>) -> Router {
        let release = std::sync::Arc::new(tokio::sync::Mutex::new(release));
        Router::new()
            .route(
                "/slow",
                get(move || {
                    let release = std::sync::Arc::clone(&release);
                    async move {
                        if let Some(receiver) = release.lock().await.take() {
                            let _ = receiver.await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                        "done"
                    }
                }),
            )
            .route("/health", get(|| async { "ok" }))
            .layer(middleware::from_fn(move |request, next| {
                let limits = limits.clone();
                async move { limits.handle(request, next).await }
            }))
    }

    fn get_request(path: &str) -> Request<Body> {
        Request::builder().uri(path).body(Body::empty()).unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_past_its_deadline_gets_503() {
        let app = router(RequestLimits::new(8, Duration::from_secs(5)), None);
        let response = app.oneshot(get_request("/slow")).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn a_request_over_the_cap_is_refused_and_probes_still_pass() {
        let (release, parked) = oneshot::channel();
        let limits = RequestLimits::new(1, Duration::from_secs(60));
        let app = router(limits.clone(), Some(parked));

        let held = tokio::spawn(app.clone().oneshot(get_request("/slow")));
        // Wait for the first request to take the only permit.
        while limits.permits.available_permits() > 0 {
            tokio::task::yield_now().await;
        }

        let refused = app.clone().oneshot(get_request("/slow")).await.unwrap();
        assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(refused.headers()["retry-after"], "1");

        let probe = app.clone().oneshot(get_request("/health")).await.unwrap();
        assert_eq!(probe.status(), StatusCode::OK);

        release.send(()).unwrap();
        let first = held.await.unwrap().unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(
            limits.permits.available_permits(),
            1,
            "the permit is returned"
        );
    }
}
