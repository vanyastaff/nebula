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
mod tests {
    //! Integration tests for durable webhook dispatch.
    //!
    //! Each test drives `dispatch_inner` directly — the same code path the
    //! axum handler calls — so the assertions cover the real dispatch logic.
    //!
    //! # Test fixture overview
    //!
    //! `Fixture` wires a complete in-memory stack:
    //! - `WebhookTransport` with `activation_store` + `durable_dispatch`
    //! - `InMemoryWebhookActivationStore` (token resolution)
    //! - `WorkflowStartService` (atomic trigger dedup + ControlStart + bundle)
    //! - `InMemoryWorkflowVersionStore` (workflow load)
    //! - `InMemoryExecutionStore` (execution rows + control queue + exact references)
    //! - frozen registry and explicitly activated workflow revisions
    //! - `ConfigurableWebhookAction` — a `WebhookAction` whose outcome
    //!   is set per-test via `Arc<Mutex<TriggerEventOutcome>>`.

    use std::sync::Arc;
    use std::time::SystemTime;

    use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
    use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
    use nebula_action::{
        Action, ActionMetadataDraft, ActionResult, FromWorkflowNode, GenericTriggerFactory,
        InstanceFactory, RequiredPolicy, SignaturePolicy, SignatureScheme, StatelessAction,
        TriggerAction, TriggerContext, TriggerEventOutcome, TriggerHandler, WebhookAction,
        WebhookConfig, WebhookRequest, WebhookResponse, WebhookSource, WebhookTriggerAdapter,
        hmac_sha256_compute,
    };
    use nebula_core::{
        BaseContext, Dependencies, NodeKey, WorkflowId, action_key, node_key, scope::Principal,
    };
    use nebula_storage::inmem::{
        InMemoryExecutionStore, InMemoryWebhookActivationStore, InMemoryWorkflowVersionStore,
    };
    use nebula_storage_port::{
        Scope,
        dto::WebhookMode,
        store::{ExecutionStore, WebhookActivationStore, WorkflowStore},
    };
    use nebula_workflow::{WorkflowBuilder, WorkflowDefinition, node::NodeDefinition};
    use parking_lot::Mutex;
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::transport::webhook::transport::{
        DEFAULT_PER_TOKEN_RPM, PersistParams, WebhookTransport, WebhookTransportConfig,
        activate_and_persist,
    };

    fn tenant_scope() -> Scope {
        Scope::new(
            nebula_core::WorkspaceId::new().to_string(),
            nebula_core::OrgId::new().to_string(),
        )
    }

    #[tokio::test]
    async fn start_failures_are_payload_free_and_preserve_retryability() {
        let unavailable =
            webhook_start_failure_response(&nebula_engine::WorkflowStartError::BackendUnavailable);
        assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);
        let unavailable_body = axum::body::to_bytes(unavailable.into_body(), 1)
            .await
            .expect("empty response body is readable");
        assert!(unavailable_body.is_empty());

        let invalid_scope =
            webhook_start_failure_response(&nebula_engine::WorkflowStartError::InvalidScope);
        assert_eq!(invalid_scope.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let invalid_scope_body = axum::body::to_bytes(invalid_scope.into_body(), 1)
            .await
            .expect("empty response body is readable");
        assert!(invalid_scope_body.is_empty());
    }

    struct FixtureStatelessAction;

    impl Action for FixtureStatelessAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadataDraft {
            ActionMetadataDraft::new(
                action_key!("core.echo"),
                nebula_action::metadata_name!("Echo fixture"),
                "Admission fixture",
            )
            .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
        }

        fn dependencies() -> &'static Dependencies {
            static DEPENDENCIES: std::sync::OnceLock<Dependencies> = std::sync::OnceLock::new();
            DEPENDENCIES.get_or_init(Dependencies::new)
        }
    }

    impl StatelessAction for FixtureStatelessAction {
        async fn execute(
            &self,
            input: Self::Input,
            _context: &(impl nebula_action::ActionContext + ?Sized),
        ) -> Result<ActionResult<Self::Output>, nebula_action::ActionError> {
            Ok(ActionResult::success(input))
        }
    }

    struct FixtureTriggerAction;

    impl Action for FixtureTriggerAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadataDraft {
            ActionMetadataDraft::new(
                action_key!("test.webhook"),
                nebula_action::metadata_name!("Webhook fixture"),
                "Admission fixture",
            )
            .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
        }

        fn dependencies() -> &'static Dependencies {
            static DEPENDENCIES: std::sync::OnceLock<Dependencies> = std::sync::OnceLock::new();
            DEPENDENCIES.get_or_init(Dependencies::new)
        }
    }

    impl TriggerAction for FixtureTriggerAction {
        type Source = WebhookSource;
        type Error = nebula_action::ActionError;

        async fn start(&self, _ctx: &(impl TriggerContext + ?Sized)) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn stop(&self, _ctx: &(impl TriggerContext + ?Sized)) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn handle(
            &self,
            _ctx: &(impl TriggerContext + ?Sized),
            _event: WebhookRequest,
        ) -> Result<TriggerEventOutcome, Self::Error> {
            Ok(TriggerEventOutcome::skip())
        }
    }

    impl FromWorkflowNode for FixtureTriggerAction {
        type Error = nebula_action::ActionError;

        async fn from_workflow_node(
            _node: &NodeDefinition,
            _ctx: &dyn nebula_action::ActionContext,
        ) -> Result<Self, Self::Error> {
            Ok(Self)
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum FixtureAction {
        Stateless,
        Trigger,
    }

    #[derive(Debug)]
    struct FixturePlugin {
        manifest: nebula_plugin::PluginManifest,
        action: FixtureAction,
    }
    impl nebula_plugin::Plugin for FixturePlugin {
        fn manifest(&self) -> &nebula_plugin::PluginManifest {
            &self.manifest
        }
        fn actions(&self) -> Vec<Arc<dyn nebula_action::ActionFactory>> {
            let factory: Arc<dyn nebula_action::ActionFactory> = match self.action {
                FixtureAction::Stateless => Arc::new(
                    InstanceFactory::new(
                        FixtureStatelessAction::metadata(),
                        FixtureStatelessAction,
                    )
                    .expect("valid test catalog definition"),
                ),
                FixtureAction::Trigger => Arc::new(
                    GenericTriggerFactory::<FixtureTriggerAction>::new()
                        .expect("valid test catalog definition"),
                ),
            };
            vec![factory]
        }
    }
    struct RuntimeFixture {
        starts: Arc<nebula_engine::WorkflowStartService>,
        activation: nebula_engine::WorkflowActivationService,
        workflows: Arc<nebula_storage::InMemoryWorkflowStore>,
    }
    impl RuntimeFixture {
        fn new(
            executions: &InMemoryExecutionStore,
            versions: &Arc<InMemoryWorkflowVersionStore>,
        ) -> Self {
            let mut registry = nebula_plugin::PluginRegistry::new();
            for (plugin, action) in [
                ("core", FixtureAction::Stateless),
                ("test", FixtureAction::Trigger),
            ] {
                registry
                    .register(Arc::new(
                        nebula_plugin::ResolvedPlugin::from(FixturePlugin {
                            manifest: nebula_plugin::PluginManifest::builder(plugin, plugin)
                                .build()
                                .unwrap(),
                            action,
                        })
                        .unwrap(),
                    ))
                    .unwrap();
            }
            let registry = Arc::new(
                registry
                    .freeze(
                        nebula_core::ArtifactSetDigest::from_bytes([0x31; 32]),
                        "1.0.0".parse().unwrap(),
                    )
                    .unwrap(),
            );
            let workflows = Arc::new(nebula_storage::InMemoryWorkflowStore::new_with_versions(
                versions, executions,
            ));
            let catalog = Arc::new(executions.plan_flavor_catalog());
            let clock = Arc::new(nebula_core::accessor::SystemClock);
            let activation = nebula_engine::WorkflowActivationService::new(
                workflows.clone(),
                versions.clone(),
                registry.clone(),
                nebula_engine::PlanFlavorRevisionInstaller::new(catalog.clone()),
                clock.clone(),
            );
            let starts = Arc::new(
                nebula_engine::WorkflowStartService::new(
                    nebula_engine::WorkflowStores {
                        workflow: workflows.clone(),
                        versions: versions.clone(),
                    },
                    Arc::new(executions.clone()),
                    Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                        executions,
                    )),
                    nebula_engine::PlanFlavorRevisionLoader::new(catalog),
                    registry,
                    clock,
                    Default::default(),
                )
                .unwrap(),
            );
            Self {
                starts,
                activation,
                workflows,
            }
        }
        async fn activate(&self, scope: &Scope, definition: WorkflowDefinition) {
            self.workflows
                .create(
                    scope,
                    nebula_storage_port::dto::WorkflowRecord {
                        id: definition.id.to_string(),
                        scope: scope.clone(),
                        version: 1,
                        slug: definition.id.to_string(),
                        deleted: false,
                    },
                )
                .await
                .unwrap();
            self.activation
                .activate(scope, definition.id, 1, definition)
                .await
                .unwrap();
        }
    }

    // ── Standard Webhooks test signing ────────────────────────────────────────

    /// HMAC key used by all Prod-mode test fixtures.
    ///
    /// Raw bytes — no `whsec_` prefix. The verify path operates on
    /// already-decoded bytes (registration concern is out of scope here).
    const TEST_SWH_SECRET: &[u8] = b"nebula-dispatch-test-secret";

    /// Build the SWH `SignaturePolicy` used by Prod fixtures.
    fn swh_required_policy() -> SignaturePolicy {
        SignaturePolicy::Required(
            RequiredPolicy::new()
                .with_secret(TEST_SWH_SECRET)
                .with_scheme(SignatureScheme::StandardWebhooks),
        )
    }

    /// Compute a valid Standard Webhooks `webhook-signature` header value.
    ///
    /// `to_sign = "{msg_id}.{ts_secs}.{body}"`
    fn sign_swh(msg_id: &str, ts_secs: u64, body: &[u8]) -> String {
        let prefix = format!("{msg_id}.{ts_secs}.");
        let mut content = Vec::with_capacity(prefix.len() + body.len());
        content.extend_from_slice(prefix.as_bytes());
        content.extend_from_slice(body);
        format!(
            "v1,{}",
            B64.encode(hmac_sha256_compute(TEST_SWH_SECRET, &content))
        )
    }

    /// Unix seconds for the current wall time — used when constructing
    /// test requests so the signature is valid against `SystemClock`.
    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system time after epoch")
            .as_secs()
    }

    /// Build a header map with `webhook-id`, `webhook-timestamp`, and a
    /// valid `webhook-signature` for the given body.
    fn swh_headers(msg_id: &str, body: &[u8]) -> HeaderMap {
        let ts = now_secs();
        let sig = sign_swh(msg_id, ts, body);
        let mut h = HeaderMap::new();
        h.insert(WEBHOOK_ID_HEADER, HeaderValue::from_str(msg_id).unwrap());
        h.insert(
            HeaderName::from_static("webhook-timestamp"),
            HeaderValue::from_str(&ts.to_string()).unwrap(),
        );
        h.insert(
            HeaderName::from_static("webhook-signature"),
            HeaderValue::from_str(&sig).unwrap(),
        );
        h
    }

    // ── TestWebhookAction ─────────────────────────────────────────────────────

    /// Minimal `WebhookAction` whose `handle_request` returns whatever
    /// `outcome_cell` says.
    ///
    /// The `config` method returns the caller-supplied `WebhookConfig`, which
    /// carries the `SignaturePolicy` for the test.  Prod-mode fixtures supply
    /// `SignaturePolicy::Required` (SWH); non-Prod / legacy fixtures may use
    /// `OptionalAcceptUnsigned`.
    struct ConfigurableWebhookAction {
        outcome_cell: Arc<Mutex<TriggerEventOutcome>>,
    }

    impl Action for ConfigurableWebhookAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadataDraft {
            ActionMetadataDraft::new(
                action_key!("test.dispatch.configurable"),
                nebula_action::metadata_name!("ConfigurableWebhookAction"),
                "Test fixture",
            )
        }

        fn dependencies() -> &'static Dependencies {
            static D: std::sync::OnceLock<Dependencies> = std::sync::OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl WebhookAction for ConfigurableWebhookAction {
        type State = ();

        async fn on_activate(
            &self,
            _ctx: &(impl TriggerContext + ?Sized),
        ) -> Result<(), nebula_action::ActionError> {
            Ok(())
        }

        async fn handle_request(
            &self,
            _request: &WebhookRequest,
            _state: &(),
            _ctx: &(impl TriggerContext + ?Sized),
        ) -> Result<WebhookResponse, nebula_action::ActionError> {
            let outcome = self.outcome_cell.lock().clone();
            Ok(WebhookResponse::accept(outcome))
        }
    }

    /// Activation store that delegates everything to an in-memory store EXCEPT
    /// `resolve_by_token`, which errors — simulating a transient store outage on
    /// the durable read path, which must fail closed with 503.
    #[derive(Debug)]
    struct FailingResolveStore(InMemoryWebhookActivationStore);

    #[async_trait::async_trait]
    impl WebhookActivationStore for FailingResolveStore {
        async fn upsert(
            &self,
            scope: &Scope,
            record: nebula_storage_port::dto::WebhookActivationRecord,
        ) -> Result<(), nebula_storage_port::StorageError> {
            self.0.upsert(scope, record).await
        }

        async fn resolve(
            &self,
            scope: &Scope,
            slug: &str,
        ) -> Result<
            Option<nebula_storage_port::dto::WebhookActivationRecord>,
            nebula_storage_port::StorageError,
        > {
            self.0.resolve(scope, slug).await
        }

        async fn deactivate(
            &self,
            scope: &Scope,
            trigger_id: &str,
        ) -> Result<(), nebula_storage_port::StorageError> {
            self.0.deactivate(scope, trigger_id).await
        }

        async fn resolve_by_token(
            &self,
            _token_hash: &[u8; 32],
        ) -> Result<
            Option<nebula_storage_port::dto::WebhookActivationRecord>,
            nebula_storage_port::StorageError,
        > {
            Err(nebula_storage_port::StorageError::Connection(
                "injected resolve_by_token outage".into(),
            ))
        }

        async fn list_all_active(
            &self,
        ) -> Result<
            Vec<nebula_storage_port::dto::WebhookActivationRecord>,
            nebula_storage_port::StorageError,
        > {
            self.0.list_all_active().await
        }
    }

    // ── TestFixture ───────────────────────────────────────────────────────────

    /// Fully-wired in-memory fixture for durable dispatch tests.
    ///
    /// Stores the `ActivationHandle` so `dispatch_with_id` / `dispatch_without_id`
    /// can derive the correct `(trigger_uuid, nonce)` webhook key.
    #[expect(dead_code)]
    struct TestFixture {
        transport: WebhookTransport,
        version_store: Arc<InMemoryWorkflowVersionStore>,
        exec_store: Arc<InMemoryExecutionStore>,
        outcome_cell: Arc<Mutex<TriggerEventOutcome>>,
        scope: Scope,
        workflow_id: WorkflowId,
        trigger_uuid: uuid::Uuid,
        nonce: String,
    }

    impl TestFixture {
        /// Build a full Prod-mode fixture (in-memory activation store).
        async fn prod(mode: WebhookMode) -> Self {
            Self::prod_with_store(mode, Arc::new(InMemoryWebhookActivationStore::new())).await
        }

        /// Build a Prod-mode fixture with a caller-supplied activation store —
        /// lets a test inject a store whose `resolve_by_token` errors.
        async fn prod_with_store(
            mode: WebhookMode,
            activation_store: Arc<dyn WebhookActivationStore>,
        ) -> Self {
            let scope = tenant_scope();
            let workflow_id = WorkflowId::new();
            let trigger_id_key = node_key!("webhook_trigger");
            let trigger_id = trigger_id_key.as_str().to_string();

            let exec_store = InMemoryExecutionStore::new();
            let exec_store = Arc::new(exec_store);
            let version_store = Arc::new(InMemoryWorkflowVersionStore::new());
            let runtime = RuntimeFixture::new(&exec_store, &version_store);

            let outcome_cell = Arc::new(Mutex::new(TriggerEventOutcome::emit(json!({"ok": true}))));

            let handler: Arc<dyn TriggerHandler> = Arc::new(
                WebhookTriggerAdapter::new(ConfigurableWebhookAction {
                    outcome_cell: Arc::clone(&outcome_cell),
                })
                .expect("valid test catalog definition"),
            );
            let ctx_template = base_ctx(workflow_id, trigger_id_key);

            // `WebhookTriggerAdapter::handle_event` requires state populated by
            // `start()`.  Call it before handing the handler to `activate_and_persist`
            // so dispatch does not see `state = None` → Fatal/500.
            handler
                .start(&ctx_template)
                .await
                .expect("handler start must succeed");

            let transport = WebhookTransport::new(WebhookTransportConfig::default())
                .with_activation_store(Arc::clone(&activation_store))
                .with_durable_dispatch(runtime.starts.clone());

            // Prod rows must carry the required Standard Webhooks policy.
            // Test rows can remain unsigned for simplicity.
            let sig_policy = if mode == WebhookMode::Prod {
                swh_required_policy()
            } else {
                SignaturePolicy::OptionalAcceptUnsigned
            };

            let handle = activate_and_persist(
                &transport,
                activation_store.as_ref(),
                PersistParams {
                    handler,
                    action_config: WebhookConfig::default().with_signature_policy(sig_policy),
                    ctx_template,
                    trigger_id: trigger_id.clone(),
                    spec_trigger_id: format!("trg_test_{trigger_id}"),
                    scope: scope.clone(),
                    workflow_id: Some(workflow_id.to_string()),
                    mode,
                },
            )
            .await
            .expect("activate_and_persist must succeed");

            // Publish a valid workflow definition
            let def = minimal_workflow_def(workflow_id, trigger_id.clone());
            runtime.activate(&scope, def).await;

            let trigger_uuid = handle.trigger_uuid;
            let nonce = handle.nonce.clone();

            Self {
                transport,
                version_store,
                exec_store,
                outcome_cell,
                scope,
                workflow_id,
                trigger_uuid,
                nonce,
            }
        }

        fn key(&self) -> WebhookKey {
            WebhookKey::programmatic(self.trigger_uuid, self.nonce.clone())
        }

        /// The durable side effect: execution rows materialized under the
        /// fixture's tenant scope. Asserting this (not just the HTTP status)
        /// proves Prod Emit spawns exactly one, Test/Skip spawn zero, and
        /// redelivery deduplicates to one.
        async fn execution_count(&self) -> u64 {
            self.exec_store.count(&self.scope, None).await.unwrap()
        }

        async fn assert_exact_start(&self, delivery_id: &str) {
            use nebula_storage_port::store::{
                ControlQueue, JobDispatchQueue, StartAcceptanceStore,
            };

            let starts = nebula_storage::inmem::InMemoryStartAcceptanceStore::new(&self.exec_store);
            let trigger =
                nebula_storage_port::dto::TriggerStartKey::new("webhook_trigger", delivery_id);
            let execution_id = starts
                .lookup_trigger_start(&self.scope, &trigger)
                .await
                .unwrap()
                .unwrap();
            let bundle = starts
                .read_contract_bundle(&self.scope, &execution_id)
                .await
                .unwrap()
                .unwrap();
            let revisions = bundle.record().identity().revisions();
            let execution = self
                .exec_store
                .get(&self.scope, &execution_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                execution.state["executable_plan_revision_id"],
                json!(revisions.plan())
            );
            assert_eq!(
                execution.state["worker_flavor_revision_id"],
                json!(revisions.worker_flavor())
            );
            let controls = nebula_storage::InMemoryControlQueue::new(&self.exec_store)
                .claim_pending_for_flavor(&[7; 16], 10, revisions.worker_flavor())
                .await
                .unwrap();
            assert_eq!(
                controls.len(),
                1,
                "one Start backed by the persisted exact flavor reference"
            );
            assert_eq!(controls[0].msg.execution_id, execution_id);
            assert_eq!(controls[0].msg.scope, self.scope);
            assert_eq!(
                controls[0].msg.command,
                nebula_storage_port::dto::ControlCommand::Start
            );
            let jobs = nebula_storage::inmem::InMemoryJobDispatchQueue::new(&self.exec_store)
                .claim_pending(
                    &[8; 16],
                    10,
                    &[
                        nebula_core::plugin_key!("core"),
                        nebula_core::plugin_key!("test"),
                    ],
                    revisions.worker_flavor(),
                )
                .await
                .unwrap();
            assert!(
                jobs.is_empty(),
                "webhook admission must not also enqueue a Job"
            );
        }

        /// Dispatch with a fully-signed SWH request including `webhook-id`.
        ///
        /// The body is `{}`.  The headers include `webhook-id`,
        /// `webhook-timestamp` (current wall time), and a valid
        /// `webhook-signature` computed with [`TEST_SWH_SECRET`].
        async fn dispatch_with_id(&self, delivery_id: &str) -> Response {
            let body = b"{}";
            let headers = swh_headers(delivery_id, body);
            dispatch_inner(
                self.transport.clone(),
                self.key(),
                Method::POST,
                Uri::from_static("http://localhost/webhooks/test"),
                headers,
                Bytes::from(body as &[u8]),
            )
            .await
        }

        /// Dispatch with `webhook-timestamp` + `webhook-signature` but WITHOUT
        /// `webhook-id`.
        ///
        /// With StandardWebhooks, the absence of `webhook-id` means the
        /// signed content cannot be constructed → `SignatureOutcome::Missing`
        /// → 401 because signature validation runs before durable emission.
        async fn dispatch_without_id(&self) -> Response {
            // Only timestamp + signature, no id — SWH will return Missing → 401.
            let ts = now_secs();
            let mut headers = HeaderMap::new();
            headers.insert(
                HeaderName::from_static("webhook-timestamp"),
                HeaderValue::from_str(&ts.to_string()).unwrap(),
            );
            // Omit webhook-signature too: no id → no canonical content → no sig.
            dispatch_inner(
                self.transport.clone(),
                self.key(),
                Method::POST,
                Uri::from_static("http://localhost/webhooks/test"),
                headers,
                Bytes::from(b"{}" as &[u8]),
            )
            .await
        }

        /// Dispatch without ANY SWH headers.  For tests that expect the raw
        /// behavior when neither id nor timestamp is present (e.g., Test-mode
        /// routes with `OptionalAcceptUnsigned`).
        async fn dispatch_bare(&self) -> Response {
            dispatch_inner(
                self.transport.clone(),
                self.key(),
                Method::POST,
                Uri::from_static("http://localhost/webhooks/test"),
                HeaderMap::new(),
                Bytes::from(b"{}" as &[u8]),
            )
            .await
        }

        fn set_outcome(&self, o: TriggerEventOutcome) {
            *self.outcome_cell.lock() = o;
        }
    }

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn base_ctx(
        workflow_id: WorkflowId,
        node_key: NodeKey,
    ) -> nebula_action::TriggerRuntimeContext {
        nebula_action::TriggerRuntimeContext::new(
            Arc::new(
                BaseContext::builder(nebula_core::scope::Scope::default())
                    .principal(Principal::System)
                    .cancellation(CancellationToken::new())
                    .build()
                    .expect("scope + principal must produce a valid BaseContext"),
            ),
            workflow_id,
            node_key,
        )
    }

    /// Build the minimal valid `WorkflowDefinition` referencing `trigger_id`
    /// as the trigger binding.  The trigger node itself does not appear in
    /// `nodes` (trigger bindings are separate in the definition schema).
    fn minimal_workflow_def(workflow_id: WorkflowId, trigger_id: String) -> WorkflowDefinition {
        let trigger_key: NodeKey = trigger_id.parse().unwrap_or_else(|_| node_key!("t"));
        WorkflowBuilder::new("test-workflow")
            .id(workflow_id)
            .add_node(NodeDefinition::new(node_key!("step"), "Step", "core", "echo").unwrap())
            .add_trigger(
                trigger_key,
                "test".parse().unwrap(),
                "webhook".parse().unwrap(),
                json!({}),
            )
            .build()
            .expect("minimal workflow must be valid")
    }

    // ── Prod-mode emission ────────────────────────────────────────────────────

    /// Prod-mode activation + valid SWH `webhook-id` header → emitter is
    /// called. Verify the response is 200 OK and one execution was spawned.
    ///
    /// The fixture requires a Standard Webhooks signature and wires durable
    /// dispatch, so both the acknowledgement and persisted execution are checked.
    #[tokio::test]
    async fn prod_mode_emit_returns_200() {
        let fix = TestFixture::prod(WebhookMode::Prod).await;
        let resp = fix.dispatch_with_id("delivery-001").await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "Prod emit must return 200 (Emit + durable inbox, SWH-signed)"
        );
        assert_eq!(
            fix.execution_count().await,
            1,
            "Prod Emit must materialize exactly one execution (durable side effect, not just 200)"
        );
        fix.assert_exact_start("delivery-001").await;
    }

    // ── Test-mode dispatch ────────────────────────────────────────────────────

    /// mode=Test → the durable path is NEVER taken even when `durable_dispatch`
    /// is wired.  The fallthrough in-memory path returns 200.
    ///
    /// Test-mode activations use `OptionalAcceptUnsigned`, so requests can be
    /// sent without a signature.
    ///
    /// Red-on-revert: removing the `row.mode == WebhookMode::Prod` guard would
    /// cause Test-mode activations to take the durable path, which would
    /// fail (inbox claim) OR spawn unexpectedly.
    #[tokio::test]
    async fn test_mode_skips_durable_dispatch_returns_200() {
        let fix = TestFixture::prod(WebhookMode::Test).await;
        // Test mode permits an unsigned request.
        let resp = fix.dispatch_bare().await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "Test-mode must return 200 via fallthrough path"
        );
        assert_eq!(
            fix.execution_count().await,
            0,
            "Test mode must spawn NO execution (durable side effect, not just 200)"
        );
    }

    // ── Redelivery deduplication ──────────────────────────────────────────────

    /// Delivering the same `webhook-id` twice with valid SWH signatures: the
    /// first call spawns the execution; the second is deduplicated by the inbox.
    /// Both should return 200 (the dedup is at the storage layer; the HTTP ack
    /// is always 200).
    ///
    /// This proves the dedup PK contract: `(scope, trigger_id, event_id)`.
    #[tokio::test]
    async fn redelivery_deduplicates() {
        let fix = TestFixture::prod(WebhookMode::Prod).await;
        let r1 = fix.dispatch_with_id("delivery-003").await;
        let r2 = fix.dispatch_with_id("delivery-003").await;
        assert_eq!(r1.status(), StatusCode::OK, "first delivery must succeed");
        assert_eq!(
            r2.status(),
            StatusCode::OK,
            "redelivery (dedup) must also succeed"
        );
        assert_eq!(
            fix.execution_count().await,
            1,
            "redelivery with the same webhook-id must dedup to exactly ONE execution, not two"
        );
        fix.assert_exact_start("delivery-003").await;
    }

    // ── Delivery identity validation ──────────────────────────────────────────

    /// Prod-mode activation without `webhook-id` header → 401 (SWH signature
    /// missing, because the id is part of the signed content and cannot be
    /// constructed without it).
    ///
    /// Signature validation runs first: without the delivery id there is no
    /// signed content, so the request never reaches durable emission.
    #[tokio::test]
    async fn prod_mode_missing_webhook_id_returns_401() {
        let fix = TestFixture::prod(WebhookMode::Prod).await;
        // dispatch_without_id: has webhook-timestamp but no webhook-id or
        // webhook-signature → SWH can't build signed content → Missing → 401.
        let resp = fix.dispatch_without_id().await;
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Prod-mode without webhook-id must return 401 (SWH signature missing)"
        );
        assert_eq!(
            fix.execution_count().await,
            0,
            "sig-failed request must not spawn any execution"
        );
    }

    /// An over-long `webhook-id` header is rejected at the edge with a clean
    /// 400 (overflow guard fires before signature enforcement — the id
    /// extraction happens before `WebhookRequest::try_new` and signature
    /// check, so the 400 is independent of SWH policy).
    #[tokio::test]
    async fn prod_mode_oversized_webhook_id_returns_400() {
        let fix = TestFixture::prod(WebhookMode::Prod).await;
        let oversized = "x".repeat(300);
        // dispatch_with_id includes valid SWH headers, but the oversized-id
        // guard (step 7) fires before signature enforcement (step 5.5).
        let resp = fix.dispatch_with_id(&oversized).await;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "oversized webhook-id must be a clean 400, not a backend 5xx"
        );
    }

    /// Two `webhook-id` headers → 400. `HeaderMap::get` would silently pick one,
    /// letting conflicting delivery ids bypass dedup; reject the ambiguity.
    ///
    /// The duplicate-id check fires at step 7 (header extraction), before
    /// signature enforcement (step 5.5), so this returns 400 regardless of
    /// the SWH policy.
    #[tokio::test]
    async fn duplicate_webhook_id_headers_returns_400() {
        let fix = TestFixture::prod(WebhookMode::Prod).await;
        let mut headers = HeaderMap::new();
        headers.append(&WEBHOOK_ID_HEADER, HeaderValue::from_static("delivery-a"));
        headers.append(&WEBHOOK_ID_HEADER, HeaderValue::from_static("delivery-b"));
        let resp = dispatch_inner(
            fix.transport.clone(),
            fix.key(),
            Method::POST,
            Uri::from_static("http://localhost/webhooks/test"),
            headers,
            Bytes::from(b"{}" as &[u8]),
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "duplicate webhook-id headers must be a clean 400"
        );
        assert_eq!(
            fix.execution_count().await,
            0,
            "duplicate webhook-id must spawn nothing"
        );
    }

    /// A Prod verification probe (`Skip` outcome) with a valid SWH signature
    /// must return 200 — the dedup key (`webhook-id`) is present (SWH requires
    /// it for signing), and the dedup requirement applies only to `Emit` outcomes.
    /// A verification probe from a
    /// well-behaved provider will include a valid SWH signature; the probe's
    /// `Skip` outcome does NOT hit the "missing webhook-id" dedup guard because
    /// that guard is in the `Emit` arm of `dispatch_durable`, not before.
    #[tokio::test]
    async fn prod_mode_skip_with_signed_id_returns_200() {
        let fix = TestFixture::prod(WebhookMode::Prod).await;
        fix.set_outcome(TriggerEventOutcome::skip());
        // Use dispatch_with_id: provides webhook-id + timestamp + valid sig.
        let resp = fix.dispatch_with_id("probe-001").await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "Prod Skip with valid SWH signature must return 200 (no dedup-key guard on Skip)"
        );
        assert_eq!(
            fix.execution_count().await,
            0,
            "Skip must spawn NO execution"
        );
    }

    /// A transient activation-store outage on the durable read path must fail
    /// closed with 503 (sender retries) rather than silently downgrade a Prod
    /// trigger to the Noop path and return 2xx, which would lose the event
    /// without a retry.
    ///
    /// Red-on-revert: the old `Err(_) => fall-through` arm returned the
    /// handler's 200, hiding the store failure.
    #[tokio::test]
    async fn resolve_store_error_returns_503() {
        let failing: Arc<dyn WebhookActivationStore> =
            Arc::new(FailingResolveStore(InMemoryWebhookActivationStore::new()));
        let fix = TestFixture::prod_with_store(WebhookMode::Prod, failing).await;
        let resp = fix.dispatch_with_id("delivery-503").await;
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "resolve_by_token outage must fail closed 503, not downgrade to 200"
        );
    }

    // ── Unsupported fan-out ───────────────────────────────────────────────────

    /// `EmitMany` outcome in Prod mode → fail-closed 500 (with valid SWH sig).
    ///
    /// One `event_id` cannot safely fan-out to N executions (dedup-collision
    /// data-loss bug). No first-party webhook action returns EmitMany; if one
    /// does, the operator must fix the action.
    ///
    /// Red-on-revert: removing the `EmitMany => 500` arm would allow partial
    /// fan-out under one event_id, silently losing all but one execution.
    #[tokio::test]
    async fn prod_mode_emit_many_returns_500() {
        let fix = TestFixture::prod(WebhookMode::Prod).await;
        fix.set_outcome(TriggerEventOutcome::emit_many(vec![json!(1), json!(2)]));
        let resp = fix.dispatch_with_id("delivery-004").await;
        assert_eq!(
            resp.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "EmitMany in Prod must be 500"
        );
    }

    // ── Skipped delivery ──────────────────────────────────────────────────────

    /// `Skip` outcome in Prod mode → no execution spawned, HTTP 200.
    ///
    /// The adapter sends the HTTP response before returning; `dispatch_durable`
    /// reads the oneshot and returns 200 without hitting the inbox.
    #[tokio::test]
    async fn prod_mode_skip_returns_200_no_spawn() {
        let fix = TestFixture::prod(WebhookMode::Prod).await;
        fix.set_outcome(TriggerEventOutcome::skip());
        // Signed request with a valid webhook-id.
        let resp = fix.dispatch_with_id("delivery-005").await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "Skip in Prod must return 200"
        );
        assert_eq!(
            fix.execution_count().await,
            0,
            "Skip must spawn NO execution (durable side effect, not just 200)"
        );
    }

    // ── Corrupt activation identity ───────────────────────────────────────────

    /// Prod-mode activation with `workflow_id = None` on the row → fail-closed 500.
    ///
    /// This covers inv #5: a Prod activation without a wired workflow_id must
    /// never spawn a dedup-blind execution.
    #[tokio::test]
    async fn prod_mode_no_workflow_id_returns_500() {
        // Build a transport with a Prod-mode row that has workflow_id = None.
        let scope = tenant_scope();
        let workflow_id = WorkflowId::new();
        let trigger_id = node_key!("webhook_trigger").as_str().to_string();

        let exec_store = InMemoryExecutionStore::new();
        let version_store: Arc<InMemoryWorkflowVersionStore> =
            Arc::new(InMemoryWorkflowVersionStore::new());
        let runtime = RuntimeFixture::new(&exec_store, &version_store);
        let activation_store: Arc<dyn WebhookActivationStore> =
            Arc::new(InMemoryWebhookActivationStore::new());

        let outcome_cell = Arc::new(Mutex::new(TriggerEventOutcome::emit(json!({"ok": true}))));
        let handler: Arc<dyn TriggerHandler> = Arc::new(
            WebhookTriggerAdapter::new(ConfigurableWebhookAction {
                outcome_cell: Arc::clone(&outcome_cell),
            })
            .expect("valid test catalog definition"),
        );
        let ctx_template = base_ctx(workflow_id, node_key!("webhook_trigger"));
        handler
            .start(&ctx_template)
            .await
            .expect("handler start must succeed");

        let transport = WebhookTransport::new(WebhookTransportConfig::default())
            .with_activation_store(Arc::clone(&activation_store))
            .with_durable_dispatch(runtime.starts.clone());

        let handle = activate_and_persist(
            &transport,
            activation_store.as_ref(),
            PersistParams {
                handler,
                action_config: WebhookConfig::default()
                    .with_signature_policy(swh_required_policy()),
                ctx_template,
                spec_trigger_id: format!("trg_test_{trigger_id}"),
                trigger_id,
                scope: scope.clone(),
                // Deliberately None — no workflow wired.
                workflow_id: None,
                mode: WebhookMode::Prod,
            },
        )
        .await
        .expect("activate_and_persist must succeed");

        let key = WebhookKey::programmatic(handle.trigger_uuid, handle.nonce.clone());
        let body = b"{}";
        let headers = swh_headers("delivery-006", body);
        let resp = dispatch_inner(
            transport,
            key,
            Method::POST,
            Uri::from_static("http://localhost/webhooks/test"),
            headers,
            Bytes::from(body as &[u8]),
        )
        .await;

        assert_eq!(
            resp.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "Prod-mode with workflow_id=None must be 500"
        );
    }

    // ── Tenant isolation ──────────────────────────────────────────────────────

    /// Prod-mode activation whose activation-row scope is scope_A, while the
    /// workflow version exists only under scope_B.
    ///
    /// `WorkflowStartService` pins the lookup to `row.scope` (scope_A), so the
    /// activated workflow under scope_B remains invisible. The webhook surface
    /// deliberately collapses that internal admission failure to a payload-free
    /// 5xx so the sender retries without learning whether the workflow exists.
    ///
    /// Passing the request-derived scope or omitting scoping from
    /// workflow lookup would let a cross-scope lookup succeed, violating the
    /// tenant boundary.
    #[tokio::test]
    async fn tenant_isolation_wrong_scope_is_hidden_behind_retryable_failure() {
        let scope_a = tenant_scope();
        let scope_b = tenant_scope();
        let workflow_id = WorkflowId::new();
        let trigger_id = node_key!("webhook_trigger").as_str().to_string();

        let exec_store = InMemoryExecutionStore::new();
        let version_store = Arc::new(InMemoryWorkflowVersionStore::new());
        let runtime = RuntimeFixture::new(&exec_store, &version_store);
        let activation_store: Arc<dyn WebhookActivationStore> =
            Arc::new(InMemoryWebhookActivationStore::new());

        let outcome_cell = Arc::new(Mutex::new(TriggerEventOutcome::emit(json!({"ok": true}))));
        let handler: Arc<dyn TriggerHandler> = Arc::new(
            WebhookTriggerAdapter::new(ConfigurableWebhookAction {
                outcome_cell: Arc::clone(&outcome_cell),
            })
            .expect("valid test catalog definition"),
        );
        let ctx_template_a = base_ctx(workflow_id, node_key!("webhook_trigger"));
        handler
            .start(&ctx_template_a)
            .await
            .expect("handler start must succeed");

        let transport = WebhookTransport::new(WebhookTransportConfig::default())
            .with_activation_store(Arc::clone(&activation_store))
            .with_durable_dispatch(runtime.starts.clone());

        // Activation row registered under scope_a, with SWH policy (Prod).
        let handle = activate_and_persist(
            &transport,
            activation_store.as_ref(),
            PersistParams {
                handler,
                action_config: WebhookConfig::default()
                    .with_signature_policy(swh_required_policy()),
                ctx_template: ctx_template_a,
                trigger_id: trigger_id.clone(),
                spec_trigger_id: format!("trg_test_{trigger_id}"),
                scope: scope_a.clone(),
                workflow_id: Some(workflow_id.to_string()),
                mode: WebhookMode::Prod,
            },
        )
        .await
        .expect("activate_and_persist");

        // Workflow stored under scope_b — should NOT be visible to scope_a lookup.
        let def = minimal_workflow_def(workflow_id, trigger_id);
        runtime.activate(&scope_b, def).await;

        let key = WebhookKey::programmatic(handle.trigger_uuid, handle.nonce.clone());
        let body = b"{}";
        let headers = swh_headers("delivery-007", body);
        let resp = dispatch_inner(
            transport,
            key,
            Method::POST,
            Uri::from_static("http://localhost/webhooks/test"),
            headers,
            Bytes::from(body as &[u8]),
        )
        .await;

        let status = resp.status();
        let response_body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "workflow stored in another scope must stay hidden behind a bare retryable failure: {}",
            String::from_utf8_lossy(&response_body)
        );
        assert!(
            response_body.is_empty(),
            "internal identifiers must not be returned"
        );
        assert_eq!(exec_store.count(&scope_a, None).await.unwrap(), 0);
        assert_eq!(exec_store.count(&scope_b, None).await.unwrap(), 0);
        use nebula_storage_port::store::{ControlQueue, StartAcceptanceStore};
        let controls = nebula_storage::InMemoryControlQueue::new(&exec_store);
        assert!(
            controls
                .claim_pending(&[9; 16], 10)
                .await
                .unwrap()
                .is_empty()
        );
        let starts = nebula_storage::inmem::InMemoryStartAcceptanceStore::new(&exec_store);
        let trigger =
            nebula_storage_port::dto::TriggerStartKey::new("webhook_trigger", "delivery-007");
        for scope in [&scope_a, &scope_b] {
            assert!(
                starts
                    .lookup_trigger_start(scope, &trigger)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    }

    // ── Rate-limit tier tests ─────────────────────────────────────────────────

    /// Helper: build a transport with a *very* tight per-tenant limit (1 req/min)
    /// but a generous per-token limit (1 000 req/min), then register two activations
    /// under the same scope with distinct tokens.  Returns (transport, key_a, key_b,
    /// activation_store) so callers can drive dispatches.
    async fn two_token_same_scope_fixture(
        tenant_rpm: u64,
        per_token_rpm: u64,
    ) -> (
        WebhookTransport,
        WebhookKey,
        WebhookKey,
        Arc<dyn WebhookActivationStore>,
    ) {
        let scope = tenant_scope();
        let activation_store: Arc<dyn WebhookActivationStore> =
            Arc::new(InMemoryWebhookActivationStore::new());
        let exec_store = InMemoryExecutionStore::new();
        let version_store = Arc::new(InMemoryWorkflowVersionStore::new());
        let runtime = RuntimeFixture::new(&exec_store, &version_store);

        // Build transport with explicit tight limits so the test is deterministic.
        let cfg = WebhookTransportConfig {
            rate_limit_per_minute: Some(per_token_rpm),
            tenant_rate_limit_per_minute: Some(tenant_rpm),
            ..WebhookTransportConfig::default()
        };
        let transport = WebhookTransport::new(cfg)
            .with_activation_store(Arc::clone(&activation_store))
            .with_durable_dispatch(runtime.starts.clone());

        // Register activation A under the shared scope.
        let outcome_a = Arc::new(Mutex::new(TriggerEventOutcome::skip()));
        let handler_a: Arc<dyn TriggerHandler> = Arc::new(
            WebhookTriggerAdapter::new(ConfigurableWebhookAction {
                outcome_cell: Arc::clone(&outcome_a),
            })
            .expect("valid test catalog definition"),
        );
        let wf_a = WorkflowId::new();
        let ctx_a = base_ctx(wf_a, node_key!("trigger_a"));
        handler_a.start(&ctx_a).await.unwrap();
        let handle_a = activate_and_persist(
            &transport,
            activation_store.as_ref(),
            PersistParams {
                handler: handler_a,
                action_config: WebhookConfig::default()
                    .with_signature_policy(SignaturePolicy::OptionalAcceptUnsigned),
                ctx_template: ctx_a,
                trigger_id: node_key!("trigger_a").as_str().to_string(),
                spec_trigger_id: "trg_test_trigger_a".to_string(),
                scope: scope.clone(),
                workflow_id: Some(wf_a.to_string()),
                mode: WebhookMode::Test, // Test mode: no durable emit needed
            },
        )
        .await
        .unwrap();

        // Register activation B — different token, same scope.
        let outcome_b = Arc::new(Mutex::new(TriggerEventOutcome::skip()));
        let handler_b: Arc<dyn TriggerHandler> = Arc::new(
            WebhookTriggerAdapter::new(ConfigurableWebhookAction {
                outcome_cell: Arc::clone(&outcome_b),
            })
            .expect("valid test catalog definition"),
        );
        let wf_b = WorkflowId::new();
        let ctx_b = base_ctx(wf_b, node_key!("trigger_b"));
        handler_b.start(&ctx_b).await.unwrap();
        let handle_b = activate_and_persist(
            &transport,
            activation_store.as_ref(),
            PersistParams {
                handler: handler_b,
                action_config: WebhookConfig::default()
                    .with_signature_policy(SignaturePolicy::OptionalAcceptUnsigned),
                ctx_template: ctx_b,
                trigger_id: node_key!("trigger_b").as_str().to_string(),
                spec_trigger_id: "trg_test_trigger_b".to_string(),
                scope: scope.clone(),
                workflow_id: Some(wf_b.to_string()),
                mode: WebhookMode::Test,
            },
        )
        .await
        .unwrap();

        let key_a = WebhookKey::programmatic(handle_a.trigger_uuid, handle_a.nonce);
        let key_b = WebhookKey::programmatic(handle_b.trigger_uuid, handle_b.nonce);
        (transport, key_a, key_b, activation_store)
    }

    async fn dispatch_skip(
        transport: &WebhookTransport,
        key: WebhookKey,
    ) -> axum::http::StatusCode {
        dispatch_inner(
            transport.clone(),
            key,
            Method::POST,
            Uri::from_static("http://localhost/webhooks/test"),
            HeaderMap::new(),
            Bytes::from(b"{}" as &[u8]),
        )
        .await
        .status()
    }

    /// Per-tenant-aggregate 429.
    ///
    /// Two activations share the SAME scope (tenant).  The per-tenant quota
    /// is 1 req/min; per-token quota is generous (1 000 req/min).  The first
    /// request from token A succeeds (per-token OK, per-tenant OK).  The
    /// second request — from token B, a DIFFERENT token, so per-token is
    /// fresh — must be rate-limited at the per-tenant tier and return 429.
    ///
    /// Without the per-tenant limiter, both requests
    /// succeed (200) because each token is within its own per-token quota.
    #[tokio::test]
    async fn per_tenant_aggregate_429() {
        // per-tenant limit = 1 req/min; per-token limit = 1 000 req/min
        let (transport, key_a, key_b, _store) = two_token_same_scope_fixture(1, 1_000).await;

        // First request (token A) — should succeed.
        let r1 = dispatch_skip(&transport, key_a).await;
        assert_eq!(r1, StatusCode::OK, "first request (token A) must succeed");

        // Second request (token B) — different token, fresh per-token window,
        // but the tenant aggregate was exhausted by the first request.
        let r2 = dispatch_skip(&transport, key_b).await;
        assert_eq!(
            r2,
            StatusCode::TOO_MANY_REQUESTS,
            "second request under a different token but same tenant must be 429 \
             (per-tenant-aggregate limit exceeded)"
        );
    }

    /// Per-token quotas remain independent across different scopes (tenants).
    ///
    /// Two tokens in DIFFERENT scopes each start with a fresh per-token window
    /// and a fresh per-tenant aggregate — no cross-contamination.
    ///
    /// Keying the per-tenant limiter by a constant
    /// or by the trigger UUID would cause cross-scope contamination.
    #[tokio::test]
    async fn per_token_independent_scopes() {
        let scope_x = tenant_scope();
        let scope_y = tenant_scope();
        let activation_store: Arc<dyn WebhookActivationStore> =
            Arc::new(InMemoryWebhookActivationStore::new());
        let exec_store = InMemoryExecutionStore::new();
        let version_store = Arc::new(InMemoryWorkflowVersionStore::new());
        let runtime = RuntimeFixture::new(&exec_store, &version_store);

        // 1 req/min per-tenant; 1 000 per-token — tight enough to prove isolation.
        let cfg = WebhookTransportConfig {
            rate_limit_per_minute: Some(1_000),
            tenant_rate_limit_per_minute: Some(1),
            ..WebhookTransportConfig::default()
        };
        let transport = WebhookTransport::new(cfg)
            .with_activation_store(Arc::clone(&activation_store))
            .with_durable_dispatch(runtime.starts.clone());

        // Register one activation in scope_x.
        let outcome_x = Arc::new(Mutex::new(TriggerEventOutcome::skip()));
        let handler_x: Arc<dyn TriggerHandler> = Arc::new(
            WebhookTriggerAdapter::new(ConfigurableWebhookAction {
                outcome_cell: Arc::clone(&outcome_x),
            })
            .expect("valid test catalog definition"),
        );
        let wf_x = WorkflowId::new();
        let ctx_x = base_ctx(wf_x, node_key!("trigger_x"));
        handler_x.start(&ctx_x).await.unwrap();
        let handle_x = activate_and_persist(
            &transport,
            activation_store.as_ref(),
            PersistParams {
                handler: handler_x,
                action_config: WebhookConfig::default()
                    .with_signature_policy(SignaturePolicy::OptionalAcceptUnsigned),
                ctx_template: ctx_x,
                trigger_id: node_key!("trigger_x").as_str().to_string(),
                spec_trigger_id: "trg_test_trigger_x".to_string(),
                scope: scope_x.clone(),
                workflow_id: Some(wf_x.to_string()),
                mode: WebhookMode::Test,
            },
        )
        .await
        .unwrap();

        // Register one activation in scope_y.
        let outcome_y = Arc::new(Mutex::new(TriggerEventOutcome::skip()));
        let handler_y: Arc<dyn TriggerHandler> = Arc::new(
            WebhookTriggerAdapter::new(ConfigurableWebhookAction {
                outcome_cell: Arc::clone(&outcome_y),
            })
            .expect("valid test catalog definition"),
        );
        let wf_y = WorkflowId::new();
        let ctx_y = base_ctx(wf_y, node_key!("trigger_y"));
        handler_y.start(&ctx_y).await.unwrap();
        let handle_y = activate_and_persist(
            &transport,
            activation_store.as_ref(),
            PersistParams {
                handler: handler_y,
                action_config: WebhookConfig::default()
                    .with_signature_policy(SignaturePolicy::OptionalAcceptUnsigned),
                ctx_template: ctx_y,
                trigger_id: node_key!("trigger_y").as_str().to_string(),
                spec_trigger_id: "trg_test_trigger_y".to_string(),
                scope: scope_y.clone(),
                workflow_id: Some(wf_y.to_string()),
                mode: WebhookMode::Test,
            },
        )
        .await
        .unwrap();

        let key_x = WebhookKey::programmatic(handle_x.trigger_uuid, handle_x.nonce.clone());
        let key_y = WebhookKey::programmatic(handle_y.trigger_uuid, handle_y.nonce.clone());

        // Each tenant gets one allowed request — they should NOT interfere.
        let r_x = dispatch_skip(&transport, key_x).await;
        let r_y = dispatch_skip(&transport, key_y).await;
        assert_eq!(r_x, StatusCode::OK, "scope_x first request must succeed");
        assert_eq!(
            r_y,
            StatusCode::OK,
            "scope_y first request must succeed independently of scope_x"
        );
    }

    /// Structural check that durable dispatch installs both limiters.
    ///
    /// A transport built from a `None`-rate-limit config and then passed to
    /// `with_durable_dispatch` must have BOTH `rate_limiter` and
    /// `tenant_rate_limiter` populated — the defaults are installed
    /// automatically by the builder, not by composition-root discipline.
    ///
    /// Removing the `if i.rate_limiter.is_none()` /
    /// `if i.tenant_rate_limiter.is_none()` installs in `with_durable_dispatch`
    /// causes both `is_some()` assertions to fail.
    #[tokio::test]
    async fn structural_coupling_durable_dispatch_installs_both_limiters() {
        let exec_store = InMemoryExecutionStore::new();
        let version_store: Arc<InMemoryWorkflowVersionStore> =
            Arc::new(InMemoryWorkflowVersionStore::new());
        let runtime = RuntimeFixture::new(&exec_store, &version_store);

        // Config with both rate limits as None — no explicit operator override.
        let cfg = WebhookTransportConfig {
            rate_limit_per_minute: None,
            tenant_rate_limit_per_minute: None,
            ..WebhookTransportConfig::default()
        };
        let transport = WebhookTransport::new(cfg).with_durable_dispatch(runtime.starts);

        assert!(
            transport.inner.rate_limiter.is_some(),
            "with_durable_dispatch must install per-token rate_limiter even when \
             config.rate_limit_per_minute is None (structural guarantee)"
        );
        assert!(
            transport.inner.tenant_rate_limiter.is_some(),
            "with_durable_dispatch must install per-tenant tenant_rate_limiter even when \
             config.tenant_rate_limit_per_minute is None (structural guarantee)"
        );
    }

    // ── F: structural_coupling behavioral — per-token rate enforced at DEFAULT_PER_TOKEN_RPM ─
    //
    // The `structural_coupling_durable_dispatch_installs_both_limiters` test above proves
    // `is_some()`.  This test proves the installed limiter is *behaviorally* active: a
    // transport with `rate_limit_per_minute = Some(2)` allows exactly 2 requests then
    // rejects the 3rd with 429.
    //
    // An absent default limiter would make the second request succeed.
    // → no limiter → all requests succeed → 429 assertion fails.

    /// The per-token limiter installed by `with_durable_dispatch` actually
    /// rejects requests once the quota is exhausted.
    ///
    /// Uses `rate_limit_per_minute: Some(2)` for a deterministic in-test
    /// quota (driving `DEFAULT_PER_TOKEN_RPM` = 600 requests is impractical).
    /// The constant value is verified by a separate assertion to pin the default.
    #[tokio::test]
    async fn structural_coupling_per_token_limiter_is_behaviorally_active() {
        // Pin the default value so a silent change is caught here.
        assert_eq!(
            DEFAULT_PER_TOKEN_RPM, 600,
            "DEFAULT_PER_TOKEN_RPM must be 600"
        );

        let exec_store = InMemoryExecutionStore::new();
        let version_store: Arc<InMemoryWorkflowVersionStore> =
            Arc::new(InMemoryWorkflowVersionStore::new());
        let runtime = RuntimeFixture::new(&exec_store, &version_store);
        let activation_store: Arc<dyn WebhookActivationStore> =
            Arc::new(InMemoryWebhookActivationStore::new());

        // Tight per-token quota (2 req/min) for a fast, deterministic test.
        let cfg = WebhookTransportConfig {
            rate_limit_per_minute: Some(2),
            tenant_rate_limit_per_minute: Some(10_000), // generous so per-tenant does not fire
            ..WebhookTransportConfig::default()
        };
        let transport = WebhookTransport::new(cfg)
            .with_activation_store(Arc::clone(&activation_store))
            .with_durable_dispatch(runtime.starts.clone());

        // Register one Test-mode activation (OptionalAcceptUnsigned, no SWH needed).
        let outcome = Arc::new(Mutex::new(TriggerEventOutcome::skip()));
        let handler: Arc<dyn TriggerHandler> = Arc::new(
            WebhookTriggerAdapter::new(ConfigurableWebhookAction {
                outcome_cell: Arc::clone(&outcome),
            })
            .expect("valid test catalog definition"),
        );
        let wf = WorkflowId::new();
        let ctx = base_ctx(wf, node_key!("trigger_rl"));
        handler.start(&ctx).await.unwrap();
        let handle = activate_and_persist(
            &transport,
            activation_store.as_ref(),
            PersistParams {
                handler,
                action_config: WebhookConfig::default()
                    .with_signature_policy(SignaturePolicy::OptionalAcceptUnsigned),
                ctx_template: ctx,
                trigger_id: node_key!("trigger_rl").as_str().to_string(),
                spec_trigger_id: "trg_test_trigger_rl".to_string(),
                scope: tenant_scope(),
                workflow_id: Some(wf.to_string()),
                mode: WebhookMode::Test,
            },
        )
        .await
        .unwrap();
        let key = WebhookKey::programmatic(handle.trigger_uuid, handle.nonce);

        // First two requests must pass (quota = 2).
        assert_eq!(
            dispatch_skip(&transport, key.clone()).await,
            StatusCode::OK,
            "first request within quota must succeed"
        );
        assert_eq!(
            dispatch_skip(&transport, key.clone()).await,
            StatusCode::OK,
            "second request within quota must succeed"
        );
        // Third request must be rate-limited.
        assert_eq!(
            dispatch_skip(&transport, key).await,
            StatusCode::TOO_MANY_REQUESTS,
            "third request over per-token quota must return 429 \
             (the second request must remain rate limited)"
        );
    }

    // ── E: with_activation_store preserves both rate limiters (structural) ───
    //
    // The fast path (`Arc::try_unwrap` succeeds) moves `TransportInner` in-place
    // and does not touch the rate limiters — correct by construction.
    //
    // The slow path (shared Arc, refcount > 1) previously dropped `rate_limiter`
    // while `tenant_rate_limiter` had an `.or_else()` fallback.  The fix adds
    // `.or_else(|| arc.rate_limiter.clone())` to the slow path too.
    //
    // The slow path is guarded by `debug_assert!(false, …)` so it cannot be
    // triggered in test/debug builds without a panic.  This test verifies the
    // structural invariant instead: the normal fast-path composition
    // `.with_durable_dispatch(runtime.starts.clone()).with_activation_store(...)` must leave BOTH
    // limiters populated, and must enforce them behaviorally.
    //
    // The fast path preserves limiters because
    // `with_activation_store` mutates the existing `TransportInner` field —
    // it does NOT construct a new one, so there is nothing to revert on the fast
    // path.  The slow-path fix is a code-level defence for the rare misuse case
    // (composition-root ordering bug); its correctness is verified by code review.

    /// `with_durable_dispatch().with_activation_store()` (fast path) leaves
    /// both rate limiters intact and behaviorally enforced.
    #[tokio::test]
    async fn with_activation_store_fast_path_preserves_both_rate_limiters() {
        let exec_store = InMemoryExecutionStore::new();
        let version_store: Arc<InMemoryWorkflowVersionStore> =
            Arc::new(InMemoryWorkflowVersionStore::new());
        let runtime = RuntimeFixture::new(&exec_store, &version_store);
        let activation_store: Arc<dyn WebhookActivationStore> =
            Arc::new(InMemoryWebhookActivationStore::new());

        // Standard composition order: durable_dispatch first, then activation_store.
        let cfg = WebhookTransportConfig {
            rate_limit_per_minute: Some(1), // 1 req/min — fires on second dispatch
            tenant_rate_limit_per_minute: Some(10_000), // generous
            ..WebhookTransportConfig::default()
        };
        let transport = WebhookTransport::new(cfg)
            .with_durable_dispatch(runtime.starts.clone())
            .with_activation_store(Arc::clone(&activation_store));

        // Both limiters must be present.
        assert!(
            transport.inner.rate_limiter.is_some(),
            "with_activation_store must preserve the per-token rate_limiter \
             installed by with_durable_dispatch"
        );
        assert!(
            transport.inner.tenant_rate_limiter.is_some(),
            "with_activation_store must preserve the per-tenant rate_limiter \
             installed by with_durable_dispatch"
        );

        // Behavioral check: the preserved per-token limiter is actually enforced.
        let outcome = Arc::new(Mutex::new(TriggerEventOutcome::skip()));
        let handler: Arc<dyn TriggerHandler> = Arc::new(
            WebhookTriggerAdapter::new(ConfigurableWebhookAction {
                outcome_cell: Arc::clone(&outcome),
            })
            .expect("valid test catalog definition"),
        );
        let wf = WorkflowId::new();
        let ctx = base_ctx(wf, node_key!("trigger_e"));
        handler.start(&ctx).await.unwrap();
        let handle = activate_and_persist(
            &transport,
            activation_store.as_ref(),
            PersistParams {
                handler,
                action_config: WebhookConfig::default()
                    .with_signature_policy(SignaturePolicy::OptionalAcceptUnsigned),
                ctx_template: ctx,
                trigger_id: node_key!("trigger_e").as_str().to_string(),
                spec_trigger_id: "trg_test_trigger_e".to_string(),
                scope: tenant_scope(),
                workflow_id: Some(wf.to_string()),
                mode: WebhookMode::Test,
            },
        )
        .await
        .unwrap();
        let key = WebhookKey::programmatic(handle.trigger_uuid, handle.nonce);

        // First request within quota — must pass.
        assert_eq!(
            dispatch_skip(&transport, key.clone()).await,
            StatusCode::OK,
            "first request must succeed (per-token quota = 1, not yet exhausted)"
        );
        // Second request exceeds the 1 req/min quota — must be rejected.
        assert_eq!(
            dispatch_skip(&transport, key).await,
            StatusCode::TOO_MANY_REQUESTS,
            "second request must be rate-limited (429) — proves per-token limiter \
             is behaviorally active after with_activation_store"
        );
    }

    // ── Tenant rate limiting before signature enforcement ────────────────────
    //
    // Two Prod-mode activations share the same tenant scope, both using
    // `SignaturePolicy::Required` + StandardWebhooks.  The per-tenant quota is 1
    // req/min.  The first signed request succeeds.  The second signed request —
    // from a different token (fresh per-token quota) but the same tenant — must
    // return 429 from the tenant rate limit, before the signature policy check
    // at step 5.5, and NOT 200 (both exhausted).
    //
    // This proves the ordering: tenant-rate-limit check (step 4.5) comes before
    // and before durable dispatch.

    /// Two Prod+SWH activations under the same tenant: per-tenant aggregate
    /// The tenant limit returns 429 before signature enforcement.
    #[tokio::test]
    async fn prod_swh_per_tenant_429_fires_before_b2_guard() {
        let scope = tenant_scope();
        let activation_store: Arc<dyn WebhookActivationStore> =
            Arc::new(InMemoryWebhookActivationStore::new());
        let exec_store = InMemoryExecutionStore::new();
        let exec_store = Arc::new(exec_store);
        let version_store = Arc::new(InMemoryWorkflowVersionStore::new());
        let runtime = RuntimeFixture::new(&exec_store, &version_store);

        let cfg = WebhookTransportConfig {
            rate_limit_per_minute: Some(1_000),    // generous per-token
            tenant_rate_limit_per_minute: Some(1), // 1 req/min per tenant — fires on second
            ..WebhookTransportConfig::default()
        };
        let transport = WebhookTransport::new(cfg)
            .with_activation_store(Arc::clone(&activation_store))
            .with_durable_dispatch(runtime.starts.clone());

        // Register two Prod activations under the same scope.
        let mut handles = vec![];
        for (trig, wf_id) in [
            (node_key!("trigger_g1"), WorkflowId::new()),
            (node_key!("trigger_g2"), WorkflowId::new()),
        ] {
            let outcome = Arc::new(Mutex::new(TriggerEventOutcome::emit(json!({"g": true}))));
            let handler: Arc<dyn TriggerHandler> = Arc::new(
                WebhookTriggerAdapter::new(ConfigurableWebhookAction {
                    outcome_cell: Arc::clone(&outcome),
                })
                .expect("valid test catalog definition"),
            );
            let ctx = base_ctx(wf_id, trig.clone());
            handler.start(&ctx).await.unwrap();

            // Both must carry the published workflow so if the rate-limit check
            // somehow fails the emit would succeed — proving it's the 429 stopping it.
            let def = minimal_workflow_def(wf_id, trig.as_str().to_string());
            runtime.activate(&scope, def).await;

            let handle = activate_and_persist(
                &transport,
                activation_store.as_ref(),
                PersistParams {
                    handler,
                    action_config: WebhookConfig::default()
                        .with_signature_policy(swh_required_policy()),
                    ctx_template: ctx,
                    trigger_id: trig.as_str().to_string(),
                    spec_trigger_id: format!("trg_test_{}", trig.as_str()),
                    scope: scope.clone(),
                    workflow_id: Some(wf_id.to_string()),
                    mode: WebhookMode::Prod,
                },
            )
            .await
            .unwrap();
            handles.push((handle, wf_id));
        }

        let body = b"{}";
        let msg_id_1 = "g-msg-001";
        let msg_id_2 = "g-msg-002";

        // Request 1 — token A, fully signed → must pass (per-token OK, per-tenant OK).
        let headers_1 = swh_headers(msg_id_1, body);
        let key_1 = WebhookKey::programmatic(handles[0].0.trigger_uuid, handles[0].0.nonce.clone());
        let r1 = dispatch_inner(
            transport.clone(),
            key_1,
            Method::POST,
            Uri::from_static("http://localhost/webhooks/test"),
            headers_1,
            Bytes::from(body as &[u8]),
        )
        .await;
        assert_eq!(
            r1.status(),
            StatusCode::OK,
            "first Prod+SWH request must succeed"
        );

        // Request 2 — token B (fresh per-token window), signed, same tenant.
        // Per-tenant aggregate is now exhausted → must return 429.
        // The request must be rejected before signature enforcement or emission.
        let headers_2 = swh_headers(msg_id_2, body);
        let key_2 = WebhookKey::programmatic(handles[1].0.trigger_uuid, handles[1].0.nonce.clone());
        let r2 = dispatch_inner(
            transport.clone(),
            key_2,
            Method::POST,
            Uri::from_static("http://localhost/webhooks/test"),
            headers_2,
            Bytes::from(body as &[u8]),
        )
        .await;
        assert_eq!(
            r2.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "second Prod+SWH request same tenant must be per-tenant 429 \
             before signature enforcement and durable emit)"
        );

        // Zero executions spawned for the rejected request.
        // One execution from the first request (Prod emit succeeded).
        assert_eq!(
            exec_store.count(&scope, None).await.unwrap(),
            1,
            "only the first Prod request should have spawned an execution; \
             the 429 must not reach the durable emit path"
        );
    }

    // ── Standard Webhooks tamper detection ───────────────────────────────────

    /// A Prod request with a valid SWH signature over the ORIGINAL body, but
    /// the body has been tampered with in transit → 401 (SignatureInvalid).
    ///
    /// Removing `SignatureScheme::StandardWebhooks` from
    /// `verify_with` reverts to the default `Sha256Hex` scheme, which checks a
    /// different header and different content — the tampered body would slip
    /// through whatever the routing-entry config says.
    #[tokio::test]
    async fn prod_mode_swh_tampered_body_returns_401() {
        let fix = TestFixture::prod(WebhookMode::Prod).await;

        let msg_id = "msg-tamper-001";
        let ts = now_secs();
        let original_body = b"{\"original\":true}";
        let tampered_body = b"{\"tampered\":true}";

        // Sign over the ORIGINAL body.
        let sig = sign_swh(msg_id, ts, original_body);

        // But send the TAMPERED body.
        let mut headers = HeaderMap::new();
        headers.insert(WEBHOOK_ID_HEADER, HeaderValue::from_str(msg_id).unwrap());
        headers.insert(
            HeaderName::from_static("webhook-timestamp"),
            HeaderValue::from_str(&ts.to_string()).unwrap(),
        );
        headers.insert(
            HeaderName::from_static("webhook-signature"),
            HeaderValue::from_str(&sig).unwrap(),
        );

        let resp = dispatch_inner(
            fix.transport.clone(),
            fix.key(),
            Method::POST,
            Uri::from_static("http://localhost/webhooks/test"),
            headers,
            Bytes::from(tampered_body as &[u8]),
        )
        .await;

        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "tampered body with valid sig (over original) must return 401"
        );
        assert_eq!(
            fix.execution_count().await,
            0,
            "tampered-body request must not spawn any execution"
        );
    }

    // ── Production signature policy consistency ──────────────────────────────

    /// A Prod-mode activation whose **in-memory routing entry** carries
    /// `SignaturePolicy::OptionalAcceptUnsigned` → 500 (misconfiguration
    /// detected at dispatch time, no execution spawned).
    ///
    /// A durable Prod row
    /// will spawn) AND `OptionalAcceptUnsigned` is a composition-root
    /// misconfiguration.  The invariant is enforced at the transport layer,
    /// not only at activation time, so a stale in-memory routing entry cannot
    /// silently downgrade a durable Prod path to unsigned acceptance.
    ///
    /// Removing the policy consistency check (`if durable.is_some() &&
    /// matches!(…OptionalAcceptUnsigned)` block in `dispatch_inner`) causes the
    /// unsigned request to pass sig enforcement (`OptionalAcceptUnsigned →
    /// Pass`) and the action to emit, materializing an execution.  The
    /// `execution_count == 1` assertion below FAILS → test is RED.
    #[tokio::test]
    async fn prod_unsigned_policy_returns_500_and_spawns_nothing() {
        // Build a Prod-mode fixture that DELIBERATELY uses OptionalAcceptUnsigned.
        // This is the misconfiguration scenario: the activation row is Prod but
        // the in-memory routing entry's action_config has no sig policy.
        let scope = tenant_scope();
        let workflow_id = WorkflowId::new();
        let trigger_id_key = node_key!("webhook_trigger");
        let trigger_id = trigger_id_key.as_str().to_string();

        let exec_store = InMemoryExecutionStore::new();
        let exec_store = Arc::new(exec_store);
        let version_store = Arc::new(InMemoryWorkflowVersionStore::new());
        let runtime = RuntimeFixture::new(&exec_store, &version_store);
        let activation_store: Arc<dyn WebhookActivationStore> =
            Arc::new(InMemoryWebhookActivationStore::new());

        let outcome_cell = Arc::new(Mutex::new(TriggerEventOutcome::emit(json!({"ok": true}))));
        let handler: Arc<dyn TriggerHandler> = Arc::new(
            WebhookTriggerAdapter::new(ConfigurableWebhookAction {
                outcome_cell: Arc::clone(&outcome_cell),
            })
            .expect("valid test catalog definition"),
        );
        let ctx_template = base_ctx(workflow_id, trigger_id_key);
        handler.start(&ctx_template).await.expect("handler start");

        let transport = WebhookTransport::new(WebhookTransportConfig::default())
            .with_activation_store(Arc::clone(&activation_store))
            .with_durable_dispatch(runtime.starts.clone());

        // Deliberately misconfiguged: Prod mode row + OptionalAcceptUnsigned.
        let handle = activate_and_persist(
            &transport,
            activation_store.as_ref(),
            PersistParams {
                handler,
                action_config: WebhookConfig::default()
                    .with_signature_policy(SignaturePolicy::OptionalAcceptUnsigned),
                ctx_template,
                trigger_id: trigger_id.clone(),
                spec_trigger_id: "trg_test_webhook_trigger".to_string(),
                scope: scope.clone(),
                workflow_id: Some(workflow_id.to_string()),
                mode: WebhookMode::Prod, // Prod row — durable path will be taken
            },
        )
        .await
        .expect("activate_and_persist must succeed");

        // Publish a valid workflow definition (so if the guard were missing,
        // the emit would succeed — proving the guard is what stops it).
        let def = minimal_workflow_def(workflow_id, trigger_id);
        runtime.activate(&scope, def).await;

        let key = WebhookKey::programmatic(handle.trigger_uuid, handle.nonce.clone());

        // Request carries `webhook-id` (satisfying the dedup-key requirement in
        // `dispatch_durable`) but NO signature (correct for `OptionalAcceptUnsigned`
        // — no sig is required when that policy is active).
        //
        // Why `webhook-id` is mandatory here: without it the request hits the
        // 400-missing-dedup-key guard in `dispatch_durable`, and `execution_count`
        // stays 0 for the wrong reason, masking the spawn-prevention property.
        // With `webhook-id` present, `OptionalAcceptUnsigned`
        // → sig-enforcement Pass → handler emits → durable execution spawns →
        // `execution_count == 1` → the count assertion below goes RED.
        let mut headers_with_id = HeaderMap::new();
        headers_with_id.insert(
            WEBHOOK_ID_HEADER,
            HeaderValue::from_static("unsigned-prod-probe-001"),
        );
        let resp = dispatch_inner(
            transport,
            key,
            Method::POST,
            Uri::from_static("http://localhost/webhooks/test"),
            headers_with_id,
            Bytes::from(b"{}" as &[u8]),
        )
        .await;

        // Count checked FIRST so both assertions are visible in revert runs.
        // Without the consistency check, the unsigned handler emits and
        // materializes one execution.
        assert_eq!(
            exec_store.count(&scope, None).await.unwrap(),
            0,
            "an unsigned Prod activation must not spawn an execution"
        );
        assert_eq!(
            resp.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "Prod row with OptionalAcceptUnsigned must return 500"
        );
    }
}
