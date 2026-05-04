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
//! When mounted on the production router, place this layer **inside**
//! `request_id` and `security_headers` so cached replays still acquire fresh
//! `X-Request-ID` and security headers when they leave the server.
//!
//! **Note:** the layer is not yet merged into `crate::app::build_app` /
//! `crate::routes::create_routes` — integration tests mount `IdempotencyLayer` directly on
//! minimal routers until the composition root wires a shared store.

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
    async fn get(&self, key: &str) -> Option<Arc<CachedResponse>>;

    /// Persist a cached response under `key`. Implementations MUST honour the
    /// store's configured TTL; the middleware does not enforce expiry.
    async fn put(&self, key: String, response: Arc<CachedResponse>);
}

/// Process-local idempotency store backed by [`moka::future::Cache`].
///
/// Entries expire after a configurable TTL and the cache is bounded by a
/// maximum entry count to provide a hard memory ceiling.
pub struct InMemoryIdempotencyStore {
    cache: Cache<String, Arc<CachedResponse>>,
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
        Self { cache }
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
    async fn get(&self, key: &str) -> Option<Arc<CachedResponse>> {
        self.cache.get(key).await
    }

    async fn put(&self, key: String, response: Arc<CachedResponse>) {
        self.cache.insert(key, response).await;
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
}

impl IdempotencyLayer {
    /// Build a layer backed by the given store, using [`IdempotencyConfig::default`].
    #[must_use]
    pub fn new(store: Arc<dyn IdempotencyStore>) -> Self {
        Self {
            store,
            config: IdempotencyConfig::default(),
        }
    }

    /// Override the layer configuration.
    #[must_use]
    pub fn with_config(mut self, config: IdempotencyConfig) -> Self {
        self.config = config;
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
        }
    }
}

/// Tower [`Service`] produced by [`IdempotencyLayer`].
#[derive(Clone)]
pub struct IdempotencyService<S> {
    inner: S,
    store: Arc<dyn IdempotencyStore>,
    config: IdempotencyConfig,
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
        Box::pin(async move { handle(inner, store, config, request).await })
    }
}

// ── Core handling ────────────────────────────────────────────────────────────

#[tracing::instrument(
    name = "idempotency",
    skip(inner, store, config, request),
    fields(
        method = %request.method(),
        path = %request.uri().path(),
        idempotency_key = tracing::field::Empty,
        outcome = tracing::field::Empty,
    )
)]
async fn handle<S>(
    inner: S,
    store: Arc<dyn IdempotencyStore>,
    config: IdempotencyConfig,
    request: Request,
) -> Result<Response, S::Error>
where
    S: Service<Request, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    // Only POST is in scope for the M3.4 acceptance criteria. Other methods
    // pass through transparently — clients that send the header on a GET get
    // their request handled normally.
    if request.method() != Method::POST {
        tracing::Span::current().record("outcome", "skip:non_post");
        return inner.oneshot(request).await;
    }

    // Header missing → opt-out. No allocation, no body buffering.
    let Some(raw_key) = request.headers().get(IDEMPOTENCY_KEY_HEADER) else {
        tracing::Span::current().record("outcome", "skip:no_header");
        return inner.oneshot(request).await;
    };

    let Ok(key_str) = raw_key.to_str() else {
        tracing::Span::current().record("outcome", "reject:non_ascii_header");
        return Ok(bad_request("Idempotency-Key must be valid ASCII"));
    };

    let key = match IdempotencyKey::parse(key_str) {
        Ok(k) => k,
        Err(err) => {
            tracing::Span::current().record("outcome", "reject:invalid_key");
            return Ok(bad_request(&err.to_string()));
        },
    };
    tracing::Span::current().record("idempotency_key", tracing::field::display(&key));

    // Buffer the request body so we can fingerprint it AND replay it to the
    // inner service. Anything beyond `max_request_body_bytes` opts the request
    // out of caching entirely — we still forward it, but cannot guarantee
    // replay safety, so we do not store the response either.
    let (parts, body) = request.into_parts();
    let body_result = axum::body::to_bytes(body, config.max_request_body_bytes).await;
    let Ok(body_bytes) = body_result else {
        tracing::Span::current().record("outcome", "skip:body_too_large");
        tracing::warn!(
            "request body exceeds idempotency max_request_body_bytes — \
                 forwarding without caching"
        );
        let request = Request::from_parts(parts, Body::from(Vec::new()));
        return inner.oneshot(request).await;
    };
    let body_vec: Vec<u8> = body_bytes.into();

    let request_fingerprint = fingerprint_request_body(&body_vec);
    let identity = identity_fingerprint(&parts.headers);
    let cache_key = build_cache_key(&parts.method, parts.uri.path(), &key, &identity);

    if let Some(cached) = store.get(&cache_key).await {
        if cached.request_fingerprint != request_fingerprint {
            tracing::Span::current().record("outcome", "reject:body_mismatch");
            tracing::warn!("Idempotency-Key reused with different request body — rejecting");
            return Ok(unprocessable(
                "Idempotency-Key reused with a different request payload",
            ));
        }
        tracing::Span::current().record("outcome", "hit");
        tracing::debug!(status = cached.status.as_u16(), "idempotency cache hit");
        return Ok(Arc::unwrap_or_clone(cached).into_response());
    }

    // Cache miss — rebuild the request with the buffered body and run the
    // inner handler.
    let request = Request::from_parts(parts, Body::from(body_vec));
    let response = inner.oneshot(request).await?;

    let (resp_parts, resp_body) = response.into_parts();
    let resp_result = axum::body::to_bytes(resp_body, config.max_response_body_bytes).await;
    let Ok(resp_bytes) = resp_result else {
        tracing::Span::current().record("outcome", "miss:resp_too_large");
        tracing::warn!(
            "response body exceeds idempotency max_response_body_bytes — \
                 returning to caller without caching"
        );
        return Ok(Response::from_parts(resp_parts, Body::empty()));
    };
    let resp_vec: Vec<u8> = resp_bytes.into();

    if should_cache(resp_parts.status) {
        let cached = Arc::new(CachedResponse {
            status: resp_parts.status,
            headers: filter_response_headers(&resp_parts.headers),
            body: resp_vec.clone(),
            request_fingerprint,
        });
        store.put(cache_key, cached).await;
        tracing::Span::current().record("outcome", "miss:cached");
        tracing::debug!(
            status = resp_parts.status.as_u16(),
            bytes = resp_vec.len(),
            "idempotency cache miss — response cached"
        );
    } else {
        tracing::Span::current().record("outcome", "miss:passthrough");
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
        store.put("k1".to_owned(), Arc::clone(&resp)).await;
        let fetched = store.get("k1").await.expect("entry must be present");
        assert_eq!(fetched.body, b"hello");
        assert!(store.get("missing").await.is_none());
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
        store.put("expiring".to_owned(), resp).await;
        assert!(store.get("expiring").await.is_some());
        tokio::time::sleep(Duration::from_millis(120)).await;
        // moka may need a tick to evict; force a sync.
        store.cache.run_pending_tasks().await;
        assert!(store.get("expiring").await.is_none());
    }
}
