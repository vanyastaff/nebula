//! Idempotency store trait, store-side error type, and the PG-backed bridge.
//!
//! [`IdempotencyStore`] is the persistence port for the middleware: any
//! backend (in-memory, PostgreSQL, Redis, …) satisfies the two-method
//! `get` / `put` contract. [`StorageBackedIdempotencyStore`] adapts the
//! `nebula-storage` [`IdempotencyStoreRepo`] onto this trait; it lives
//! here because it is the "storage-backed" half of the persistence port,
//! not an independent backend with its own concern.

use std::{fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use nebula_storage::repos::{CachedRecord, IdempotencyStoreRepo};

// ── Error ────────────────────────────────────────────────────────────────────

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

// ── Store trait ──────────────────────────────────────────────────────────────

/// Storage backend for cached idempotent responses.
///
/// Implementations MUST be safe to share across async tasks. The default
/// implementation is [`super::memory::InMemoryIdempotencyStore`]; a Redis / SQL
/// backend can be slotted in by implementing this trait without touching the
/// middleware.
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
    /// [`NEBULA_API_IDEMPOTENCY_STORE_SATURATION_PPM`](nebula_metrics::naming::NEBULA_API_IDEMPOTENCY_STORE_SATURATION_PPM)
    /// gauge after every successful `put`, so dashboards see a steady proxy for
    /// "how full is the dedup store right now". Default impl returns
    /// `None` so unbounded backends opt out without ceremony.
    fn saturation_ppm(&self) -> Option<u64> {
        None
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
