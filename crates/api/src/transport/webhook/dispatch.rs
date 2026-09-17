//! Webhook dispatch pipeline (webhook activation).
//!
//! Contains the shared `dispatch_inner` pipeline for programmatic webhook
//! activations. The slug-routed surface was retired in ADR-0096 commit 3.
//!
//! ## Pipeline order (webhook activation)
//!
//! 1. Body size check → 413
//! 2. Route lookup → 404 (before rate-limit so unregistered keys
//!    never touch the limiter — #271 follow-up)
//! 3. Rate-limit by key → 429 + `Retry-After`
//! 4. Token resolution via B-world port store — after route+rate-limit so
//!    unauthenticated churn never hits the DB (ADR-0096 security fix)
//! 5. Construct [`WebhookRequest`] → 400 / 413
//! 6. Signature enforcement ([`super::signature::enforce_signature`]) → 401 / 500
//! 7. Extract `webhook-id` header → `event_id: Option<IdempotencyKey>`
//! 8. Dispatch via [`TriggerHandler::handle_event`](nebula_action::TriggerHandler::handle_event) with timeout → 504 / 500 / handler response
//! 9. Prod rows with `durable_dispatch` wired call
//!    [`DurableExecutionEmitter::emit`] before acking the HTTP response

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderName, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use nebula_action::{
    ExecutionEmitter, IdempotencyKey, SignaturePolicy, TriggerEvent, TriggerEventOutcome,
    WebhookHttpResponse, WebhookRequest,
};
use nebula_core::NodeKey;
use nebula_engine::DurableExecutionEmitter;
use nebula_metrics::{
    NEBULA_WEBHOOK_RATE_LIMIT_REJECTIONS_TOTAL, webhook_key_kind, webhook_rate_limit_tier,
    webhook_signature_failure_reason,
};
use nebula_storage_port::dto::WebhookMode;
use tokio::sync::oneshot;
use tracing::{debug, warn};

use super::{
    key::WebhookKey,
    signature::{
        SignatureVerdict, enforce_signature, missing_secret_response,
        prod_requires_signature_response, record_signature_failure, signature_rejected_response,
    },
    token::token_hash,
    transport::DurableDispatchComponents,
};
use crate::transport::webhook::transport::WebhookTransport;

/// Standard Webhooks delivery-id header (lowercase).
/// Source: standardwebhooks.com — `webhook-id` is the canonical per-delivery
/// idempotency key supplied by the sender; it is NOT a secret and may be logged.
const WEBHOOK_ID_HEADER: HeaderName = HeaderName::from_static("webhook-id");

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
    let key = WebhookKey::programmatic(trigger_uuid, nonce);

    dispatch_inner(transport, key, method, uri, headers, body).await
}

/// Shared dispatch pipeline for programmatic webhook
/// surfaces (webhook activation). Order of operations:
///
/// 1. body size check → 413
/// 2. routing lookup → 404 (before rate-limit — #271)
/// 3. rate-limit by [`WebhookKey`] → 429 + `Retry-After`
/// 4. token resolution via B-world port store — after route+rate-limit so
///    unauthenticated churn never hits the DB (ADR-0096 security fix)
/// 5. construct [`WebhookRequest`] → 400 / 413
/// 6. [`enforce_signature`] (uses [`nebula_action::Clock`]) → 401 / 500
/// 7. extract `webhook-id` header → `event_id: Option<IdempotencyKey>`
/// 8. dispatch via [`TriggerHandler::handle_event`](nebula_action::TriggerHandler::handle_event) with a response
///    timeout → 504 / 500 / handler response
/// 9. Prod rows call [`DurableExecutionEmitter::emit`] before
///    returning the ack; emit failure → 5xx so the sender retries.
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

    // 4. Per-token rate limit (if configured) — only for keys that resolve to a
    // registered handler. Placed pre-resolution so unauthenticated churn against
    // registered keys never reaches the DB.
    if let Some(limiter) = &transport.inner.rate_limiter {
        let bucket = key.rate_limit_key();
        if let Err(e) = limiter.check(&bucket).await {
            // `bucket` uses the trigger uuid only — nonce (bearer token) excluded.
            debug!(bucket = %bucket, retry_after = e.retry_after_secs, "webhook per-token rate limited");
            record_rate_limit_rejection(&transport, &key, webhook_rate_limit_tier::PER_TOKEN, None);
            return rate_limit_429(e.retry_after_secs);
        }
    }

    // Resolve the capability token through the durable activation store.
    //
    // Placed AFTER route-lookup (step 3) and rate-limit (step 4) so an
    // unauthenticated attacker hitting an unregistered path or a rate-limited
    // key never triggers a DB query.  Only authenticated-enough requests
    // (registered key, under rate limit) reach the store.
    //
    // nonce / hash are deliberately excluded from all log fields — the nonce
    // is the bearer token and must never appear in log aggregators or traces.
    //
    // Prod rows use the durable target when runtime dispatch is wired. Test
    // mode and missing rows continue without spawning an execution.
    let mut durable: Option<DurableTarget> = None;

    if let Some(store) = transport.inner.activation_store.as_deref() {
        let hash = token_hash(key.nonce());
        match store.resolve_by_token(&hash).await {
            Ok(Some(row)) => {
                debug!(
                    trigger_id = %row.trigger_id,
                    scope = ?row.scope,
                    mode = ?row.mode,
                    workflow_id = ?row.workflow_id,
                    // nonce / hash deliberately excluded
                    "capability token resolved to durable row"
                );

                // Enforce the tenant aggregate limit after resolving its scope.
                //
                // Placed here — after token resolution yielded the scope —
                // so the stable tenant key is available without a second DB
                // lookup. Enforced before the durable target is set so a
                // tenant flooding across many tokens is capped even though
                // each token is individually within its own per-token quota.
                //
                // Key: `Scope::credential_owner_id()` — length-prefixed,
                // injective across arbitrary (org_id, workspace_id) pairs,
                // same derivation every plane uses (ADR-0088 D7).
                if let Some(resp) = check_tenant_rate_limit(&transport, &key, &row.scope).await {
                    return resp;
                }

                // Only Prod rows with a wired runtime spawn durable executions.
                if row.mode == WebhookMode::Prod {
                    if let Some(components) = transport.inner.durable_dispatch.as_ref() {
                        durable = Some(DurableTarget {
                            row,
                            components: components.clone(),
                        });
                    } else {
                        // Prod mode but no inbox wired — fail closed so a
                        // misconfigured composition root never spawns
                        // dedup-blind.
                        warn!(
                            mode = "Prod",
                            "Prod-mode webhook: durable_dispatch not wired; \
                             refusing to dispatch to prevent dedup-blind spawn"
                        );
                        return (StatusCode::INTERNAL_SERVER_ERROR, "").into_response();
                    }
                }
                // Test mode: no durable spawn; fall through to in-memory
                // dispatch with Noop emitter.
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
                // Storage error resolving the capability token. When durable
                // dispatch is WIRED, the row is the trusted source of
                // mode/scope/workflow, so we MUST NOT silently downgrade a Prod
                // trigger to the Noop in-memory path: that would 2xx the sender
                // and lose the event (no retry). Fail closed with 503 so the
                // sender retries; on store recovery the retry resolves and
                // trigger start materialization deduplicates by `event_id`. If durable
                // dispatch is NOT wired there is no durable contract to protect —
                // fall through to in-memory for availability.
                let durable_wired = transport.inner.durable_dispatch.is_some();
                warn!(
                    error = %err,
                    durable = durable_wired,
                    "resolve_by_token storage error"
                );
                if durable_wired {
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "webhook activation store unavailable; retry",
                    )
                        .into_response();
                }
                // No durable contract — continue via the in-memory routing map.
            },
        }
    }

    // 5. Construct WebhookRequest. Limits are already enforced by
    // `try_new` — the only failures here are body-size exceed
    // (handled above for a better error message) and header count
    // exceed (rare; returns 400).
    let path = uri.path().to_string();
    let query = uri.query().map(String::from);

    // Extract `webhook-id` before consuming `headers`
    // into `WebhookRequest`.  The header is NOT a secret (Standard Webhooks spec
    // §4 — "webhook-id must be a unique identifier per message delivery") and
    // may be logged as a tracing field.
    //
    // Fail-closed rule (inv #6): an EMITTING Prod outcome requires `webhook-id`
    // (the dedup key). The requirement is enforced AFTER dispatch, only on an
    // `Emit` outcome — a provider verification probe (Slack `url_verification`,
    // Stripe `pending_webhook`) returns `Skip` and needs no delivery id, so the
    // header must not be required before the outcome is known.
    // Test mode / no-row → `event_id = None` is fine.
    // Bound the delivery-id length at the edge: an over-long `webhook-id`
    // would flow into the dedup-key PK and surface as a backend-dependent
    // failure (e.g. a Postgres btree index-row-too-large INSERT error → 5xx)
    // instead of a clean rejection. 256 bytes is far above any real delivery
    // id (Svix `msg_…`, Stripe `evt_…`, GitHub UUID are all < 64). An empty /
    // whitespace-only value is treated as absent (the Prod fail-closed check
    // below then requires a real id).
    const MAX_WEBHOOK_ID_LEN: usize = 256;
    // Reject DUPLICATE `webhook-id` headers: `HeaderMap::get` silently returns
    // one of several values, which would let two conflicting delivery ids slip
    // past dedup. Exactly one or zero is permitted.
    let mut webhook_id_values = headers.get_all(&WEBHOOK_ID_HEADER).iter();
    let first_webhook_id = webhook_id_values.next();
    if webhook_id_values.next().is_some() {
        warn!("webhook: duplicate `webhook-id` headers; rejecting ambiguous delivery id");
        return (StatusCode::BAD_REQUEST, "duplicate webhook-id header").into_response();
    }
    let event_id: Option<IdempotencyKey> = match first_webhook_id
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) if s.len() > MAX_WEBHOOK_ID_LEN => {
            warn!(
                "webhook: `webhook-id` header exceeds {MAX_WEBHOOK_ID_LEN} bytes; \
                 rejecting (delivery id never legitimately this long)"
            );
            return (StatusCode::BAD_REQUEST, "webhook-id header too long").into_response();
        },
        Some(s) => Some(IdempotencyKey::new(s)),
        None => None,
    };

    // NOTE: the `webhook-id` requirement is NOT enforced here — it is deferred
    // to the post-dispatch `Emit` arm (see `dispatch_durable`), so a Prod
    // verification probe that returns `Skip` is not rejected for lacking a
    // delivery id it never needs.

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

    // 5.5. Signature enforcement.
    //
    // A Prod row that can spawn a durable execution must require a signature.
    // MUST NOT be verifiable under `OptionalAcceptUnsigned`.  An unsigned
    // Prod activation is an operator/composition-root misconfiguration — it
    // would let an unverified caller spawn durable executions.  Fail closed
    // with 500 (same surface as `missing_secret`) so dashboards see it.
    // `durable.is_some()` ⟺ Prod row that resolved to a durable target.
    if durable.is_some()
        && matches!(
            entry.config.signature_policy(),
            SignaturePolicy::OptionalAcceptUnsigned
        )
    {
        warn!(
            mode = "Prod",
            "Prod-mode webhook: signature policy is OptionalAcceptUnsigned; \
             Prod activations must use SignaturePolicy::Required — \
             refusing to dispatch (composition-root misconfiguration)"
        );
        record_signature_failure(
            &transport.inner.metrics,
            webhook_signature_failure_reason::PROD_UNSIGNED,
        );
        return prod_requires_signature_response(uri.path());
    }

    // The `Required` default means an action that forgot to configure a
    // secret trips a 500 here; an action that explicitly opted into
    // `OptionalAcceptUnsigned` passes through (for non-Prod paths only —
    // the Prod guard above already rejected Prod+unsigned); everything else
    // (hex / base64 / Standard Webhooks / custom) runs through the existing
    // constant-time primitives before the handler sees the request.
    match enforce_signature(
        entry.config.signature_policy(),
        &request,
        transport.inner.clock.as_ref(),
    ) {
        SignatureVerdict::Pass => {},
        SignatureVerdict::MissingSecret => {
            // `key.rate_limit_key()` is the trigger UUID only — nonce excluded.
            warn!(
                bucket = %key.rate_limit_key(),
                "webhook signature secret not configured; action must supply a secret \
                 or explicitly opt into OptionalAcceptUnsigned"
            );
            record_signature_failure(
                &transport.inner.metrics,
                webhook_signature_failure_reason::MISSING_SECRET,
            );
            return missing_secret_response(uri.path());
        },
        SignatureVerdict::Fail(reason) => {
            // `key.rate_limit_key()` is the trigger UUID only — nonce excluded.
            warn!(
                bucket = %key.rate_limit_key(),
                reason,
                "webhook signature verification failed"
            );
            record_signature_failure(&transport.inner.metrics, reason);
            return signature_rejected_response(uri.path(), reason);
        },
    }

    // 6. Oneshot response channel.
    let (tx, rx) = oneshot::channel::<WebhookHttpResponse>();
    let request = request.with_response_channel(tx);
    let event = TriggerEvent::new(None, request);

    // 8+9. Dispatch with timeout.
    //
    // The combined future wraps BOTH `handle_event` AND the conditional
    // `emitter.emit()` inside one `tokio::time::timeout` region.  This
    // satisfies the "emit inside the timeout" invariant: a stuck DB write
    // yields 504, not a hang.
    //
    // Ordering (research-confirmed at-least-once):
    //
    //   a. `handler.handle_event(event, &ctx)` → adapter sends HTTP response
    //      via the oneshot BEFORE returning `Ok(outcome)`.
    //   b. On `Emit(payload)` + `durable.is_some()`:
    //      - Load ValidatedWorkflow under `row.scope`.
    //      - Construct DurableExecutionEmitter.
    //      - `emitter.emit(payload, Some(event_id)).await`.
    //      - On emit Ok  → read the HTTP response from rx.
    //      - On emit Err → return 5xx (discard oneshot's response so the
    //        sender retries; same `webhook-id` → same `event_id` → dedup).
    //   c. The adapter sends the response BEFORE returning, so `rx.await`
    //      inside the combined future is non-blocking after `handle_event`
    //      returns.  The combined timeout region correctly accounts for the
    //      full wall-clock cost of dispatch + emit.
    let handler = Arc::clone(&entry.handler);
    let ctx = entry.ctx.clone();
    let timeout = transport.inner.config.response_timeout;

    let combined_fut = async move {
        let outcome = match handler.handle_event(event, &ctx).await {
            Ok(o) => o,
            Err(e) => {
                // Handler returned an error. The adapter ALREADY sent a
                // response via the oneshot before returning Err.
                debug!(error = %e, "webhook handler returned error");
                let http = rx.await.unwrap_or_else(|_| {
                    WebhookHttpResponse::new(StatusCode::INTERNAL_SERVER_ERROR, "")
                });
                return http_response_to_axum(http);
            },
        };

        // Handler succeeded — durable emit (if applicable) runs here,
        // still inside the timeout region.
        if let Some(target) = durable {
            return dispatch_durable(target, outcome, event_id, rx).await;
        }

        // No durable dispatch (Test mode / no-row / fall-through).
        let http = rx.await.unwrap_or_else(|_| {
            warn!("webhook handler returned Ok but oneshot sender was dropped");
            WebhookHttpResponse::new(StatusCode::INTERNAL_SERVER_ERROR, "")
        });
        http_response_to_axum(http)
    };

    match tokio::time::timeout(timeout, combined_fut).await {
        Ok(resp) => resp,
        Err(_elapsed) => {
            warn!(
                timeout_secs = timeout.as_secs(),
                "webhook handler dispatch timed out"
            );
            (StatusCode::GATEWAY_TIMEOUT, "").into_response()
        },
    }
}

/// Bundle carrying all data needed for the Prod-mode durable emit path.
struct DurableTarget {
    row: nebula_storage_port::dto::WebhookActivationRecord,
    components: DurableDispatchComponents,
}

/// Attempt to spawn a durable execution for a Prod-mode outcome.
///
/// Returns the axum `Response` to send back to the caller:
/// - `Emit(payload)` → load workflow + emit → ack on success, 5xx on failure.
/// - `EmitMany(_)` in Prod → fail-closed 5xx (dedup-collision data-loss bug
///   — one `event_id` cannot safely fan-out to N executions).
/// - `Skip` → no emit; return the adapter's HTTP response.
///
async fn dispatch_durable(
    target: DurableTarget,
    outcome: TriggerEventOutcome,
    event_id: Option<IdempotencyKey>,
    rx: oneshot::Receiver<WebhookHttpResponse>,
) -> Response {
    let DurableTarget { row, components } = target;

    match outcome {
        TriggerEventOutcome::Emit(payload) => {
            let Some(event_id) = event_id.as_ref() else {
                warn!(
                    trigger_id = %row.trigger_id,
                    mode = "Prod",
                    "Prod-mode webhook Emit without `webhook-id`; \
                     fail-closed (dedup requires a delivery id)"
                );
                return (
                    StatusCode::BAD_REQUEST,
                    "missing webhook-id header for Prod-mode emit",
                )
                    .into_response();
            };

            match emit_durable_execution(&row, &components, payload, event_id).await {
                Ok(()) => receive_adapter_response(rx, "durable emit completed").await,
                Err(status) => bare_response(status),
            }
        },
        TriggerEventOutcome::EmitMany(_) => {
            // Fail-closed: emitting N payloads under one `event_id` is a
            // dedup-collision data-loss bug.  No first-party webhook action
            // returns EmitMany; if one does, the operator must fix the action.
            warn!(
                trigger_id = %row.trigger_id,
                scope = ?row.scope,
                mode = "Prod",
                "Prod-mode webhook: EmitMany outcome refused \
                 (one event_id cannot safely fan-out to N executions; \
                 action must not return EmitMany in Prod mode)"
            );
            bare_response(StatusCode::INTERNAL_SERVER_ERROR)
        },
        TriggerEventOutcome::Skip => {
            receive_adapter_response(rx, "webhook handler skipped execution").await
        },
        // `TriggerEventOutcome` is #[non_exhaustive] — any future variant
        // whose semantics are unknown MUST be refused fail-closed.
        _ => {
            warn!(
                trigger_id = %row.trigger_id,
                scope = ?row.scope,
                "Prod-mode webhook: unknown TriggerEventOutcome variant; \
                 fail-closed — no execution spawned"
            );
            bare_response(StatusCode::INTERNAL_SERVER_ERROR)
        },
    }
}

struct DurableExecutionIdentity {
    workflow_id: nebula_core::WorkflowId,
    trigger_node_key: NodeKey,
}

async fn emit_durable_execution(
    row: &nebula_storage_port::dto::WebhookActivationRecord,
    components: &DurableDispatchComponents,
    payload: serde_json::Value,
    event_id: &IdempotencyKey,
) -> Result<(), StatusCode> {
    let scope = &row.scope;
    let identity = durable_execution_identity(row)?;
    let emitter = DurableExecutionEmitter::new(
        Arc::clone(&components.start),
        identity.workflow_id,
        identity.trigger_node_key,
        scope.clone(),
    );

    emitter
        .emit(payload, Some(event_id.clone()))
        .await
        .map(|_execution_id| ())
        .map_err(|error| {
            warn!(
                trigger_id = %row.trigger_id,
                scope = ?scope,
                workflow_id = %identity.workflow_id,
                error = %error,
                "Prod-mode webhook: durable start did not return a checked receipt"
            );
            workflow_start_error(&error).map_or(
                StatusCode::INTERNAL_SERVER_ERROR,
                webhook_start_failure_status,
            )
        })
}

fn durable_execution_identity(
    row: &nebula_storage_port::dto::WebhookActivationRecord,
) -> Result<DurableExecutionIdentity, StatusCode> {
    let scope = &row.scope;
    let workflow_id_str = if let Some(wid) = &row.workflow_id {
        wid.as_str()
    } else {
        warn!(
            trigger_id = %row.trigger_id,
            scope = ?scope,
            mode = "Prod",
            "Prod-mode webhook: activation row has no workflow_id; \
             fail-closed — no execution spawned"
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };

    use nebula_core::id::WorkflowId;
    let workflow_id: WorkflowId = match workflow_id_str.parse() {
        Ok(id) => id,
        Err(e) => {
            warn!(
                trigger_id = %row.trigger_id,
                scope = ?scope,
                workflow_id = workflow_id_str,
                error = %e,
                "Prod-mode webhook: activation row workflow_id is not a valid WorkflowId; \
                 fail-closed — no execution spawned"
            );
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        },
    };

    let trigger_node_key = match NodeKey::new(&row.trigger_id) {
        Ok(k) => k,
        Err(e) => {
            warn!(
                trigger_id = %row.trigger_id,
                scope = ?scope,
                error = %e,
                "Prod-mode webhook: activation row trigger_id is not a valid NodeKey; \
                 fail-closed — no execution spawned"
            );
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        },
    };

    Ok(DurableExecutionIdentity {
        workflow_id,
        trigger_node_key,
    })
}

fn workflow_start_error<'a>(
    mut error: &'a (dyn std::error::Error + 'static),
) -> Option<&'a nebula_engine::WorkflowStartError> {
    loop {
        if let Some(start_error) = error.downcast_ref() {
            return Some(start_error);
        }
        error = error.source()?;
    }
}

#[cfg(test)]
fn webhook_start_failure_response(error: &nebula_engine::WorkflowStartError) -> Response {
    bare_response(webhook_start_failure_status(error))
}

/// Map runtime-owned start failures to retryable, payload-free webhook responses.
///
/// The webhook sender cannot repair activation or persisted-contract failures.
/// Reporting them as client errors would make most providers drop the delivery,
/// while returning structured API errors would disclose internal identifiers.
fn webhook_start_failure_status(error: &nebula_engine::WorkflowStartError) -> StatusCode {
    use nebula_engine::{PlanFlavorRevisionBridgeError, WorkflowStartError};
    use nebula_storage_port::dto::RevisionCatalogError;

    match error {
        WorkflowStartError::BackendUnavailable
        | WorkflowStartError::MaterializationIndeterminate(_)
        | WorkflowStartError::ReceiptUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
        WorkflowStartError::RevisionUnavailable(source)
            if matches!(
                source.as_ref(),
                PlanFlavorRevisionBridgeError::Catalog {
                    source: RevisionCatalogError::Unavailable,
                    ..
                }
            ) =>
        {
            StatusCode::SERVICE_UNAVAILABLE
        },
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn receive_adapter_response(
    response: oneshot::Receiver<WebhookHttpResponse>,
    completed_operation: &'static str,
) -> Response {
    if let Ok(http) = response.await {
        http_response_to_axum(http)
    } else {
        warn!(completed_operation, "webhook response channel closed");
        bare_response(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

fn bare_response(status: StatusCode) -> Response {
    (status, "").into_response()
}

/// Convert a `nebula-action` `WebhookHttpResponse` into an axum
/// `Response`. Shared between the Ok and Err dispatch paths.
fn http_response_to_axum(resp: WebhookHttpResponse) -> Response {
    (resp.status, resp.headers, resp.body).into_response()
}

/// Build a `429 Too Many Requests` response with a `Retry-After` header.
fn rate_limit_429(retry_after_secs: u64) -> Response {
    let mut resp = (StatusCode::TOO_MANY_REQUESTS, "").into_response();
    if let Ok(v) = retry_after_secs.to_string().parse() {
        resp.headers_mut().insert("retry-after", v);
    }
    resp
}

/// Check the per-tenant-aggregate rate limiter for the resolved `scope`.
///
/// Returns `Some(Response)` (a 429) when the tenant aggregate is exceeded,
/// `None` when the request is within quota or no tenant limiter is configured.
///
/// Extracted into a free function to keep the deeply-nested `Ok(Some(row))`
/// arm under clippy's `excessive_nesting` threshold.
async fn check_tenant_rate_limit(
    transport: &WebhookTransport,
    key: &WebhookKey,
    scope: &nebula_storage_port::Scope,
) -> Option<Response> {
    let limiter = transport.inner.tenant_rate_limiter.as_ref()?;
    let tenant_key = scope.credential_owner_id();
    let err = limiter.check(&tenant_key).await.err()?;
    debug!(
        tenant_id = %tenant_key,
        retry_after = err.retry_after_secs,
        "webhook per-tenant-aggregate rate limited"
    );
    record_rate_limit_rejection(
        transport,
        key,
        webhook_rate_limit_tier::PER_TENANT,
        Some(&tenant_key),
    );
    Some(rate_limit_429(err.retry_after_secs))
}

/// Record a rate-limit rejection. Labelset: `(webhook_key_kind, tier)`; plus
/// an optional `tenant_id` label when the rejection is post-resolution.
///
/// - `tier = PER_TOKEN` (step 4, pre-resolution): pass `tenant_id = None`.
///   The trigger UUID is NOT emitted as `tenant_id` — it is unbounded in
///   cardinality and would create one series per registered webhook.
/// - `tier = PER_TENANT` (step 4.5, post-resolution): pass `tenant_id =
///   Some(&scope.credential_owner_id())`. The tenant key is the bounded
///   `(org, workspace)` slug pair — bounded per deployment.
///
/// `tier` must be one of [`webhook_rate_limit_tier::PER_TOKEN`] or
/// [`webhook_rate_limit_tier::PER_TENANT`].
fn record_rate_limit_rejection(
    transport: &WebhookTransport,
    key: &WebhookKey,
    tier: &'static str,
    tenant_id: Option<&str>,
) {
    let Some(reg) = &transport.inner.metrics else {
        return;
    };
    let interner = reg.interner();
    let WebhookKey::Programmatic { .. } = key;
    let kind = webhook_key_kind::PROGRAMMATIC;
    let labels = if let Some(tid) = tenant_id {
        interner.label_set(&[
            ("webhook_key_kind", kind),
            ("tenant_id", tid),
            ("tier", tier),
        ])
    } else {
        interner.label_set(&[("webhook_key_kind", kind), ("tier", tier)])
    };
    if let Ok(c) = reg.counter_labeled(NEBULA_WEBHOOK_RATE_LIMIT_REJECTIONS_TOTAL, &labels) {
        c.inc();
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
