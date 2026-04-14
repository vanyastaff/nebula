//! Rate Limiting Middleware
//!
//! Global per-IP rate limiting for the Axum HTTP stack using the GCRA algorithm
//! (via the [`governor`] crate).
//!
//! [`/health`] and [`/ready`] paths are always excluded so that Kubernetes liveness
//! and readiness probes are never throttled even under load.
//!
//! # Examples
//!
//! ```
//! use nebula_api::middleware::rate_limit::RateLimitState;
//!
//! // 100 requests per second per IP
//! let state = RateLimitState::new(100);
//! ```

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    num::NonZeroU32,
    sync::Arc,
};

use axum::{
    extract::{ConnectInfo, Request},
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use governor::{DefaultKeyedRateLimiter, Quota};

/// Paths that bypass rate limiting entirely.
///
/// Health and readiness probes must always succeed — orchestrators depend on
/// them independent of any traffic volume.
const EXCLUDED_PATHS: &[&str] = &["/health", "/ready"];

/// Shared, per-IP rate-limiter state.
///
/// Internally backed by a lock-free [`DashMap`](dashmap::DashMap) so it can be
/// cheaply cloned and shared across async tasks.
///
/// # Examples
///
/// ```
/// use nebula_api::middleware::rate_limit::RateLimitState;
///
/// let state = RateLimitState::new(100); // 100 req/s per IP
/// assert_eq!(state.requests_per_second(), 100);
/// ```
#[derive(Clone)]
pub struct RateLimitState {
    pub(crate) limiter: Arc<DefaultKeyedRateLimiter<IpAddr>>,
    requests_per_second: u32,
}

impl RateLimitState {
    /// Create a rate limiter that allows `requests_per_second` requests per IP.
    ///
    /// Values below 1 are clamped to 1.
    #[must_use]
    pub fn new(requests_per_second: u32) -> Self {
        let rps = NonZeroU32::new(requests_per_second.max(1)).unwrap_or(NonZeroU32::MIN);
        let quota = Quota::per_second(rps);
        Self {
            limiter: Arc::new(DefaultKeyedRateLimiter::keyed(quota)),
            requests_per_second,
        }
    }

    /// Returns the configured rate per second.
    #[must_use]
    pub fn requests_per_second(&self) -> u32 {
        self.requests_per_second
    }

    /// Run the rate-limiting logic for a single request.
    ///
    /// Intended for use inside `axum::middleware::from_fn`:
    ///
    /// ```ignore
    /// let rl = RateLimitState::new(config.rate_limit_per_second);
    /// .layer(middleware::from_fn(move |req, next| {
    ///     let rl = rl.clone();
    ///     async move { rl.handle(req, next).await }
    /// }))
    /// ```
    pub async fn handle(&self, request: Request, next: Next) -> Response {
        // Bypass excluded paths (health / readiness probes)
        if EXCLUDED_PATHS.contains(&request.uri().path()) {
            return next.run(request).await;
        }

        let ip = extract_client_ip(&request);

        match self.limiter.check_key(&ip) {
            Ok(_) => next.run(request).await,
            Err(_) => {
                let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
                response
                    .headers_mut()
                    .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
                response
            }
        }
    }
}

/// Extract the best available client IP address from the request.
///
/// Resolution order:
///
/// 1. [`ConnectInfo<SocketAddr>`] extension — direct TCP peer address (most reliable,
///    requires `axum::serve` with `into_make_service_with_connect_info`).
/// 2. `X-Forwarded-For` header — first address in the comma-separated list.
/// 3. `X-Real-IP` header — single forwarded address.
/// 4. Loopback (`127.0.0.1`) as a last resort (e.g. in unit tests).
fn extract_client_ip(request: &Request) -> IpAddr {
    // 1. Direct TCP peer address
    if let Some(connect_info) = request.extensions().get::<ConnectInfo<SocketAddr>>() {
        return connect_info.0.ip();
    }

    // 2. X-Forwarded-For (first address in chain)
    if let Some(ip) = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|s| s.trim().parse::<IpAddr>().ok())
    {
        return ip;
    }

    // 3. X-Real-IP
    if let Some(ip) = request
        .headers()
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<IpAddr>().ok())
    {
        return ip;
    }

    // 4. Fallback (tests / direct loopback connections without ConnectInfo)
    IpAddr::V4(Ipv4Addr::LOCALHOST)
}
