//! Idempotency-Key Middleware (M3.4).
//!
//! Generic POST-replay protection per the IETF draft
//! [draft-ietf-httpapi-idempotency-key]. A client supplies an `Idempotency-Key`
//! header with any state-changing request; the middleware caches the first
//! response and replays it byte-for-byte for subsequent requests carrying the
//! same key (within the configured TTL) — the inner handler runs **at most
//! once** per (method, path, key, identity, body) tuple.
//!
//! [draft-ietf-httpapi-idempotency-key]: https://datatracker.ietf.org/doc/draft-ietf-httpapi-idempotency-key/
//!
//! ## Cache key composition
//!
//! The lookup key combines:
//!
//! - HTTP method (uppercase ASCII)
//! - Request URI path (without query string)
//! - The validated `Idempotency-Key` header value
//! - A fingerprint of identity-bearing material (`Authorization`, `X-API-Key`, and the raw `Cookie`
//!   header when present so session-cookie auth does not collide across principals)
//!
//! Including the identity fingerprint is **mandatory** because this layer sits
//! in the outer middleware stack — it runs *before* `auth_middleware` resolves
//! the [`Principal`](nebula_core::scope::Principal). Two callers MUST never
//! share a cached response even if they happen to choose the same key.
//!
//! ## Body fingerprint
//!
//! Every cached entry also stores a SHA-256 fingerprint of the request body.
//! Reusing the same key with a different body yields **422 Unprocessable
//! Entity** per draft §2.5 — protects against client bugs that re-use keys
//! across logically distinct operations.
//!
//! ## What gets cached
//!
//! Only `2xx` and `4xx` responses. `5xx` responses are passed through
//! uncached so transient backend failures do not pin a permanent error for
//! the TTL window. Responses larger than
//! [`IdempotencyConfig::max_response_body_bytes`] are passed through
//! uncached as well.
//!
//! ## Position in the middleware stack
//!
//! [`crate::app::build_app`] mounts this layer on the API router **before**
//! merging the webhook transport — webhook ingress has its own dedup
//! contract (provider signature + replay-window timestamp) and never
//! carries `Idempotency-Key`. The mount sits **inside** `request_id` and
//! `security_headers` so cached replays still acquire fresh
//! `X-Request-ID` and security headers when they leave the server:
//!
//! ```text
//! outermost                                                            innermost
//!  rate_limit -> request_id -> security_headers -> middleware_stack -> idempotency -> routes
//!                                                                       (api only)
//! ```
//!
//! ## Backend selection
//!
//! [`InMemoryIdempotencyStore`] is the dev / single-process default;
//! [`StorageBackedIdempotencyStore`] adapts a layer-1
//! `nebula_storage::repos::IdempotencyStoreRepo` (PG-backed in
//! production deployments) onto this trait. Selection is driven by
//! `ApiConfig.idempotency.backend` per **ADR-0048**: the in-memory
//! backend loses dedup state across restart and across runners, so
//! production deployments running more than one API replica must select
//! `Postgres` to satisfy the §M3 1.0 closure criterion.

use std::{
    fmt,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use async_trait::async_trait;
use axum::{
    body::Body,
    extract::Request,
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures::future::BoxFuture;
use moka::future::Cache;
use nebula_metrics::{
    MetricsRegistry,
    naming::{
        NEBULA_API_IDEMPOTENCY_HITS_TOTAL, NEBULA_API_IDEMPOTENCY_LATENCY_MS,
        NEBULA_API_IDEMPOTENCY_MISSES_TOTAL, NEBULA_API_IDEMPOTENCY_REJECTS_TOTAL,
        NEBULA_API_IDEMPOTENCY_STORE_SATURATION_PPM, idempotency_reject_reason,
    },
};
use nebula_storage::repos::{CachedRecord, IdempotencyStoreRepo};
use sha2::{Digest, Sha256};
use tower::{Layer, Service, ServiceExt};

/// Header name carrying the client-supplied idempotency key.
pub const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

/// Default cache lifetime — 24h matches the draft RFC's recommendation.
pub const DEFAULT_TTL_SECS: u64 = 24 * 60 * 60;

/// Default cap on the number of cached responses.
pub const DEFAULT_MAX_ENTRIES: u64 = 10_000;

/// Default cap on the request/response body size eligible for caching (1 MiB).
pub const DEFAULT_MAX_BODY_BYTES: usize = 1024 * 1024;

/// Maximum acceptable length of an `Idempotency-Key` header value.
///
/// Picked to align with common gateway header limits and the draft RFC's
/// "opaque token, ≤ 255 octets" guidance.
pub const MAX_KEY_LEN: usize = 255;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors returned by [`IdempotencyStore`] operations.
///
/// Storage backends bubble these through the middleware. The handler
/// translates them to a 500 response — silently treating a `Decode`
/// error as a cache miss would drop replay protection on data
/// corruption (rejected by ADR-0048).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IdempotencyStoreError {
    /// The underlying repository returned an error (PG connection,
    /// constraint violation, etc.).
    #[error("idempotency-store backend failed: {0}")]
    Backend(String),

    /// The persisted record could not be decoded back into a typed
    /// `CachedResponse` (malformed status code, header bytes, etc.).
    /// Bubbles up as 500.
    #[error("idempotency-store decode failed: {0}")]
    Decode(String),
}

/// Errors returned when parsing/validating an `Idempotency-Key` header.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdempotencyKeyError {
    /// The header was present but contained no characters.
    #[error("Idempotency-Key header is empty")]
    Empty,

    /// The header value exceeded [`MAX_KEY_LEN`] octets.
    #[error("Idempotency-Key header exceeds max length of {MAX_KEY_LEN} bytes")]
    TooLong,

    /// The header contained bytes outside the printable-ASCII range.
    ///
    /// Restricting to printable ASCII keeps the cache key safe to render in
    /// logs and metrics labels without a separate encoding step.
    #[error("Idempotency-Key must be printable ASCII")]
    InvalidCharacters,
}

// ── Idempotency key newtype ──────────────────────────────────────────────────

/// Validated `Idempotency-Key` header value.
///
/// Construct via [`IdempotencyKey::parse`] — direct construction is forbidden
/// so the validation invariants below are guaranteed by the type:
///
/// - non-empty
/// - ≤ [`MAX_KEY_LEN`] bytes
/// - printable ASCII (`0x21..=0x7e`)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Parse a raw header value into a validated [`IdempotencyKey`].
    ///
    /// # Errors
    ///
    /// Returns [`IdempotencyKeyError`] if the value is empty, exceeds
    /// [`MAX_KEY_LEN`], or contains non-printable / non-ASCII bytes.
    pub fn parse(raw: &str) -> Result<Self, IdempotencyKeyError> {
        if raw.is_empty() {
            return Err(IdempotencyKeyError::Empty);
        }
        if raw.len() > MAX_KEY_LEN {
            return Err(IdempotencyKeyError::TooLong);
        }
        if !raw.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
            return Err(IdempotencyKeyError::InvalidCharacters);
        }
        Ok(Self(raw.to_owned()))
    }

    /// Borrow the validated key as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ── Cached response ──────────────────────────────────────────────────────────

/// Snapshot of an HTTP response retained for idempotent replay.
///
/// Stored behind an [`Arc`] inside the [`IdempotencyStore`] so concurrent
/// readers share the bytes without copying.
#[derive(Debug, Clone)]
pub struct CachedResponse {
    /// HTTP status code returned by the original handler.
    pub status: StatusCode,
    /// Filtered response headers (hop-by-hop and per-request headers stripped).
    pub headers: HeaderMap,
    /// Buffered response body bytes.
    pub body: Vec<u8>,
    /// SHA-256 fingerprint of the original request body — used to detect
    /// "same key, different body" reuse and reject it with 422 per draft §2.5.
    pub request_fingerprint: [u8; 32],
}

impl CachedResponse {
    fn into_response(self) -> Response {
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

/// Header set on replayed responses so callers can distinguish a cache hit
/// from a fresh handler invocation.
pub const IDEMPOTENT_REPLAY_HEADER: HeaderName = HeaderName::from_static("idempotent-replay");

// ── Store trait + in-memory impl ─────────────────────────────────────────────

/// Storage backend for cached idempotent responses.
///
/// Implementations MUST be safe to share across async tasks. The default
/// implementation is [`InMemoryIdempotencyStore`]; a Redis / SQL backend can
/// be slotted in by implementing this trait without touching the middleware.
#[async_trait]
pub trait IdempotencyStore: Send + Sync + fmt::Debug {
    /// Look up a cached response by its scoped cache key.
    ///
    /// Returns `Ok(None)` for a cache miss, `Ok(Some(_))` for a hit, or
    /// [`IdempotencyStoreError`] when the read itself failed (backend
    /// connection error or decode failure). The middleware translates
    /// errors to 500 — see ADR-0048: silent cache-miss fallback would
    /// drop replay protection on data corruption.
    async fn get(&self, key: &str) -> Result<Option<Arc<CachedResponse>>, IdempotencyStoreError>;

    /// Persist a cached response under `key`. Implementations MUST honour the
    /// store's configured TTL; the middleware does not enforce expiry.
    ///
    /// `put` errors do not surface to the caller — the response has
    /// already been computed by the inner handler and is returned
    /// unchanged; the middleware logs a warning and skips caching for
    /// this key.
    async fn put(
        &self,
        key: String,
        response: Arc<CachedResponse>,
    ) -> Result<(), IdempotencyStoreError>;

    /// Stable, log-safe identifier for the concrete impl backing this store.
    ///
    /// Used by `build_app` to log which backend was wired at startup
    /// (`"in-memory"`, `"postgres"`) without leaking implementation types
    /// like `std::any::type_name_of_val(...)` would. Keep the value short
    /// and ASCII so it composes cleanly into `tracing::info!` events and
    /// metric labels (low-cardinality).
    fn store_kind(&self) -> &'static str;

    /// Live store saturation as `entries / max_capacity`, scaled by
    /// 1_000_000 (parts per million).
    ///
    /// Returns `None` for backends without a fixed capacity (PG-backed
    /// store keyed by `expires_at` rather than a row cap). The middleware
    /// publishes the value into the
    /// [`NEBULA_API_IDEMPOTENCY_STORE_SATURATION_PPM`] gauge after every
    /// successful `put`, so dashboards see a steady proxy for
    /// "how full is the dedup store right now". Default impl returns
    /// `None` so unbounded backends opt out without ceremony.
    fn saturation_ppm(&self) -> Option<u64> {
        None
    }
}

/// Process-local idempotency store backed by [`moka::future::Cache`].
///
/// Entries expire after a configurable TTL and the cache is bounded by a
/// maximum entry count to provide a hard memory ceiling.
pub struct InMemoryIdempotencyStore {
    cache: Cache<String, Arc<CachedResponse>>,
    max_entries: u64,
}

impl InMemoryIdempotencyStore {
    /// Create a store with the default TTL ([`DEFAULT_TTL_SECS`]) and capacity
    /// ([`DEFAULT_MAX_ENTRIES`]).
    #[must_use]
    pub fn new() -> Self {
        Self::with_ttl_and_capacity(Duration::from_secs(DEFAULT_TTL_SECS), DEFAULT_MAX_ENTRIES)
    }

    /// Create a store with a custom TTL and capacity.
    #[must_use]
    pub fn with_ttl_and_capacity(ttl: Duration, max_entries: u64) -> Self {
        let cache = Cache::builder()
            .time_to_live(ttl)
            .max_capacity(max_entries)
            .build();
        Self { cache, max_entries }
    }
}

impl Default for InMemoryIdempotencyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for InMemoryIdempotencyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InMemoryIdempotencyStore")
            .field("entries", &self.cache.entry_count())
            .finish()
    }
}

#[async_trait]
impl IdempotencyStore for InMemoryIdempotencyStore {
    async fn get(&self, key: &str) -> Result<Option<Arc<CachedResponse>>, IdempotencyStoreError> {
        Ok(self.cache.get(key).await)
    }

    async fn put(
        &self,
        key: String,
        response: Arc<CachedResponse>,
    ) -> Result<(), IdempotencyStoreError> {
        self.cache.insert(key, response).await;
        Ok(())
    }

    fn store_kind(&self) -> &'static str {
        "in-memory"
    }

    fn saturation_ppm(&self) -> Option<u64> {
        if self.max_entries == 0 {
            return None;
        }
        // `entry_count` is `u64`; multiply first, then divide, capping at
        // 1_000_000 to keep the gauge inside the `0..=1_000_000` ppm range
        // even if `moka` over-counts under contention (it is documented as
        // a best-effort approximation).
        let ppm = self
            .cache
            .entry_count()
            .saturating_mul(1_000_000)
            .saturating_div(self.max_entries);
        Some(ppm.min(1_000_000))
    }
}

// ── Storage-backed bridge (Layer-1 repo → IdempotencyStore) ──────────────────

/// Bridge that adapts a layer-1 [`IdempotencyStoreRepo`] (in
/// `nebula-storage`) onto the API-side [`IdempotencyStore`] contract.
///
/// The repo speaks `CachedRecord` (status `u16`, headers as
/// `Vec<(String, Vec<u8>)>`) so the storage layer stays free of `http`
/// types. This bridge does the bidirectional translation:
/// `CachedRecord ↔ CachedResponse` (`StatusCode` / `HeaderMap`). Decode
/// failures map to [`IdempotencyStoreError::Decode`] and bubble up to
/// the middleware as 500 — silently treating them as cache misses
/// would drop replay protection on data corruption (ADR-0048).
pub struct StorageBackedIdempotencyStore<R: IdempotencyStoreRepo> {
    repo: Arc<R>,
    ttl: Duration,
}

impl<R: IdempotencyStoreRepo> StorageBackedIdempotencyStore<R> {
    /// Construct a bridge with the given TTL applied to every `put`.
    pub fn new(repo: Arc<R>, ttl: Duration) -> Self {
        Self { repo, ttl }
    }

    /// Borrow the underlying repo for sweep / introspection.
    pub fn repo(&self) -> &Arc<R> {
        &self.repo
    }
}

impl<R: IdempotencyStoreRepo> fmt::Debug for StorageBackedIdempotencyStore<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorageBackedIdempotencyStore")
            .field("ttl_secs", &self.ttl.as_secs())
            .finish()
    }
}

fn record_to_response(record: CachedRecord) -> Result<CachedResponse, IdempotencyStoreError> {
    let status = StatusCode::from_u16(record.status).map_err(|err| {
        IdempotencyStoreError::Decode(format!("status code {}: {err}", record.status))
    })?;
    let mut headers = HeaderMap::with_capacity(record.headers.len());
    for (name, value) in record.headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|err| IdempotencyStoreError::Decode(format!("header name {name:?}: {err}")))?;
        let header_value = HeaderValue::from_bytes(&value).map_err(|err| {
            IdempotencyStoreError::Decode(format!("header value for {name:?}: {err}"))
        })?;
        headers.append(header_name, header_value);
    }
    Ok(CachedResponse {
        status,
        headers,
        body: record.body,
        request_fingerprint: record.fingerprint,
    })
}

fn response_to_record(response: &CachedResponse) -> CachedRecord {
    let mut headers: Vec<(String, Vec<u8>)> = Vec::with_capacity(response.headers.len());
    for (name, value) in &response.headers {
        headers.push((name.as_str().to_owned(), value.as_bytes().to_vec()));
    }
    CachedRecord {
        status: response.status.as_u16(),
        headers,
        body: response.body.clone(),
        fingerprint: response.request_fingerprint,
    }
}

#[async_trait]
impl<R: IdempotencyStoreRepo + 'static> IdempotencyStore for StorageBackedIdempotencyStore<R> {
    async fn get(&self, key: &str) -> Result<Option<Arc<CachedResponse>>, IdempotencyStoreError> {
        let record = self
            .repo
            .get(key)
            .await
            .map_err(|err| IdempotencyStoreError::Backend(err.to_string()))?;
        match record {
            None => Ok(None),
            Some(record) => Ok(Some(Arc::new(record_to_response(record)?))),
        }
    }

    async fn put(
        &self,
        key: String,
        response: Arc<CachedResponse>,
    ) -> Result<(), IdempotencyStoreError> {
        let record = response_to_record(&response);
        self.repo
            .put(key, record, self.ttl)
            .await
            .map_err(|err| IdempotencyStoreError::Backend(err.to_string()))
    }

    fn store_kind(&self) -> &'static str {
        // The concrete kind depends on R; static dispatch through
        // `R::store_kind` would require adding a method to the repo
        // trait. Operators read backend selection from
        // `API_IDEMPOTENCY_BACKEND` instead. Returning a generic label
        // here keeps the cardinality low without leaking R's typename.
        "storage-backed"
    }
}

// ── Configuration ────────────────────────────────────────────────────────────

/// Tunables for the idempotency middleware.
#[derive(Debug, Clone)]
pub struct IdempotencyConfig {
    /// Maximum buffered request body size (in bytes) eligible for caching.
    /// Requests larger than this pass through without idempotency tracking.
    pub max_request_body_bytes: usize,
    /// Maximum buffered response body size (in bytes) eligible for caching.
    /// Larger responses are returned to the caller but never cached.
    pub max_response_body_bytes: usize,
}

impl Default for IdempotencyConfig {
    fn default() -> Self {
        Self {
            max_request_body_bytes: DEFAULT_MAX_BODY_BYTES,
            max_response_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }
}

// ── Layer + Service ──────────────────────────────────────────────────────────

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

fn fingerprint_request_body(body: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(body);
    hasher.finalize().into()
}

fn identity_fingerprint(headers: &HeaderMap) -> [u8; 32] {
    // Mix in any material that distinguishes callers before `auth_middleware`
    // runs (`Authorization`, `X-API-Key`, raw `Cookie` for session flows).
    // Order is fixed so the hash is stable across requests. Missing headers
    // contribute an empty segment — the resulting hash still differs from "no
    // headers at all" because the segment separators stay in the input.
    let mut hasher = Sha256::new();
    let auth = headers
        .get(header::AUTHORIZATION)
        .map(HeaderValue::as_bytes)
        .unwrap_or_default();
    let api_key = headers
        .get("x-api-key")
        .map(HeaderValue::as_bytes)
        .unwrap_or_default();
    let cookie = headers
        .get(header::COOKIE)
        .map(HeaderValue::as_bytes)
        .unwrap_or_default();
    hasher.update(b"authorization=");
    hasher.update(auth);
    hasher.update(b"\nx-api-key=");
    hasher.update(api_key);
    hasher.update(b"\ncookie=");
    hasher.update(cookie);
    hasher.finalize().into()
}

fn build_cache_key(
    method: &Method,
    path: &str,
    key: &IdempotencyKey,
    identity: &[u8; 32],
) -> String {
    // Hex-encode the identity fingerprint inline rather than pull `hex` as a
    // direct dep — `format!` keeps the helper allocation-light enough for the
    // hot path (single `String`) and avoids an extra crate edge.
    let mut identity_hex = String::with_capacity(identity.len() * 2);
    for byte in identity {
        identity_hex.push_str(&format!("{byte:02x}"));
    }
    format!("{method}|{path}|{key}|{identity_hex}")
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
    use super::*;

    #[test]
    fn parse_rejects_empty_key() {
        assert_eq!(IdempotencyKey::parse(""), Err(IdempotencyKeyError::Empty));
    }

    #[test]
    fn parse_rejects_oversized_key() {
        let oversized = "a".repeat(MAX_KEY_LEN + 1);
        assert_eq!(
            IdempotencyKey::parse(&oversized),
            Err(IdempotencyKeyError::TooLong)
        );
    }

    #[test]
    fn parse_rejects_non_ascii() {
        assert_eq!(
            IdempotencyKey::parse("kéy"),
            Err(IdempotencyKeyError::InvalidCharacters)
        );
    }

    #[test]
    fn parse_rejects_whitespace_and_control() {
        assert_eq!(
            IdempotencyKey::parse("a b"),
            Err(IdempotencyKeyError::InvalidCharacters),
            "spaces are outside printable-ASCII range",
        );
        assert_eq!(
            IdempotencyKey::parse("a\tb"),
            Err(IdempotencyKeyError::InvalidCharacters),
        );
    }

    #[test]
    fn parse_accepts_typical_uuid() {
        let key = IdempotencyKey::parse("3a82d4c4-78c9-4e7f-9bcf-1e7d80e9f4b1")
            .expect("uuid string is a valid key");
        assert_eq!(key.as_str(), "3a82d4c4-78c9-4e7f-9bcf-1e7d80e9f4b1");
    }

    #[test]
    fn cache_key_includes_identity_so_callers_cannot_share() {
        let key = IdempotencyKey::parse("k1").unwrap();
        let mut h1 = HeaderMap::new();
        h1.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer A"));
        let mut h2 = HeaderMap::new();
        h2.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer B"));

        let id1 = identity_fingerprint(&h1);
        let id2 = identity_fingerprint(&h2);
        assert_ne!(
            id1, id2,
            "different bearer tokens MUST yield different scopes"
        );

        let ck1 = build_cache_key(&Method::POST, "/x", &key, &id1);
        let ck2 = build_cache_key(&Method::POST, "/x", &key, &id2);
        assert_ne!(ck1, ck2);
    }

    #[test]
    fn fingerprint_is_stable_and_distinguishes_payloads() {
        let a = fingerprint_request_body(b"payload-a");
        let a_again = fingerprint_request_body(b"payload-a");
        let b = fingerprint_request_body(b"payload-b");
        assert_eq!(a, a_again);
        assert_ne!(a, b);
    }

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

    #[tokio::test]
    async fn in_memory_store_round_trip() {
        let store = InMemoryIdempotencyStore::new();
        let resp = Arc::new(CachedResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: b"hello".to_vec(),
            request_fingerprint: [0u8; 32],
        });
        store
            .put("k1".to_owned(), Arc::clone(&resp))
            .await
            .expect("in-memory put never errors");
        let fetched = store
            .get("k1")
            .await
            .expect("get must not error")
            .expect("entry must be present");
        assert_eq!(fetched.body, b"hello");
        assert!(
            store
                .get("missing")
                .await
                .expect("get must not error")
                .is_none()
        );
    }

    #[tokio::test]
    async fn in_memory_store_honours_ttl() {
        let store = InMemoryIdempotencyStore::with_ttl_and_capacity(Duration::from_millis(50), 16);
        let resp = Arc::new(CachedResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Vec::new(),
            request_fingerprint: [0u8; 32],
        });
        store.put("expiring".to_owned(), resp).await.expect("put");
        assert!(store.get("expiring").await.expect("get").is_some());
        tokio::time::sleep(Duration::from_millis(120)).await;
        // moka may need a tick to evict; force a sync.
        store.cache.run_pending_tasks().await;
        assert!(store.get("expiring").await.expect("get").is_none());
    }
}
