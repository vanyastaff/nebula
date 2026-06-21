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
//! 1. Body cap → 413
//! 2. Extract bearer from `Authorization: Bearer <token>` → uniform 404 on
//!    missing/wrong-scheme/empty (NOT 401 — do not reveal auth was attempted)
//! 3. Per-IP rate-limit → 429 + `Retry-After` (BEFORE any store hit)
//! 4. Global rate-limit → 429 (BEFORE any store hit)
//! 5. Hash bearer; `store.consume(hash)`:
//!    - `Err` → 503 + `Retry-After` (token unconsumed; abuse-case 15)
//!    - `Ok(None)` → uniform 404
//!    - `Ok(Some(row))` → continue
//! 6. Per-tenant rate-limit (key = `row.scope.credential_owner_id()`) → 429;
//!    token is already burned (accepted, documented)
//! 7. Kind-match: `Webhook` → `ResumeTarget::Webhook{callback_id}`; `_` → 404
//! 8. Expiry check via injectable clock; expired or malformed → 404, no enqueue
//! 9. Enqueue `ControlCommand::Resume` via `enqueue_resume_from_row` → 202 / 503

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{ConnectInfo, State},
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
use crate::state::AppState;

/// Small body cap for `POST /resume`.
///
/// v1 has no meaningful request body — the resume intent is fully carried
/// by the bearer token.  Any body beyond this limit is rejected 413; the
/// cap prevents a large body from consuming read buffers.
const RESUME_BODY_LIMIT_BYTES: usize = 4 * 1024; // 4 KiB

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
    /// checked post-consume (step 6).  Token is already burned at this
    /// point; see W-S3d spec note.
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
pub(crate) async fn resume_handler(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
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
    let Some(token_store) = state.resume_token_store.as_ref() else {
        warn!("resume_handler called without resume_token_store wired — composition-root bug");
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

    // Step 5 — hash and consume.
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

    let consumed_row = match token_store.consume(&token_hash_value).await {
        // Storage error → 503 so the caller can retry; the token is
        // unconsumed on a transient storage fault (abuse-case 15).
        Err(storage_err) => {
            warn!(
                error = %storage_err,
                "resume: storage error on consume — returning 503 (token unconsumed)"
            );
            return service_unavailable_with_retry_after();
        },
        // Absent, forged, or already-consumed → uniform 404.
        Ok(None) => return uniform_not_found(),
        Ok(Some(row)) => row,
    };

    // Step 6 — per-tenant rate-limit (post-consume; token already burned).
    let tenant_key = consumed_row.scope.credential_owner_id();
    if let Err(exceeded) = components.tenant_rate_limiter.check(&tenant_key).await {
        debug!(
            tenant_id = %tenant_key,
            retry_after = exceeded.retry_after_secs,
            "resume: per-tenant rate limit exceeded (token consumed)"
        );
        return rate_limit_429(exceeded.retry_after_secs);
    }

    // Step 7 — kind-match (fail-closed).
    // `ResumeTokenWaitKind` is `#[non_exhaustive]`; equality to `Webhook` is
    // the only admissible kind at this endpoint.  All other variants — Approval,
    // and any future variant added to the non-exhaustive enum without recompiling
    // this crate — fall to the `else` branch and return a uniform 404.
    let resume_target = if consumed_row.wait_kind == ResumeTokenWaitKind::Webhook {
        ResumeTarget::Webhook {
            callback_id: consumed_row.callback_label.clone(),
        }
    } else {
        debug!(
            execution_id = %consumed_row.execution_id,
            wait_kind = ?consumed_row.wait_kind,
            "resume: token wait_kind does not match Webhook; returning 404 (fail-closed)"
        );
        return uniform_not_found();
    };

    // Step 8 — expiry check via injectable clock (fail-closed on parse failure).
    if let Some(expires_at_str) = consumed_row.expires_at.as_deref() {
        if let Ok(expiry) = parse_rfc3339_as_system_time(expires_at_str) {
            let now = components.clock.now();
            if now >= expiry {
                debug!(
                    execution_id = %consumed_row.execution_id,
                    expires_at = %expires_at_str,
                    "resume: token expired (consumed, no enqueue)"
                );
                return uniform_not_found();
            }
        } else {
            // Malformed RFC-3339 — fail-closed; token is already consumed.
            warn!(
                execution_id = %consumed_row.execution_id,
                expires_at = %expires_at_str,
                "resume: malformed expires_at — fail-closed (token consumed, no enqueue)"
            );
            return uniform_not_found();
        }
    }

    // Step 9 — enqueue.
    // Log only non-secret fields; bearer/hash never appear here.
    debug!(
        execution_id = %consumed_row.execution_id,
        scope = ?consumed_row.scope,
        wait_kind = "Webhook",
        "resume: enqueuing ControlCommand::Resume"
    );

    match state
        .enqueue_resume_from_row(&consumed_row, resume_target)
        .await
    {
        Ok(()) => (StatusCode::ACCEPTED, "").into_response(),
        Err(api_err) => {
            warn!(
                execution_id = %consumed_row.execution_id,
                error = %api_err,
                "resume: failed to enqueue ControlCommand::Resume"
            );
            match api_err {
                ApiError::ServiceUnavailable(_) => service_unavailable_with_retry_after(),
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "").into_response(),
            }
        },
    }
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
