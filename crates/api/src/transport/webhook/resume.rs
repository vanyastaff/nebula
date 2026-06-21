//! W-S3d — Webhook-callback resume producer.
//!
//! `POST /resume` is the ONLY attacker-reachable wait-state surface.
//! Design invariants (all enforced structurally, not by discipline):
//!
//! - **Scope-from-row**: no `TenantContext` extractor; the only scope in
//!   the enqueued `ControlMsg` is `row.scope`, read from the consumed
//!   token row.  A reviewer verifies by the ABSENCE of a tenant extractor.
//! - **Body inert**: the request body is capped and discarded; no field
//!   from it ever reaches the `ControlMsg`.
//! - **Uniform 404**: absent / expired / consumed / forged / wrong-kind →
//!   all return the byte-identical `ApiError::NotFound` ProblemDetails.
//!   No 401, no 410, no existence-revealing messages.
//! - **No bearer in logs/traces**: the bearer is hashed and dropped;
//!   only `execution_id`, `scope`, and `wait_kind` are logged on success.
//!
//! ## Pipeline order (mirrors `dispatch_inner` ordering in `dispatch.rs`)
//!
//! The burn (token DELETE) and the `Resume` enqueue happen in ONE transaction
//! ([`nebula_storage_port::store::ResumeProducer::consume_and_enqueue_resume`])
//! at step 10. Everything before it is non-destructive: a read-only `peek`
//! (step 6) supplies the row for the kind / expiry checks, so a wrong-kind or
//! expired token returns 404 WITHOUT burning the token (the prior
//! consume-first handler burned it first, then 404'd — a wart this fixes).
//!
//! 1. Body cap → 413
//! 2. Extract bearer from `Authorization: Bearer <token>` → uniform 404 on
//!    missing/wrong-scheme/empty (NOT 401 — do not reveal auth was attempted)
//! 3. Per-IP rate-limit → 429 + `Retry-After` (BEFORE any store hit)
//! 4. Global rate-limit → 429 (BEFORE any store hit)
//! 5. Hash bearer → `TokenHash` (bearer dropped here — never logged or stored)
//! 6. `producer.peek(hash)` (read-only, NO burn):
//!    - `Err` → 503 + `Retry-After` (token NOT burned; abuse-case 15)
//!    - `None` → uniform 404
//!    - `Some(row)` → continue
//! 7. Kind-match: `Webhook` → `ResumeTarget::Webhook{callback_id}`; `_` → 404
//!    (NO burn — wart fixed)
//! 8. Expiry check via injectable clock; expired or malformed → 404 (NO burn)
//! 9. Build the `ControlMsg` (scope FROM the row, never the request; command
//!    `Resume`; traceparent from request extensions)
//! 10. `producer.consume_and_enqueue_resume(hash, &msg)` — atomic burn+enqueue:
//!     - `Err` → 503 + `Retry-After` (tx rolled back; token LIVE; gap CLOSED)
//!     - `Ok(false)` → uniform 404 (raced / replayed)
//!     - `Ok(true)` → continue
//! 11. Per-tenant rate-limit (key = `row.scope.credential_owner_id()`) — fires
//!     ONLY on the atomic-delete winner, so a 429 is observable only on a
//!     request that ALSO burned the token (single-shot per token). Running it on
//!     the un-burned `peek` row would expose a repeatable 429 = "valid token +
//!     throttled tenant" oracle the consume-first model never did. → 429 (token
//!     already burned + `Resume` already enqueued) or 202.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{ConnectInfo, Extension, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use nebula_action::Clock;
use nebula_storage_port::dto::ResumeTarget;
use nebula_storage_port::dto::resume_token::{ResumeTokenWaitKind, TokenHash};
use tracing::{debug, warn};

use super::ratelimit::WebhookRateLimiter;
use super::token::token_hash;
use crate::error::ApiError;
use crate::middleware::InboundW3cTraceContext;
use crate::state::AppState;

/// Small body cap for `POST /resume`.
///
/// v1 has no meaningful request body — the resume intent is fully carried by the
/// bearer token.  Any body beyond this limit is rejected at two layers:
///
/// 1. The `axum::extract::DefaultBodyLimit::max` tower layer on the `/resume`
///    sub-router (enforced by axum BEFORE the body is buffered — prevents
///    large-body DoS from ever reaching the handler).
/// 2. The in-handler `body.len() > RESUME_BODY_LIMIT_BYTES` check (defense-in-depth
///    for callers that bypass the tower layer, e.g. `oneshot` tests that inject a
///    pre-built `Bytes` directly).
///
/// `pub(crate)` so `app.rs` can reference it when installing the layer.
pub(crate) const RESUME_BODY_LIMIT_BYTES: usize = 4 * 1024; // 4 KiB

/// Rate-limit defaults for the three `POST /resume` tiers.
///
/// Kept conservative relative to the webhook ingress defaults because
/// `/resume` tokens are per-execution (scarce) while webhook keys are
/// per-trigger (potentially high-fan-out).
const DEFAULT_RESUME_IP_RPM: u64 = 60;
const DEFAULT_RESUME_GLOBAL_RPM: u64 = 600;
const DEFAULT_RESUME_TENANT_RPM: u64 = 120;

/// Fixed key for the global (IP-agnostic) rate-limit bucket.
const RESUME_GLOBAL_RL_KEY: &str = "resume:global";

/// Shared components for the `POST /resume` handler.
///
/// Cloned cheaply (all internals behind `Arc`) and stored in the axum
/// `Router` state.
#[derive(Clone)]
pub struct ResumeHandlerComponents {
    /// Per-IP rate limiter — keyed on client IP.  Checked BEFORE any
    /// store hit (step 3).
    pub ip_rate_limiter: WebhookRateLimiter,
    /// Global backstop rate limiter — keyed on the fixed
    /// `RESUME_GLOBAL_RL_KEY` constant.  IP-trust-independent
    /// anti-enumeration backstop (step 4).
    pub global_rate_limiter: WebhookRateLimiter,
    /// Per-tenant rate limiter — keyed on `Scope::credential_owner_id()`,
    /// checked at step 11 ONLY on the atomic consume+enqueue winner.  Token is
    /// already burned at this point (and the `Resume` already enqueued); see the
    /// module doc for why this fires post-burn (oracle avoidance).
    pub tenant_rate_limiter: WebhookRateLimiter,
    /// Injectable clock — used for expiry comparison (step 8).
    pub clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for ResumeHandlerComponents {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResumeHandlerComponents")
            .field("ip_rate_limiter", &self.ip_rate_limiter)
            .field("global_rate_limiter", &self.global_rate_limiter)
            .field("tenant_rate_limiter", &self.tenant_rate_limiter)
            .field("clock", &"Arc<dyn Clock>")
            .finish()
    }
}

impl ResumeHandlerComponents {
    /// Construct with default RPM settings and a [`nebula_action::SystemClock`].
    ///
    /// Use this at production composition roots.  For tests inject a
    /// [`nebula_action::MockClock`] via [`Self::with_clock`].
    #[must_use]
    pub fn with_defaults() -> Self {
        Self {
            ip_rate_limiter: WebhookRateLimiter::new(DEFAULT_RESUME_IP_RPM),
            global_rate_limiter: WebhookRateLimiter::new(DEFAULT_RESUME_GLOBAL_RPM),
            tenant_rate_limiter: WebhookRateLimiter::new(DEFAULT_RESUME_TENANT_RPM),
            clock: Arc::new(nebula_action::SystemClock::new()),
        }
    }

    /// Override the clock with a test-injectable implementation.
    ///
    /// Intended for tests that must control expiry comparison.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }
}

/// Axum handler for `POST /resume`.
///
/// See module-level doc for the full pipeline order.
///
/// # Security notes
///
/// - No tenant extractor — scope comes from the consumed row only.
/// - Bearer token is hashed immediately; the plaintext never escapes this fn.
/// - All failure paths that could reveal token existence produce uniform 404.
/// - `traceparent` is forwarded to the `ControlMsg` for distributed trace
///   continuity; the bearer token is NEVER included in trace fields.
pub(crate) async fn resume_handler(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    // `Option<Extension<_>>` because `trace_context_middleware` may not be
    // present in all test harnesses (oneshot without the full middleware stack).
    w3c_trace: Option<Extension<InboundW3cTraceContext>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Components are None-gated at the call site (router mounting); by the
    // time this handler runs they are always Some.  The absence is a
    // composition-root bug surfaced at startup, not a per-request concern.
    let Some(components) = state.resume_handler_components.as_ref() else {
        warn!("resume_handler called without ResumeHandlerComponents wired — composition-root bug");
        return (StatusCode::SERVICE_UNAVAILABLE, "").into_response();
    };
    let Some(resume_producer) = state.resume_producer.as_ref() else {
        warn!("resume_handler called without resume_producer wired — composition-root bug");
        return (StatusCode::SERVICE_UNAVAILABLE, "").into_response();
    };

    // Step 1 — body cap.
    if body.len() > RESUME_BODY_LIMIT_BYTES {
        debug!(
            size = body.len(),
            cap = RESUME_BODY_LIMIT_BYTES,
            "resume: body exceeds cap"
        );
        return (StatusCode::PAYLOAD_TOO_LARGE, "").into_response();
    }
    // Body is dropped; it never reaches downstream logic.
    drop(body);

    // Step 2 — extract bearer from `Authorization: Bearer <token>`.
    // Missing header, wrong scheme, or empty token → uniform 404.
    // NOT 401 — do not reveal that authentication was even attempted.
    let bearer_token = match extract_bearer(&headers) {
        Some(t) => t,
        None => return uniform_not_found(),
    };

    // Step 3 — per-IP rate-limit BEFORE any store hit.
    let client_ip = peer_addr.ip().to_string();
    if let Err(exceeded) = components.ip_rate_limiter.check(&client_ip).await {
        debug!(
            ip = %client_ip,
            retry_after = exceeded.retry_after_secs,
            "resume: per-IP rate limit exceeded"
        );
        return rate_limit_429(exceeded.retry_after_secs);
    }

    // Step 4 — global rate-limit BEFORE any store hit.
    if let Err(exceeded) = components
        .global_rate_limiter
        .check(RESUME_GLOBAL_RL_KEY)
        .await
    {
        debug!(
            retry_after = exceeded.retry_after_secs,
            "resume: global rate limit exceeded"
        );
        return rate_limit_429(exceeded.retry_after_secs);
    }

    // Step 5 — hash the bearer.
    // bearer_token / hash deliberately excluded from all log fields.
    let raw_hash = token_hash(&bearer_token);
    // bearer_token is dropped here — never logged or stored past this point.
    drop(bearer_token);

    // SHA-256 always produces exactly 32 bytes; the `Err` arm is an
    // implementation-bug guard that can never fire in practice.
    let Ok(token_hash_value) = TokenHash::try_from_bytes(raw_hash.to_vec()) else {
        warn!("resume: SHA-256 produced unexpected hash length — implementation bug");
        return (StatusCode::INTERNAL_SERVER_ERROR, "").into_response();
    };

    // Step 6 — read-only `peek` (NO burn). The row drives the kind / expiry
    // checks below; the token is consumed only at step 10, atomically with the
    // enqueue.
    let token_row = match resume_producer.peek(&token_hash_value).await {
        // Storage error → 503 so the caller can retry; the token is NOT burned
        // on a transient storage fault (abuse-case 15).
        Err(storage_err) => {
            warn!(
                error = %storage_err,
                "resume: storage error on peek — returning 503 (token not burned)"
            );
            return service_unavailable_with_retry_after();
        },
        // Absent, forged, or already-consumed → uniform 404.
        Ok(None) => return uniform_not_found(),
        Ok(Some(row)) => row,
    };

    // Step 7 — kind-match (fail-closed, NO burn — wart fixed).
    // `ResumeTokenWaitKind` is `#[non_exhaustive]`; equality to `Webhook` is
    // the only admissible kind at this endpoint.  All other variants — Approval,
    // and any future variant added to the non-exhaustive enum without recompiling
    // this crate — fall to the `else` branch and return a uniform 404 WITHOUT
    // consuming the token (the prior consume-first handler burned it first).
    //
    // Ordering invariant: kind-match fires BEFORE the consume.  A wrong-kind or
    // expired token must return 404, never 429 — returning 429 would reveal that
    // the token was structurally valid (existence oracle).
    let resume_target = if token_row.wait_kind == ResumeTokenWaitKind::Webhook {
        ResumeTarget::Webhook {
            callback_id: token_row.callback_label.clone(),
        }
    } else {
        debug!(
            execution_id = %token_row.execution_id,
            wait_kind = ?token_row.wait_kind,
            "resume: token wait_kind does not match Webhook; returning 404 (fail-closed, no burn)"
        );
        return uniform_not_found();
    };

    // Step 8 — expiry check via injectable clock (fail-closed on parse failure).
    // Fires BEFORE the consume; expired / malformed → 404 WITHOUT burning.
    if let Some(expires_at_str) = token_row.expires_at.as_deref() {
        if let Ok(expiry) = parse_rfc3339_as_system_time(expires_at_str) {
            let now = components.clock.now();
            if now >= expiry {
                debug!(
                    execution_id = %token_row.execution_id,
                    expires_at = %expires_at_str,
                    "resume: token expired (no burn, no enqueue)"
                );
                return uniform_not_found();
            }
        } else {
            // Malformed RFC-3339 — fail-closed WITHOUT burning the token.
            warn!(
                execution_id = %token_row.execution_id,
                expires_at = %expires_at_str,
                "resume: malformed expires_at — fail-closed (no burn, no enqueue)"
            );
            return uniform_not_found();
        }
    }

    // Step 9 — build the `Resume` control message.
    // Scope comes FROM the row (never the request); `traceparent` from the
    // inbound W3C context (set by `trace_context_middleware` into request
    // extensions). The bearer token NEVER appears here.
    let traceparent = w3c_trace.map(|Extension(ctx)| ctx.0.traceparent().to_owned());
    let resume_msg = nebula_storage_port::dto::ControlMsg {
        id: *uuid::Uuid::new_v4().as_bytes(),
        execution_id: token_row.execution_id.clone(),
        command: nebula_storage_port::dto::ControlCommand::Resume,
        scope: token_row.scope.clone(),
        w3c_traceparent: traceparent,
        reclaim_count: 0,
        resume_target: Some(resume_target),
    };
    debug!(
        execution_id = %token_row.execution_id,
        scope = ?token_row.scope,
        has_traceparent = resume_msg.w3c_traceparent.is_some(),
        wait_kind = "Webhook",
        "resume: consuming token + enqueuing ControlCommand::Resume (atomic)"
    );

    // Step 10 — atomic burn + enqueue in ONE transaction. A transient fault
    // rolls back: the token stays live and no Resume is enqueued, so a retry
    // succeeds (the durability gap the consume-then-enqueue handler had).
    match resume_producer
        .consume_and_enqueue_resume(&token_hash_value, &resume_msg)
        .await
    {
        // Storage error → 503 (tx rolled back; token LIVE; caller retries).
        Err(storage_err) => {
            warn!(
                execution_id = %token_row.execution_id,
                error = %storage_err,
                "resume: storage error on consume_and_enqueue — 503 (tx rolled back; token live)"
            );
            return service_unavailable_with_retry_after();
        },
        // Zero rows deleted — raced or replayed between peek and consume.
        Ok(false) => return uniform_not_found(),
        // Won the atomic delete: token burned AND Resume enqueued.
        Ok(true) => {},
    }

    // Step 11 — per-tenant rate-limit, fired ONLY on the atomic-delete winner.
    // The token is already burned and the Resume already enqueued; a 429 is thus
    // observable only on a request that ALSO burned the token (single-shot per
    // token) — byte-identical to the prior post-burn semantics. Running this on
    // the un-burned `peek` row (step 6) would expose a repeatable 429 oracle
    // ("valid token + throttled tenant"); see the module doc.
    let tenant_key = token_row.scope.credential_owner_id();
    if let Err(exceeded) = components.tenant_rate_limiter.check(&tenant_key).await {
        debug!(
            tenant_id = %tenant_key,
            retry_after = exceeded.retry_after_secs,
            "resume: per-tenant rate limit exceeded (token already burned + enqueued)"
        );
        return rate_limit_429(exceeded.retry_after_secs);
    }

    (StatusCode::ACCEPTED, "").into_response()
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Extract a bearer token from the `Authorization` header.
///
/// Returns `None` for:
/// - Missing header
/// - Any scheme other than `Bearer` (case-insensitive)
/// - Empty token after stripping the scheme prefix
fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(axum::http::header::AUTHORIZATION)?;
    let header_str = value.to_str().ok()?;
    let token = header_str
        .strip_prefix("Bearer ")
        .or_else(|| header_str.strip_prefix("bearer "))
        .unwrap_or_else(|| {
            // Try case-insensitive split for other capitalizations.
            let lower = header_str.to_ascii_lowercase();
            if lower.starts_with("bearer ") {
                &header_str[7..]
            } else {
                ""
            }
        });
    if token.is_empty() {
        return None;
    }
    Some(token.to_owned())
}

/// Parse an RFC-3339 timestamp string to `SystemTime`.
///
/// Returns `Err(())` on any parse failure — callers must fail-closed.
fn parse_rfc3339_as_system_time(ts: &str) -> Result<std::time::SystemTime, ()> {
    // Use `humantime` which is already in the workspace via nebula-core.
    // Alternatively parse via chrono which is available workspace-wide.
    // We use chrono::DateTime::parse_from_rfc3339 → convert to SystemTime.
    use std::time::{Duration, UNIX_EPOCH};
    let parsed = chrono::DateTime::parse_from_rfc3339(ts).map_err(|_| ())?;
    let unix_secs = parsed.timestamp();
    let unix_nanos = parsed.timestamp_subsec_nanos();
    if unix_secs < 0 {
        return Err(());
    }
    let duration = Duration::new(unix_secs as u64, unix_nanos);
    Ok(UNIX_EPOCH + duration)
}

/// Uniform 404 response — byte-identical for all "not found" cases.
/// Absent / expired / consumed / forged / wrong-kind all produce this.
fn uniform_not_found() -> Response {
    ApiError::NotFound("not found".to_string()).into_response()
}

/// 429 response with `Retry-After` header (seconds as string).
fn rate_limit_429(retry_after_secs: u64) -> Response {
    let mut response = ApiError::RateLimitExceeded.into_response();
    if let Ok(value) = retry_after_secs.to_string().parse() {
        response.headers_mut().insert("retry-after", value);
    }
    response
}

/// 503 response with a fixed 60-second `Retry-After` hint (storage fault).
fn service_unavailable_with_retry_after() -> Response {
    let mut response =
        ApiError::ServiceUnavailable("resume token store unavailable; retry".to_string())
            .into_response();
    if let Ok(value) = "60".parse() {
        response.headers_mut().insert("retry-after", value);
    }
    response
}
