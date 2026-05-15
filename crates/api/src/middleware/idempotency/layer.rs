//! Tower [`Layer`] + [`Service`] implementing idempotent-replay middleware.
//!
//! [`IdempotencyLayer`] wraps any inner service with the full ADR-0048
//! idempotency contract: request-body buffering, SHA-256 fingerprinting,
//! cache lookup, 422 on body mismatch, 5xx pass-through (not cached),
//! response-header filtering, `Idempotent-Replay: true` on replay, and
//! metrics emission via [`MetricsRegistry`].

use std::{
    sync::Arc,
    task::{Context, Poll},
};

use axum::{
    body::Body,
    extract::Request,
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures::future::BoxFuture;
use nebula_metrics::{
    MetricsRegistry,
    naming::{
        NEBULA_API_IDEMPOTENCY_HITS_TOTAL, NEBULA_API_IDEMPOTENCY_LATENCY_MS,
        NEBULA_API_IDEMPOTENCY_MISSES_TOTAL, NEBULA_API_IDEMPOTENCY_REJECTS_TOTAL,
        NEBULA_API_IDEMPOTENCY_STORE_SATURATION_PPM, idempotency_reject_reason,
    },
};
use tower::{Layer, Service, ServiceExt};

use super::{
    IDEMPOTENCY_KEY_HEADER, IDEMPOTENT_REPLAY_HEADER, IdempotencyConfig,
    key::{IdempotencyKey, build_cache_key, fingerprint_request_body, identity_fingerprint},
    store::{CachedResponse, IdempotencyStore},
};

// ── Layer ────────────────────────────────────────────────────────────────────

/// Tower [`Layer`] that wraps an inner service with idempotent-replay logic.
///
/// Construct with [`IdempotencyLayer::new`] and apply via `Router::layer`.
#[derive(Clone)]
pub struct IdempotencyLayer {
    store: Arc<dyn IdempotencyStore>,
    config: IdempotencyConfig,
    metrics: Option<Arc<MetricsRegistry>>,
}

impl IdempotencyLayer {
    /// Build a layer backed by the given store, using [`IdempotencyConfig::default`].
    #[must_use]
    pub fn new(store: Arc<dyn IdempotencyStore>) -> Self {
        Self {
            store,
            config: IdempotencyConfig::default(),
            metrics: None,
        }
    }

    /// Override the layer configuration.
    #[must_use]
    pub fn with_config(mut self, config: IdempotencyConfig) -> Self {
        self.config = config;
        self
    }

    /// Attach a [`MetricsRegistry`] so the layer records
    /// `nebula_api_idempotency_*` counters / gauge / histogram on every
    /// outcome branch. When `None`, the layer skips metric recording but
    /// still emits the existing `tracing` span fields.
    ///
    /// Constructor injection is intentional — the layer runs before
    /// `axum::extract::State`, so it cannot pull the registry from
    /// `AppState` at request time. `build_app` reads
    /// `state.metrics_registry` and threads it in here.
    #[must_use]
    pub fn with_metrics(mut self, metrics: Option<Arc<MetricsRegistry>>) -> Self {
        self.metrics = metrics;
        self
    }
}

impl<S> Layer<S> for IdempotencyLayer {
    type Service = IdempotencyService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        IdempotencyService {
            inner,
            store: Arc::clone(&self.store),
            config: self.config.clone(),
            metrics: self.metrics.clone(),
        }
    }
}

// ── Service ──────────────────────────────────────────────────────────────────

/// Tower [`Service`] produced by [`IdempotencyLayer`].
#[derive(Clone)]
pub struct IdempotencyService<S> {
    inner: S,
    store: Arc<dyn IdempotencyStore>,
    config: IdempotencyConfig,
    metrics: Option<Arc<MetricsRegistry>>,
}

impl<S> Service<Request> for IdempotencyService<S>
where
    S: Service<Request, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let inner = self.inner.clone();
        let store = Arc::clone(&self.store);
        let config = self.config.clone();
        let metrics = self.metrics.clone();
        Box::pin(async move { handle(inner, store, config, metrics, request).await })
    }
}

// ── Core handling ────────────────────────────────────────────────────────────

/// Metric counter family the request hits at this branch.
///
/// The string outcome on the tracing span is left as the operator-facing
/// label; this enum is the structured shape used to drive
/// `nebula_api_idempotency_*` counters. Decoupling them lets us extend
/// span outcomes (`miss:resp_too_large`, `miss:passthrough`) without
/// re-shuffling counter cardinality and vice-versa.
#[derive(Debug, Clone, Copy)]
enum MetricOutcome {
    /// Pass-through (non-POST or no header) — no counter.
    None,
    /// Cache hit — bumps `nebula_api_idempotency_hits_total`.
    Hit,
    /// Cache miss — bumps `nebula_api_idempotency_misses_total`. Covers
    /// `miss:cached` (response stored), `miss:passthrough` (5xx not
    /// cached), and `miss:resp_too_large` (handler ran but response
    /// wasn't cacheable).
    Miss,
    /// Reject path with reason label (closed set per
    /// [`idempotency_reject_reason`]).
    Reject(&'static str),
}

fn record_outcome(
    metrics: &Option<Arc<MetricsRegistry>>,
    span_outcome: &'static str,
    metric: MetricOutcome,
) {
    tracing::Span::current().record("outcome", span_outcome);
    let Some(registry) = metrics.as_ref() else {
        return;
    };
    match metric {
        MetricOutcome::None => {},
        MetricOutcome::Hit => {
            if let Ok(c) = registry.counter(NEBULA_API_IDEMPOTENCY_HITS_TOTAL) {
                c.inc();
            }
        },
        MetricOutcome::Miss => {
            if let Ok(c) = registry.counter(NEBULA_API_IDEMPOTENCY_MISSES_TOTAL) {
                c.inc();
            }
        },
        MetricOutcome::Reject(reason) => {
            let labels = registry.interner().single("reason", reason);
            if let Ok(c) = registry.counter_labeled(NEBULA_API_IDEMPOTENCY_REJECTS_TOTAL, &labels) {
                c.inc();
            }
        },
    }
    tracing::debug!(label = span_outcome, "idempotency metric recorded");
}

fn update_saturation(metrics: &Option<Arc<MetricsRegistry>>, store: &Arc<dyn IdempotencyStore>) {
    let Some(registry) = metrics.as_ref() else {
        return;
    };
    let Some(ppm) = store.saturation_ppm() else {
        return;
    };
    if let Ok(g) = registry.gauge(NEBULA_API_IDEMPOTENCY_STORE_SATURATION_PPM) {
        g.set(i64::try_from(ppm).unwrap_or(i64::MAX));
    }
}

/// Drop guard observing the middleware-path latency histogram regardless
/// of which branch returned. Construction starts the timer; `Drop` reads
/// the elapsed time and records it. Putting it on the stack at the top of
/// `handle` avoids inserting a record call at every return statement.
struct LatencyGuard {
    metrics: Option<Arc<MetricsRegistry>>,
    start: std::time::Instant,
}

impl Drop for LatencyGuard {
    fn drop(&mut self) {
        let Some(registry) = self.metrics.as_ref() else {
            return;
        };
        let Ok(h) = registry.histogram(NEBULA_API_IDEMPOTENCY_LATENCY_MS) else {
            return;
        };
        let elapsed_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        h.observe(elapsed_ms);
    }
}

impl CachedResponse {
    pub(super) fn into_response(self) -> Response {
        let mut response = Response::builder()
            .status(self.status)
            .body(Body::from(self.body))
            .unwrap_or_else(|_| {
                // Building from a static StatusCode + Vec<u8> body cannot fail
                // under any documented `http::response::Builder` precondition;
                // fall back to an empty 500 only to keep the type checker
                // happy. Recoverable rather than panicking.
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            });
        *response.headers_mut() = self.headers;
        response
            .headers_mut()
            .insert(IDEMPOTENT_REPLAY_HEADER, HeaderValue::from_static("true"));
        response
    }
}

#[tracing::instrument(
    name = "idempotency",
    skip(inner, store, config, metrics, request),
    fields(
        method = %request.method(),
        path = %request.uri().path(),
        idempotency_key = tracing::field::Empty,
        outcome = tracing::field::Empty,
        cache_key_prefix = tracing::field::Empty,
        identity_prefix = tracing::field::Empty,
        body_size_bytes = tracing::field::Empty,
    )
)]
async fn handle<S>(
    inner: S,
    store: Arc<dyn IdempotencyStore>,
    config: IdempotencyConfig,
    metrics: Option<Arc<MetricsRegistry>>,
    request: Request,
) -> Result<Response, S::Error>
where
    S: Service<Request, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    let _latency_guard = LatencyGuard {
        metrics: metrics.clone(),
        start: std::time::Instant::now(),
    };
    // Only POST is in scope for the M3.4 acceptance criteria. Other methods
    // pass through transparently — clients that send the header on a GET get
    // their request handled normally.
    if request.method() != Method::POST {
        record_outcome(&metrics, "skip:non_post", MetricOutcome::None);
        return inner.oneshot(request).await;
    }

    // Header missing → opt-out. No allocation, no body buffering.
    let Some(raw_key) = request.headers().get(IDEMPOTENCY_KEY_HEADER) else {
        record_outcome(&metrics, "skip:no_header", MetricOutcome::None);
        return inner.oneshot(request).await;
    };

    let Ok(key_str) = raw_key.to_str() else {
        record_outcome(
            &metrics,
            "reject:non_ascii_header",
            MetricOutcome::Reject(idempotency_reject_reason::NON_ASCII_HEADER),
        );
        return Ok(bad_request("Idempotency-Key must be valid ASCII"));
    };

    let key = match IdempotencyKey::parse(key_str) {
        Ok(k) => k,
        Err(err) => {
            record_outcome(
                &metrics,
                "reject:invalid_key",
                MetricOutcome::Reject(idempotency_reject_reason::INVALID_KEY),
            );
            return Ok(bad_request(&err.to_string()));
        },
    };
    tracing::Span::current().record("idempotency_key", tracing::field::display(&key));

    // Buffer the request body so we can fingerprint it AND replay it to the
    // inner service. Anything beyond `max_request_body_bytes` opts the request
    // out of caching entirely — we still forward it, but cannot guarantee
    // replay safety, so we do not store the response either.
    //
    // Buffer with `usize::MAX` (no middleware-level cap) so the bytes
    // are never lost on overflow. The router-level
    // `axum::extract::DefaultBodyLimit` (default 1 MiB) is the
    // authoritative request-size cap for the public surface — it
    // rejects oversized bodies with 413 before reaching this layer.
    // After buffering we check the size against
    // `max_request_body_bytes` and forward without caching when it
    // exceeds the cap — handler MUST see the original body unchanged
    // (Codex P1 on PR #658: silent body truncation broke handler
    // semantics on every oversized request).
    let (parts, body) = request.into_parts();
    let body_bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(b) => b,
        Err(err) => {
            record_outcome(&metrics, "error:body_read", MetricOutcome::None);
            tracing::error!(
                error = %err,
                "request body read failed — failing closed"
            );
            return Ok(internal_error(
                "request body read failed; cannot evaluate idempotency",
            ));
        },
    };
    if body_bytes.len() > config.max_request_body_bytes {
        record_outcome(
            &metrics,
            "skip:body_too_large",
            MetricOutcome::Reject(idempotency_reject_reason::BODY_TOO_LARGE),
        );
        tracing::warn!(
            body_size = body_bytes.len(),
            limit = config.max_request_body_bytes,
            "request body exceeds idempotency max_request_body_bytes — \
             forwarding without caching"
        );
        let request = Request::from_parts(parts, Body::from(body_bytes));
        return inner.oneshot(request).await;
    }
    let body_vec: Vec<u8> = body_bytes.into();
    tracing::Span::current().record("body_size_bytes", body_vec.len());

    let request_fingerprint = fingerprint_request_body(&body_vec);
    let identity = identity_fingerprint(&parts.headers);
    let cache_key = build_cache_key(&parts.method, parts.uri.path(), &key, &identity);
    // Privacy: never record the full cache key or identity fingerprint —
    // the key embeds the client-supplied `Idempotency-Key` and identity
    // material derived from auth headers. An 8-byte prefix (cache key)
    // and 16 hex chars (identity, 8 leading bytes) are enough to
    // disambiguate spans in a trace without leaking client identity into
    // log sinks.
    let cache_key_prefix: String = cache_key.chars().take(8).collect();
    let mut identity_prefix = String::with_capacity(16);
    for byte in &identity[..8] {
        identity_prefix.push_str(&format!("{byte:02x}"));
    }
    tracing::Span::current().record("cache_key_prefix", cache_key_prefix.as_str());
    tracing::Span::current().record("identity_prefix", identity_prefix.as_str());

    let lookup = match store.get(&cache_key).await {
        Ok(opt) => opt,
        Err(err) => {
            record_outcome(&metrics, "error:get", MetricOutcome::None);
            tracing::error!(
                error = %err,
                "idempotency-store get failed — failing the request closed (ADR-0048)"
            );
            return Ok(internal_error(
                "idempotency store unavailable; request rejected to preserve replay protection",
            ));
        },
    };
    if let Some(cached) = lookup {
        if cached.request_fingerprint != request_fingerprint {
            record_outcome(
                &metrics,
                "reject:body_mismatch",
                MetricOutcome::Reject(idempotency_reject_reason::BODY_MISMATCH),
            );
            tracing::warn!("Idempotency-Key reused with different request body — rejecting");
            return Ok(unprocessable(
                "Idempotency-Key reused with a different request payload",
            ));
        }
        record_outcome(&metrics, "hit", MetricOutcome::Hit);
        tracing::debug!(status = cached.status.as_u16(), "idempotency cache hit");
        return Ok(Arc::unwrap_or_clone(cached).into_response());
    }

    // Cache miss — rebuild the request with the buffered body and run the
    // inner handler.
    let request = Request::from_parts(parts, Body::from(body_vec));
    let response = inner.oneshot(request).await?;

    let (resp_parts, resp_body) = response.into_parts();

    // Buffer with `usize::MAX` (no middleware-level cap) so the bytes
    // are never lost on overflow. axum's IntoResponse does not set
    // `Content-Length` inside the middleware chain (hyper sets it on
    // serialization), so a Content-Length pre-check would be dead code.
    // Memory bound: response bodies in this codebase are produced by
    // first-party handlers and are not adversarial; if a future handler
    // can return unbounded output it must apply its own streaming cap
    // upstream of this layer.
    let resp_bytes = match axum::body::to_bytes(resp_body, usize::MAX).await {
        Ok(b) => b,
        Err(err) => {
            record_outcome(&metrics, "error:resp_read", MetricOutcome::Miss);
            tracing::error!(
                error = %err,
                "response body read failed — returning 500"
            );
            return Ok(internal_error(
                "response body read failed; idempotency layer cannot forward",
            ));
        },
    };
    if resp_bytes.len() > config.max_response_body_bytes {
        // Forward the full body to the caller; just skip caching it.
        // Previously this branch returned `Body::empty()`, which
        // truncated valid responses once the layer was mounted in
        // production (Codex P1 on PR #658).
        record_outcome(&metrics, "miss:resp_too_large", MetricOutcome::Miss);
        tracing::warn!(
            body_size = resp_bytes.len(),
            limit = config.max_response_body_bytes,
            "response body exceeds idempotency max_response_body_bytes — \
             forwarding without caching"
        );
        return Ok(Response::from_parts(resp_parts, Body::from(resp_bytes)));
    }
    let resp_vec: Vec<u8> = resp_bytes.into();

    if should_cache(resp_parts.status) {
        let cached = Arc::new(CachedResponse {
            status: resp_parts.status,
            headers: filter_response_headers(&resp_parts.headers),
            body: resp_vec.clone(),
            request_fingerprint,
        });
        match store.put(cache_key, cached).await {
            Ok(()) => {
                update_saturation(&metrics, &store);
                record_outcome(&metrics, "miss:cached", MetricOutcome::Miss);
                tracing::debug!(
                    status = resp_parts.status.as_u16(),
                    bytes = resp_vec.len(),
                    "idempotency cache miss — response cached"
                );
            },
            Err(err) => {
                // `put` failure does not surface to the caller — the
                // response is valid, only the dedup record was lost.
                // The next replay of this key will run the inner
                // handler again. Operators see the warn-level log.
                record_outcome(&metrics, "miss:put_failed", MetricOutcome::Miss);
                tracing::warn!(
                    error = %err,
                    "idempotency cache put failed — response returned without caching"
                );
            },
        }
    } else {
        record_outcome(&metrics, "miss:passthrough", MetricOutcome::Miss);
        tracing::debug!(
            status = resp_parts.status.as_u16(),
            "idempotency cache miss — response not cached (5xx)"
        );
    }

    Ok(Response::from_parts(resp_parts, Body::from(resp_vec)))
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn should_cache(status: StatusCode) -> bool {
    // Cache 2xx (success) and 4xx (client errors). 5xx is left uncached so a
    // transient backend failure does not pin a permanent error for the TTL
    // window — the caller can safely retry the same key.
    status.is_success() || status.is_client_error()
}

fn filter_response_headers(headers: &HeaderMap) -> HeaderMap {
    // Strip headers that must never be replayed verbatim:
    // - hop-by-hop (`Connection`, `Transfer-Encoding`, `Upgrade`, …)
    // - per-request (`X-Request-ID` is regenerated by `request_id_middleware` on the outer wrap;
    //   replaying the original would shadow the new one)
    // - `Set-Cookie` (security: never reissue session/csrf cookies to a different caller, even if
    //   scoping should already prevent it).
    const STRIPPED: &[&str] = &[
        "connection",
        "transfer-encoding",
        "upgrade",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "trailer",
        "te",
        "x-request-id",
        "set-cookie",
    ];
    let mut out = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        if STRIPPED
            .iter()
            .any(|s| name.as_str().eq_ignore_ascii_case(s))
        {
            continue;
        }
        out.append(name.clone(), value.clone());
    }
    out
}

fn internal_error(detail: &str) -> Response {
    problem_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal Server Error",
        detail,
    )
}

fn bad_request(detail: &str) -> Response {
    problem_response(StatusCode::BAD_REQUEST, "Bad Request", detail)
}

fn unprocessable(detail: &str) -> Response {
    problem_response(
        StatusCode::UNPROCESSABLE_ENTITY,
        "Unprocessable Entity",
        detail,
    )
}

fn problem_response(status: StatusCode, title: &str, detail: &str) -> Response {
    // Hand-rolled application/problem+json body — `errors::ProblemDetails` is
    // out of this layer's owned-files scope and pulling it would couple the
    // middleware to a heavier serde structure. The shape matches RFC 9457
    // §3 well enough for clients that already speak the format.
    let body = format!(
        r#"{{"type":"about:blank","title":"{title}","status":{code},"detail":{detail_json}}}"#,
        code = status.as_u16(),
        detail_json = serde_json::Value::String(detail.to_owned()),
    );
    let mut response = Response::builder()
        .status(status)
        .body(Body::from(body))
        .unwrap_or_else(|_| status.into_response());
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/problem+json"),
    );
    response
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};

    use super::*;

    #[test]
    fn should_cache_includes_2xx_and_4xx_excludes_5xx() {
        assert!(should_cache(StatusCode::OK));
        assert!(should_cache(StatusCode::CREATED));
        assert!(should_cache(StatusCode::BAD_REQUEST));
        assert!(should_cache(StatusCode::CONFLICT));
        assert!(!should_cache(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(!should_cache(StatusCode::SERVICE_UNAVAILABLE));
    }

    #[test]
    fn filter_strips_set_cookie_and_request_id() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.insert("set-cookie", HeaderValue::from_static("sid=abc"));
        headers.insert("x-request-id", HeaderValue::from_static("req-1"));
        headers.insert("connection", HeaderValue::from_static("close"));

        let filtered = filter_response_headers(&headers);
        assert!(filtered.contains_key(header::CONTENT_TYPE));
        assert!(!filtered.contains_key("set-cookie"));
        assert!(!filtered.contains_key("x-request-id"));
        assert!(!filtered.contains_key("connection"));
    }
}
