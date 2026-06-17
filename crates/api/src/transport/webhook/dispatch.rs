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
//! 7. Extract `webhook-id` header → `event_id: Option<IdempotencyKey>` (Commit 3)
//! 8. Dispatch via [`TriggerHandler::handle_event`] with timeout → 504 / 500 / handler response
//! 9. Mode-gate: Prod rows with `durable_dispatch` wired call
//!    [`DurableExecutionEmitter::emit`] before acking the HTTP response (Commit 4)

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderName, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use nebula_action::{
    ExecutionEmitter, IdempotencyKey, TriggerEvent, TriggerEventOutcome, WebhookHttpResponse,
    WebhookRequest,
};
use nebula_core::NodeKey;
use nebula_engine::DurableExecutionEmitter;
use nebula_metrics::{
    NEBULA_WEBHOOK_RATE_LIMIT_REJECTIONS_TOTAL, webhook_key_kind, webhook_signature_failure_reason,
};
use nebula_storage_port::dto::WebhookMode;
use nebula_storage_port::store::WorkflowVersionStore;
use nebula_tenancy::ScopedWorkflowVersionStore;
use nebula_workflow::{ValidatedWorkflow, WorkflowDefinition};
use tokio::sync::oneshot;
use tracing::{debug, warn};

use super::{
    key::WebhookKey,
    signature::{
        SignatureVerdict, enforce_signature, missing_secret_response, record_signature_failure,
        signature_rejected_response,
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
/// 8. dispatch via [`TriggerHandler::handle_event`] with a response
///    timeout → 504 / 500 / handler response
/// 9. mode-gate: Prod rows call [`DurableExecutionEmitter::emit`] BEFORE
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

    // 4.5. Token resolution via the B-world port store (ADR-0096 commit 2b).
    //
    // Placed AFTER route-lookup (step 3) and rate-limit (step 4) so an
    // unauthenticated attacker hitting an unregistered path or a rate-limited
    // key never triggers a DB query.  Only authenticated-enough requests
    // (registered key, under rate limit) reach the store.
    //
    // nonce / hash are deliberately excluded from all log fields — the nonce
    // is the bearer token and must never appear in log aggregators or traces.
    //
    // U-D1.4b: when the row is Prod and `durable_dispatch` is wired, `durable`
    // is set to `Some(DurableTarget { ... })` below.  Test mode and missing rows
    // leave it `None` (no durable spawn, preserve today's behaviour).
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
                // Mode gate: only Prod rows with a wired inbox spawn durable
                // executions.  Fail-closed when inbox absent in Prod mode.
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

    // 5. Construct WebhookRequest. Limits are already enforced by
    // `try_new` — the only failures here are body-size exceed
    // (handled above for a better error message) and header count
    // exceed (rare; returns 400).
    let path = uri.path().to_string();
    let query = uri.query().map(String::from);

    // Step 7 (Commit 3): extract `webhook-id` header before consuming `headers`
    // into `WebhookRequest`.  The header is NOT a secret (Standard Webhooks spec
    // §4 — "webhook-id must be a unique identifier per message delivery") and
    // may be logged as a tracing field.
    //
    // Fail-closed rule (inv #6): Prod-mode activations require `webhook-id`.
    // Missing header in Prod → 400 (sender must supply a delivery id).
    // Test mode / no-row → `event_id = None` is fine.
    let event_id: Option<IdempotencyKey> = headers
        .get(&WEBHOOK_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| IdempotencyKey::new(s.trim()));

    if durable.is_some() && event_id.is_none() {
        // Prod row resolved but `webhook-id` absent — fail closed.
        // The sender must supply a delivery id for dedup correctness.
        // Return 400 (bad request) so the sender is informed the header
        // is required; a retry with the header will succeed.
        warn!(
            mode = "Prod",
            "Prod-mode webhook: missing `webhook-id` header; \
             fail-closed (dedup requires a delivery id)"
        );
        return (
            StatusCode::BAD_REQUEST,
            "missing webhook-id header for Prod-mode dispatch",
        )
            .into_response();
    }

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

// ── Durable dispatch ─────────────────────────────────────────────────────────

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
/// The `event_id` is guaranteed `Some` at call sites (enforced by the
/// Prod+missing-header gate in `dispatch_inner`).
async fn dispatch_durable(
    target: DurableTarget,
    outcome: TriggerEventOutcome,
    event_id: Option<IdempotencyKey>,
    rx: oneshot::Receiver<WebhookHttpResponse>,
) -> Response {
    let DurableTarget { row, components } = target;

    match outcome {
        TriggerEventOutcome::Emit(payload) => {
            let emit_result = do_emit_prod(&row, &components, payload, event_id.as_ref()).await;
            match emit_result {
                Ok(()) => {
                    // Emit succeeded — ack the HTTP response the adapter sent.
                    if let Ok(http) = rx.await {
                        http_response_to_axum(http)
                    } else {
                        warn!("durable emit ok but oneshot sender was dropped");
                        (StatusCode::INTERNAL_SERVER_ERROR, "").into_response()
                    }
                },
                Err(err_response) => {
                    // Emit failed — return the 5xx so the sender retries.
                    // The adapter's oneshot response is discarded; the HTTP
                    // ack is replaced with our 5xx.
                    //
                    // Retry delivers the same `webhook-id` → same `event_id`
                    // → `claim_and_materialize_start` deduplicates.
                    err_response
                },
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
            (StatusCode::INTERNAL_SERVER_ERROR, "").into_response()
        },
        TriggerEventOutcome::Skip => {
            // Skip — no emit.  Return the adapter's HTTP response.
            if let Ok(http) = rx.await {
                http_response_to_axum(http)
            } else {
                warn!("webhook handler Skip but oneshot sender was dropped");
                (StatusCode::INTERNAL_SERVER_ERROR, "").into_response()
            }
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
            (StatusCode::INTERNAL_SERVER_ERROR, "").into_response()
        },
    }
}

/// Core Prod-mode emit path.
///
/// 1. Validates that `workflow_id` is present and parseable (fail-closed on
///    `None` or malformed ULID — inv #5: single-row resolution must fully
///    resolve the workflow).
/// 2. Parses `trigger_id` as a [`NodeKey`] (fail-closed on parse failure).
/// 3. Loads and validates the `ValidatedWorkflow` under `row.scope` via a
///    freshly-bound `ScopedWorkflowVersionStore` (confused-deputy boundary is
///    the scope carried in the row, never request-derived — inv #5).
/// 4. Constructs [`DurableExecutionEmitter`] and calls `emit`.
///
/// Returns `Ok(())` on successful dispatch or `Err(Response)` with a 5xx
/// response ready to return to the caller.
///
/// # Performance note
///
/// The per-delivery `ValidatedWorkflow` load+validate is on the request hot
/// path; canonical webhook senders time out as low as ~3 s (Slack).  If this
/// storage round-trip grows costly, cache `Arc<ValidatedWorkflow>` per
/// activation or add a thinner raw-delivery inbox.  v1 ships the direct path.
async fn do_emit_prod(
    row: &nebula_storage_port::dto::WebhookActivationRecord,
    components: &DurableDispatchComponents,
    payload: serde_json::Value,
    event_id: Option<&IdempotencyKey>,
) -> Result<(), Response> {
    let scope = &row.scope;

    // Step 1a — workflow_id must be present (inv #5).
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
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "").into_response());
    };

    // Step 1b — workflow_id must parse as a valid ULID WorkflowId (correction C).
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
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "").into_response());
        },
    };

    // Step 2 — trigger_id must parse as a NodeKey (fail-closed on invalid key).
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
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "").into_response());
        },
    };

    // Step 3 — load and validate the workflow under the row's scope (inv #5).
    // The confused-deputy boundary is the decorator: `ScopedWorkflowVersionStore`
    // pins the scope to `row.scope`; no cross-scope lookup is possible.
    let scoped_versions =
        ScopedWorkflowVersionStore::new(Arc::clone(&components.version_store), scope.clone());
    let version_record = scoped_versions
        .get_published(scope, &workflow_id.to_string())
        .await
        .map_err(|e| {
            warn!(
                trigger_id = %row.trigger_id,
                scope = ?scope,
                workflow_id = %workflow_id,
                error = %e,
                "Prod-mode webhook: storage error loading workflow version; \
                 fail-closed — no execution spawned"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response()
        })?;

    let version_record = if let Some(v) = version_record {
        v
    } else {
        warn!(
            trigger_id = %row.trigger_id,
            scope = ?scope,
            workflow_id = %workflow_id,
            "Prod-mode webhook: workflow_id not found under row scope; \
             fail-closed — no cross-scope lookup (inv #5)"
        );
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "").into_response());
    };

    let def: WorkflowDefinition =
        serde_json::from_value(version_record.definition).map_err(|e| {
            warn!(
                trigger_id = %row.trigger_id,
                scope = ?scope,
                workflow_id = %workflow_id,
                error = %e,
                "Prod-mode webhook: workflow definition failed to deserialize; \
                 fail-closed — no execution spawned"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "").into_response()
        })?;

    let validated = ValidatedWorkflow::validate(def).map_err(|errors| {
        warn!(
            trigger_id = %row.trigger_id,
            scope = ?scope,
            workflow_id = %workflow_id,
            errors = ?errors,
            "Prod-mode webhook: workflow definition failed validation; \
             fail-closed — no execution spawned"
        );
        (StatusCode::INTERNAL_SERVER_ERROR, "").into_response()
    })?;

    // Step 4 — construct emitter and call emit.
    let emitter = DurableExecutionEmitter::new(
        Arc::clone(&components.dedup),
        Arc::clone(&components.resolver),
        Arc::new(validated),
        trigger_node_key,
        scope.clone(),
    );

    emitter
        .emit(payload, event_id.cloned())
        .await
        .map(|_execution_id| ())
        .map_err(|e| {
            warn!(
                trigger_id = %row.trigger_id,
                scope = ?scope,
                workflow_id = %workflow_id,
                error = %e,
                "Prod-mode webhook: durable emit failed; returning 5xx so sender retries"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "").into_response()
        })
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
    let WebhookKey::Programmatic { uuid, .. } = key;
    let kind = webhook_key_kind::PROGRAMMATIC;
    let tenant_id = uuid.to_string();
    let labels = interner.label_set(&[
        ("webhook_key_kind", kind),
        ("tenant_id", tenant_id.as_str()),
    ]);
    if let Ok(c) = reg.counter_labeled(NEBULA_WEBHOOK_RATE_LIMIT_REJECTIONS_TOTAL, &labels) {
        c.inc();
    }
}
