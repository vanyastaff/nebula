//! Webhook dispatch pipeline (webhook activation).
//!
//! Contains the shared `dispatch_inner` pipeline for both the
//! programmatic and slug-routed webhook surfaces, plus the two axum
//! handler functions that build a [`super::key::WebhookKey`] and
//! delegate into it.
//!
//! ## Pipeline order (webhook activation single-pipe)
//!
//! 1. Body size check → 413
//! 2. Route lookup → 404 (before rate-limit so unregistered keys
//!    never touch the limiter — #271 follow-up)
//! 3. Rate-limit by key → 429 + `Retry-After`
//! 4. Construct [`WebhookRequest`] → 400 / 413
//! 5. Signature enforcement ([`super::signature::enforce_signature`]) → 401 / 500
//! 6. Dispatch via [`TriggerHandler::handle_event`] with timeout → 504 / 500 / handler response

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use nebula_action::{TriggerEvent, WebhookHttpResponse, WebhookRequest};
use nebula_metrics::{
    NEBULA_WEBHOOK_RATE_LIMIT_REJECTIONS_TOTAL, webhook_key_kind, webhook_signature_failure_reason,
};
use tokio::sync::oneshot;
use tracing::{debug, warn};

use super::{
    key::WebhookKey,
    signature::{
        SignatureVerdict, enforce_signature, missing_secret_response, record_signature_failure,
        signature_rejected_response,
    },
    token::token_hash,
};
use crate::transport::webhook::transport::WebhookTransport;

/// Axum handler for `POST /{prefix}/{trigger_uuid}/{nonce}`.
///
/// Error-to-status mapping follows the spec:
///
/// | Situation                            | Status |
/// |--------------------------------------|--------|
/// | Unknown `(uuid, nonce)`              | 404    |
/// | Invalid UUID in path                 | 404    |
/// | Body exceeds `body_limit_bytes`      | 413    |
/// | Header count exceeds 256             | 400    |
/// | Rate limit exceeded                  | 429    |
/// | Handler returns `ActionError` (any)  | 500    |
/// | Oneshot timeout                      | 504    |
/// | Oneshot RecvError (unexpected)       | 500    |
pub(super) async fn webhook_handler(
    State(transport): State<WebhookTransport>,
    Path((trigger_uuid_str, nonce)): Path<(String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // 1. Parse UUID — malformed path segment → 404.
    let trigger_uuid = match uuid::Uuid::parse_str(&trigger_uuid_str) {
        Ok(u) => u,
        Err(_) => return (StatusCode::NOT_FOUND, "").into_response(),
    };
    let key = WebhookKey::programmatic(trigger_uuid, nonce.clone());

    // 2. Token resolution via the B-world port store (ADR-0096 commit 2b).
    //
    // Resolution is wired here and proves the scope/workflow_id/mode tuple
    // can be retrieved from the durable store.  Actual durable-emitter dispatch
    // (installing a `DurableExecutionEmitter` under `row.scope`) is the NEXT
    // sub-slice (U-D1.4b).  Until that lands the emitter remains Noop and
    // dispatch continues via the in-memory routing map below.
    if let Some(store) = transport.inner.activation_store.as_deref() {
        let hash = token_hash(&nonce);
        match store.resolve_by_token(&hash).await {
            Ok(Some(row)) => {
                debug!(
                    trigger_id = %row.trigger_id,
                    scope = ?row.scope,
                    mode = ?row.mode,
                    workflow_id = ?row.workflow_id,
                    // nonce / hash deliberately excluded
                    "capability token resolved to durable row \
                     (durable-dispatch deferred to U-D1.4b, emitter still Noop)"
                );
            },
            Ok(None) => {
                // Row not found in port store.  This is expected during the
                // transition period when the store is wired but the activation
                // was minted without `activate_and_persist`.  Fall through to
                // the in-memory routing map which is always authoritative for
                // dispatch.
                debug!("capability token not in port store — continuing via in-memory map");
            },
            Err(err) => {
                // Storage errors on the read path are non-fatal: dispatch can
                // still proceed through the in-memory routing map.  Log and
                // continue; do NOT return 500 here (availability > durable
                // resolution on the inbound webhook hot path).
                warn!(
                    error = %err,
                    "resolve_by_token storage error; falling through to in-memory dispatch"
                );
            },
        }
    }

    dispatch_inner(transport, key, method, uri, headers, body).await
}

/// Axum handler for `POST /api/v1/hooks/{org}/{ws}/{slug}`. Builds a
/// [`WebhookKey::Slug`] and delegates to `dispatch_inner` — same
/// signature/replay/rate-limit/pre-handle/handle pipeline as the
/// programmatic surface.
pub(super) async fn slug_webhook_handler(
    State(transport): State<WebhookTransport>,
    Path((org, workspace, trigger)): Path<(String, String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let coords = super::key::TriggerCoordinates::new(org, workspace, trigger);
    let key = WebhookKey::Slug(coords);
    dispatch_inner(transport, key, method, uri, headers, body).await
}

/// Shared dispatch pipeline for both programmatic and slug webhook
/// surfaces (webhook activation). Order of operations:
///
/// 1. body size check → 413
/// 2. routing lookup → 404 (before rate-limit — #271)
/// 3. rate-limit by [`WebhookKey`] → 429 + `Retry-After`
/// 4. construct [`WebhookRequest`] → 400 / 413
/// 5. [`enforce_signature`] (uses [`nebula_action::Clock`]) → 401 / 500
/// 6. dispatch via [`TriggerHandler::handle_event`] with a response
///    timeout → 504 / 500 / handler response
pub(super) async fn dispatch_inner(
    transport: WebhookTransport,
    key: WebhookKey,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Body size check. Axum's `Bytes` extractor consumes the entire
    // body; we enforce the cap AFTER extraction because
    // `axum::extract::DefaultBodyLimit` is applied at the router
    // level and we want a domain-specific 413 with our cap number.
    if body.len() > transport.inner.config.body_limit_bytes {
        debug!(
            size = body.len(),
            cap = transport.inner.config.body_limit_bytes,
            "webhook body exceeds cap"
        );
        return (StatusCode::PAYLOAD_TOO_LARGE, "").into_response();
    }

    // 3. Route lookup BEFORE rate-limit so attacker churn through
    // unregistered keys cannot evict legitimate buckets from the
    // LRU-bounded path table (#271 follow-up). Unregistered keys
    // never touch the limiter.
    let entry = if let Some(e) = transport.inner.routing.lookup(&key) {
        e
    } else {
        // key's Debug impl redacts the nonce — safe to log.
        debug!(key = ?key, "no webhook registered for path");
        return (StatusCode::NOT_FOUND, "").into_response();
    };

    // 4. Rate limit (if configured) — only for keys that resolve to a
    // registered handler.
    if let Some(limiter) = &transport.inner.rate_limiter {
        let bucket = key.rate_limit_key();
        if let Err(e) = limiter.check(&bucket).await {
            // `bucket` uses the trigger uuid only — nonce (bearer token) excluded.
            debug!(bucket = %bucket, retry_after = e.retry_after_secs, "webhook rate limited");
            record_rate_limit_rejection(&transport, &key);
            let mut resp = (StatusCode::TOO_MANY_REQUESTS, "").into_response();
            if let Ok(v) = e.retry_after_secs.to_string().parse() {
                resp.headers_mut().insert("retry-after", v);
            }
            return resp;
        }
    }

    // 5. Construct WebhookRequest. Limits are already enforced by
    // `try_new` — the only failures here are body-size exceed
    // (handled above for a better error message) and header count
    // exceed (rare; returns 400).
    let path = uri.path().to_string();
    let query = uri.query().map(String::from);
    let request = match WebhookRequest::try_new(method, path, query, headers, body) {
        Ok(r) => r,
        Err(nebula_action::ActionError::DataLimitExceeded { .. }) => {
            return (StatusCode::PAYLOAD_TOO_LARGE, "").into_response();
        },
        Err(e) => {
            debug!(error = %e, "webhook request construction failed");
            return (StatusCode::BAD_REQUEST, "").into_response();
        },
    };

    // 5.5. Signature enforcement. The `Required` default
    // means an action that forgot to configure a secret trips a 500
    // here; an action that explicitly opted into
    // `OptionalAcceptUnsigned` passes through; everything else
    // (hex / base64 / custom) runs through the existing constant-time
    // primitives before the handler sees the request.
    match enforce_signature(
        entry.config.signature_policy(),
        &request,
        transport.inner.clock.as_ref(),
    ) {
        SignatureVerdict::Pass => {},
        SignatureVerdict::MissingSecret => {
            record_signature_failure(
                &transport.inner.metrics,
                webhook_signature_failure_reason::MISSING_SECRET,
            );
            return missing_secret_response(uri.path());
        },
        SignatureVerdict::Fail(reason) => {
            record_signature_failure(&transport.inner.metrics, reason);
            return signature_rejected_response(uri.path(), reason);
        },
    }

    // 6. Oneshot response channel.
    let (tx, rx) = oneshot::channel::<WebhookHttpResponse>();
    let request = request.with_response_channel(tx);
    let event = TriggerEvent::new(None, request);

    // 7. Dispatch with timeout. The handler sends the HTTP response
    // through the oneshot; we race that against the configured
    // `response_timeout`.
    let handler = Arc::clone(&entry.handler);
    let ctx = entry.ctx.clone();
    let timeout = transport.inner.config.response_timeout;
    let dispatch_fut = async move { handler.handle_event(event, &ctx).await };

    let dispatch_result = tokio::time::timeout(timeout, dispatch_fut).await;

    match dispatch_result {
        Ok(Ok(_outcome)) => {
            // Outcome is the workflow-emission outcome; it's already
            // been used by the adapter to record health. The HTTP
            // response comes through the oneshot the adapter sent to
            // right before returning Ok.
            if let Ok(http) = rx.await {
                http_response_to_axum(http)
            } else {
                warn!("webhook handler returned Ok but oneshot sender was dropped");
                (StatusCode::INTERNAL_SERVER_ERROR, "").into_response()
            }
        },
        Ok(Err(e)) => {
            // Handler returned an error. The adapter (after H1 fix)
            // ALREADY sent a 500 via the oneshot before returning
            // Err. We just read it.
            debug!(error = %e, "webhook handler returned error");
            match rx.await {
                Ok(http) => http_response_to_axum(http),
                Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "").into_response(),
            }
        },
        Err(_) => {
            warn!(
                timeout_secs = timeout.as_secs(),
                "webhook handler dispatch timed out"
            );
            (StatusCode::GATEWAY_TIMEOUT, "").into_response()
        },
    }
}

/// Convert a `nebula-action` `WebhookHttpResponse` into an axum
/// `Response`. Shared between the Ok and Err dispatch paths.
fn http_response_to_axum(resp: WebhookHttpResponse) -> Response {
    (resp.status, resp.headers, resp.body).into_response()
}

/// Record a per-key rate-limit rejection. Labelset:
/// `(tenant_id, webhook_key_kind)`.
fn record_rate_limit_rejection(transport: &WebhookTransport, key: &WebhookKey) {
    let Some(reg) = &transport.inner.metrics else {
        return;
    };
    let interner = reg.interner();
    let kind = match key {
        WebhookKey::Programmatic { .. } => webhook_key_kind::PROGRAMMATIC,
        WebhookKey::Slug(_) => webhook_key_kind::SLUG,
    };
    let tenant_id = match key {
        WebhookKey::Programmatic { .. } => "programmatic".to_owned(),
        WebhookKey::Slug(coords) => format!("{}/{}", coords.org, coords.workspace),
    };
    let labels = interner.label_set(&[
        ("webhook_key_kind", kind),
        ("tenant_id", tenant_id.as_str()),
    ]);
    if let Ok(c) = reg.counter_labeled(NEBULA_WEBHOOK_RATE_LIMIT_REJECTIONS_TOTAL, &labels) {
        c.inc();
    }
}
