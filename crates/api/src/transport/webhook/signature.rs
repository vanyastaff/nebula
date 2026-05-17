//! ADR-0022 signature-policy enforcement for the webhook transport.
//!
//! Translates a [`SignaturePolicy`] + [`WebhookRequest`] pair into a
//! [`SignatureVerdict`] that the dispatch pipeline maps to HTTP status
//! codes:
//!
//! | Verdict           | HTTP |
//! |-------------------|------|
//! | `Pass`            | 2xx (dispatch continues) |
//! | `MissingSecret`   | 500 (operator misconfiguration) |
//! | `Fail(reason)`    | 401 `application/problem+json` |
//!
//! Metric: [`nebula_metrics::NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL`]
//! with `reason ∈ {missing, invalid, missing_secret}` (+ timestamp
//! variants); only recorded when a [`MetricsRegistry`] is attached.
//!
//! Replay-window failures additionally bump
//! [`nebula_metrics::NEBULA_WEBHOOK_REPLAY_REJECTIONS_TOTAL`] via
//! [`super::replay::replay_reason_for`].

use std::sync::Arc;

use axum::{
    Json,
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use nebula_action::{Clock, SignatureError, SignatureOutcome, SignaturePolicy, WebhookRequest};
use nebula_metrics::{
    MetricsRegistry, NEBULA_WEBHOOK_REPLAY_REJECTIONS_TOTAL,
    NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL, webhook_signature_failure_reason,
};
use tracing::debug;

use super::replay::replay_reason_for;
use crate::error::ProblemDetails;

/// Result of transport-layer signature enforcement.
///
/// Internal type — the handler turns it into a 2xx pass-through or a
/// problem+json 4xx/5xx response.
pub(super) enum SignatureVerdict {
    /// Either the policy is `OptionalAcceptUnsigned` or the configured
    /// verifier returned `Valid`. Request dispatch continues.
    Pass,
    /// `Required` policy held an empty secret — no signature can be
    /// verified. Returns 500 (our misconfiguration, not the caller's).
    MissingSecret,
    /// Signature header absent (`missing`) or present-but-mismatched
    /// (`invalid`). Returns 401.
    Fail(&'static str),
}

/// Run the configured signature check against the request.
///
/// Any non-`Valid` outcome from a `Required` or `Custom` policy is a
/// 401; only an empty secret under `Required` is a 500. An `ActionError`
/// surfaced by the primitives (bad header name, empty secret before
/// the empty-secret check) is treated as `Invalid` — a 401 — rather
/// than a 500, because the payload came from an external caller and
/// the specific error is not the operator's to debug.
pub(super) fn enforce_signature(
    policy: &SignaturePolicy,
    request: &WebhookRequest,
    clock: &dyn Clock,
) -> SignatureVerdict {
    match policy {
        SignaturePolicy::OptionalAcceptUnsigned => SignatureVerdict::Pass,
        SignaturePolicy::Required(req) => match req.verify_with(request, clock) {
            Ok(()) => SignatureVerdict::Pass,
            Err(SignatureError::SecretMissing) => SignatureVerdict::MissingSecret,
            Err(SignatureError::SignatureMissing) => {
                SignatureVerdict::Fail(webhook_signature_failure_reason::MISSING)
            },
            Err(SignatureError::SignatureInvalid) => {
                SignatureVerdict::Fail(webhook_signature_failure_reason::INVALID)
            },
            Err(SignatureError::TimestampMissing) => {
                SignatureVerdict::Fail(webhook_signature_failure_reason::TIMESTAMP_MISSING)
            },
            Err(SignatureError::TimestampMalformed { reason }) => {
                debug!(reason = %reason, "webhook timestamp malformed");
                SignatureVerdict::Fail(webhook_signature_failure_reason::TIMESTAMP_MALFORMED)
            },
            Err(SignatureError::TimestampOutOfWindow { skew_secs }) => {
                debug!(skew_secs, "webhook timestamp outside replay window");
                SignatureVerdict::Fail(webhook_signature_failure_reason::TIMESTAMP_OUT_OF_WINDOW)
            },
            // `SignatureError` is `#[non_exhaustive]` — fail-closed
            // on any future variant.
            Err(_) => SignatureVerdict::Fail(webhook_signature_failure_reason::INVALID),
        },
        SignaturePolicy::Custom(verifier) => outcome_to_verdict(verifier(request)),
    }
}

pub(super) fn outcome_to_verdict(outcome: SignatureOutcome) -> SignatureVerdict {
    match outcome {
        SignatureOutcome::Valid => SignatureVerdict::Pass,
        SignatureOutcome::Missing => {
            SignatureVerdict::Fail(webhook_signature_failure_reason::MISSING)
        },
        SignatureOutcome::Invalid => {
            SignatureVerdict::Fail(webhook_signature_failure_reason::INVALID)
        },
        // `SignatureOutcome` is `#[non_exhaustive]`; any future variant
        // is fail-closed.
        _ => SignatureVerdict::Fail(webhook_signature_failure_reason::INVALID),
    }
}

/// Record a signature-failure metric (ADR-0022).
///
/// Replay-window failures additionally bump
/// [`NEBULA_WEBHOOK_REPLAY_REJECTIONS_TOTAL`] so dashboards can
/// isolate them from generic signature mismatches without scraping
/// the `reason` label.
pub(super) fn record_signature_failure(
    metrics: &Option<Arc<MetricsRegistry>>,
    reason: &'static str,
) {
    let Some(reg) = metrics else {
        return;
    };
    let labels = reg.interner().single("reason", reason);
    if let Ok(c) = reg.counter_labeled(NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL, &labels) {
        c.inc();
    }
    if let Some(replay_reason) = replay_reason_for(reason) {
        let labels = reg.interner().single("reason", replay_reason);
        if let Ok(c) = reg.counter_labeled(NEBULA_WEBHOOK_REPLAY_REJECTIONS_TOTAL, &labels) {
            c.inc();
        }
    }
}

/// Static `application/problem+json` content-type header value.
///
/// Constant-constructed at compile time via [`HeaderValue::from_static`] —
/// no runtime `.parse().expect(...)` panic path. RFC 9457 normalizes the
/// exact token, so this is the canonical value for every problem+json
/// response the transport emits.
pub(super) const PROBLEM_JSON_CONTENT_TYPE: HeaderValue =
    HeaderValue::from_static("application/problem+json");

/// Shared assembly for `application/problem+json` responses returned
/// from the webhook transport. Matches the convention in
/// [`crate::error::ApiError::into_response`].
pub(super) fn problem_response(status: StatusCode, problem: ProblemDetails) -> Response {
    let mut resp = (status, Json(problem)).into_response();
    resp.headers_mut()
        .insert(axum::http::header::CONTENT_TYPE, PROBLEM_JSON_CONTENT_TYPE);
    resp
}

/// 401 response for a signature mismatch. RFC 9457 `application/problem+json`
/// to match the rest of the API surface.
pub(super) fn signature_rejected_response(instance_path: &str, reason: &'static str) -> Response {
    let detail = match reason {
        r if r == webhook_signature_failure_reason::MISSING => {
            "webhook signature header missing".to_string()
        },
        r if r == webhook_signature_failure_reason::INVALID => {
            "webhook signature invalid".to_string()
        },
        other => format!("webhook signature rejected: {other}"),
    };
    let problem = ProblemDetails::new(
        "https://nebula.dev/problems/webhook-signature",
        "Webhook Signature Rejected",
        StatusCode::UNAUTHORIZED,
    )
    .with_detail(detail)
    .with_instance(instance_path.to_string());
    problem_response(StatusCode::UNAUTHORIZED, problem)
}

/// 500 response for an action that shipped `Required` without a secret.
/// This is fail-closed behaviour — the author's misconfiguration
/// surfaces as a server error so it shows up in dashboards rather than
/// silently accepting unsigned requests.
pub(super) fn missing_secret_response(instance_path: &str) -> Response {
    let problem = ProblemDetails::new(
        "https://nebula.dev/problems/webhook-signature-misconfigured",
        "Webhook Signature Secret Not Configured",
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .with_detail(
        "webhook action declared SignaturePolicy::Required but no HMAC secret is configured; \
         supply one via WebhookConfig::with_signature_policy + \
         SignaturePolicy::required_hmac_sha256, or explicitly opt out with \
         SignaturePolicy::OptionalAcceptUnsigned"
            .to_string(),
    )
    .with_instance(instance_path.to_string());
    problem_response(StatusCode::INTERNAL_SERVER_ERROR, problem)
}
