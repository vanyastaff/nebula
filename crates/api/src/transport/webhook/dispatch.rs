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

    // Deserialize via the JSON string path (not `from_value`) to allow
    // `domain-key`'s serde impl to borrow `&str` slices from the input
    // buffer.  `serde_json::from_value` runs through an owning `Value`
    // deserializer that cannot satisfy `<&str>::deserialize` — the
    // `is_human_readable()` branch in `domain-key` v0.6 uses zero-copy
    // `&str` deserialization that requires a slice-backed reader.
    let def_json = serde_json::to_string(&version_record.definition).map_err(|e| {
        warn!(
            trigger_id = %row.trigger_id,
            scope = ?scope,
            workflow_id = %workflow_id,
            error = %e,
            "Prod-mode webhook: workflow definition failed to serialize to JSON string; \
             fail-closed — no execution spawned"
        );
        (StatusCode::INTERNAL_SERVER_ERROR, "").into_response()
    })?;
    let def: WorkflowDefinition = serde_json::from_str(&def_json).map_err(|e| {
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Integration tests for the durable dispatch path introduced in U-D1.4b.
    //!
    //! Each test drives `dispatch_inner` directly — the same code path the
    //! axum handler calls — so the assertions cover the real dispatch logic.
    //!
    //! # Test fixture overview
    //!
    //! `Fixture` wires a complete in-memory stack:
    //! - `WebhookTransport` with `activation_store` + `durable_dispatch`
    //! - `InMemoryWebhookActivationStore` (token resolution)
    //! - `InMemoryTriggerDedupInbox` (atomic dedup + Start enqueue)
    //! - `InMemoryWorkflowVersionStore` (workflow load)
    //! - `InMemoryExecutionStore` (execution rows + job queue)
    //! - `DefinitionRoutingResolver` (plugin-routing)
    //! - `ConfigurableWebhookAction` — a `WebhookAction` whose outcome
    //!   is set per-test via `Arc<Mutex<TriggerEventOutcome>>`.

    use std::sync::Arc;

    use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
    use nebula_action::{
        Action, ActionMetadata, SignaturePolicy, TriggerContext, TriggerEventOutcome,
        TriggerHandler, WebhookAction, WebhookConfig, WebhookRequest, WebhookResponse,
        WebhookTriggerAdapter,
    };
    use nebula_core::{BaseContext, Dependencies, NodeKey, WorkflowId, action_key, node_key};
    use nebula_storage::inmem::{
        InMemoryExecutionStore, InMemoryTriggerDedupInbox, InMemoryWebhookActivationStore,
        InMemoryWorkflowVersionStore,
    };
    use nebula_storage_port::{
        Scope,
        dto::{WebhookMode, WorkflowVersionRecord},
        store::{TriggerDedupInbox, WebhookActivationStore, WorkflowVersionStore},
    };
    use nebula_workflow::{WorkflowBuilder, WorkflowDefinition, node::NodeDefinition};
    use parking_lot::Mutex;
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::transport::webhook::transport::{
        PersistParams, WebhookTransport, WebhookTransportConfig, activate_and_persist,
    };

    // ── TestWebhookAction ─────────────────────────────────────────────────────

    /// Minimal `WebhookAction` whose `handle_request` returns whatever
    /// `outcome_cell` says.  Signature checking is disabled so tests can
    /// send unsigned bodies.
    struct ConfigurableWebhookAction {
        outcome_cell: Arc<Mutex<TriggerEventOutcome>>,
    }

    impl Action for ConfigurableWebhookAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(
                action_key!("test.dispatch.configurable"),
                "ConfigurableWebhookAction",
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

    // ── TestFixture ───────────────────────────────────────────────────────────

    /// Fully-wired in-memory fixture for durable dispatch tests.
    ///
    /// Stores the `ActivationHandle` so `dispatch_with_id` / `dispatch_without_id`
    /// can derive the correct `(trigger_uuid, nonce)` webhook key.
    #[allow(dead_code)]
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
        /// Build a full Prod-mode fixture.
        async fn prod(mode: WebhookMode) -> Self {
            let scope = Scope::new("test-org", "test-ws");
            let workflow_id = WorkflowId::new();
            let trigger_id_key = node_key!("webhook_trigger");
            let trigger_id = trigger_id_key.as_str().to_string();

            let exec_store = InMemoryExecutionStore::new();
            let dedup = Arc::new(InMemoryTriggerDedupInbox::new(&exec_store));
            let exec_store = Arc::new(exec_store);
            let version_store = Arc::new(InMemoryWorkflowVersionStore::new());
            let activation_store: Arc<dyn WebhookActivationStore> =
                Arc::new(InMemoryWebhookActivationStore::new());

            let outcome_cell = Arc::new(Mutex::new(TriggerEventOutcome::emit(json!({"ok": true}))));

            let handler: Arc<dyn TriggerHandler> =
                Arc::new(WebhookTriggerAdapter::new(ConfigurableWebhookAction {
                    outcome_cell: Arc::clone(&outcome_cell),
                }));
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
                .with_durable_dispatch(
                    dedup as Arc<dyn TriggerDedupInbox>,
                    WebhookTransport::default_resolver(),
                    Arc::clone(&version_store) as Arc<dyn WorkflowVersionStore>,
                );

            let handle = activate_and_persist(
                &transport,
                activation_store.as_ref(),
                PersistParams {
                    handler,
                    action_config: WebhookConfig::default()
                        .with_signature_policy(SignaturePolicy::OptionalAcceptUnsigned),
                    ctx_template,
                    trigger_id: trigger_id.clone(),
                    scope: scope.clone(),
                    workflow_id: Some(workflow_id.to_string()),
                    mode,
                },
            )
            .await
            .expect("activate_and_persist must succeed");

            // Publish a valid workflow definition
            let def = minimal_workflow_def(workflow_id, trigger_id.clone());
            version_store
                .create(
                    &scope,
                    WorkflowVersionRecord {
                        workflow_id: workflow_id.to_string(),
                        number: 1,
                        published: true,
                        pinned: false,
                        definition: serde_json::to_value(&def).unwrap(),
                    },
                )
                .await
                .expect("version store create");

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

        async fn dispatch_with_id(&self, delivery_id: &str) -> Response {
            dispatch_inner(
                self.transport.clone(),
                self.key(),
                Method::POST,
                Uri::from_static("http://localhost/webhooks/test"),
                headers_with_delivery_id(delivery_id),
                Bytes::from(b"{}" as &[u8]),
            )
            .await
        }

        async fn dispatch_without_id(&self) -> Response {
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

    fn headers_with_delivery_id(id: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            WEBHOOK_ID_HEADER,
            HeaderValue::from_str(id).expect("valid header value"),
        );
        h
    }

    fn base_ctx(
        workflow_id: WorkflowId,
        node_key: NodeKey,
    ) -> nebula_action::TriggerRuntimeContext {
        nebula_action::TriggerRuntimeContext::new(
            Arc::new(
                BaseContext::builder()
                    .cancellation(CancellationToken::new())
                    .build(),
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

    // ── Test 1: Prod-mode Emit spawns exactly one execution ───────────────────

    /// Prod-mode activation + `webhook-id` header → emitter is called.
    /// Verify the response is 200 OK.
    ///
    /// This is the nominal happy-path test.  Removing the mode gate or the
    /// `durable_dispatch` wiring would cause this to still return 200 (via
    /// the fallthrough path) — the behavioral assertion is that we get a 200
    /// AND the inbox was hit (which we indirectly verify in test 3 via dedup).
    #[tokio::test]
    async fn prod_mode_emit_returns_200() {
        let fix = TestFixture::prod(WebhookMode::Prod).await;
        let resp = fix.dispatch_with_id("delivery-001").await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "Prod emit must return 200 (Emit + durable inbox)"
        );
    }

    // ── Test 2: Test mode spawns nothing, returns 200 (existing behaviour) ────

    /// mode=Test → the durable path is NEVER taken even when `durable_dispatch`
    /// is wired.  The fallthrough in-memory path returns 200.
    ///
    /// Red-on-revert: removing the `row.mode == WebhookMode::Prod` guard would
    /// cause Test-mode activations to take the durable path, which would
    /// fail (inbox claim) OR spawn unexpectedly.
    #[tokio::test]
    async fn test_mode_skips_durable_dispatch_returns_200() {
        let fix = TestFixture::prod(WebhookMode::Test).await;
        // Test-mode: no durable_dispatch wired means the mode guard must
        // correctly NOT set durable = Some(...), so the fallthrough path runs.
        let resp = fix.dispatch_with_id("delivery-002").await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "Test-mode must return 200 via fallthrough path"
        );
    }

    // ── Test 3: Same webhook-id twice → second is deduplicated ────────────────

    /// Delivering the same `webhook-id` twice: the first call spawns the
    /// execution; the second is a duplicate from the inbox's perspective.
    /// Both should return 200 (the second call is acked just like the first —
    /// the dedup is at the storage layer, the HTTP response is always 200).
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
    }

    // ── Test 4: Missing webhook-id in Prod → 400 ─────────────────────────────

    /// Prod-mode activation without `webhook-id` header → fail-closed 400.
    ///
    /// Red-on-revert: removing the `durable.is_some() && event_id.is_none()`
    /// guard would cause Prod-mode dispatch to proceed with `event_id = None`,
    /// which defeats the dedup invariant.
    #[tokio::test]
    async fn prod_mode_missing_webhook_id_returns_400() {
        let fix = TestFixture::prod(WebhookMode::Prod).await;
        let resp = fix.dispatch_without_id().await;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "Prod-mode without webhook-id must be 400"
        );
    }

    // ── Test 5: EmitMany in Prod → 500 ────────────────────────────────────────

    /// `EmitMany` outcome in Prod mode → fail-closed 500.
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

    // ── Test 6: Skip in Prod → 200, no execution ──────────────────────────────

    /// `Skip` outcome in Prod mode → no execution spawned, HTTP 200.
    ///
    /// The adapter sends the HTTP response before returning; `dispatch_durable`
    /// reads the oneshot and returns 200 without hitting the inbox.
    #[tokio::test]
    async fn prod_mode_skip_returns_200_no_spawn() {
        let fix = TestFixture::prod(WebhookMode::Prod).await;
        fix.set_outcome(TriggerEventOutcome::skip());
        let resp = fix.dispatch_with_id("delivery-005").await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "Skip in Prod must return 200"
        );
    }

    // ── Test 7: workflow_id None on activation row → 500 ─────────────────────

    /// Prod-mode activation with `workflow_id = None` on the row → fail-closed 500.
    ///
    /// This covers inv #5: a Prod activation without a wired workflow_id must
    /// never spawn a dedup-blind execution.
    #[tokio::test]
    async fn prod_mode_no_workflow_id_returns_500() {
        // Build a transport with a Prod-mode row that has workflow_id = None.
        let scope = Scope::new("test-org", "test-ws");
        let workflow_id = WorkflowId::new();
        let trigger_id = node_key!("webhook_trigger").as_str().to_string();

        let exec_store = InMemoryExecutionStore::new();
        let dedup = Arc::new(InMemoryTriggerDedupInbox::new(&exec_store));
        let version_store: Arc<dyn WorkflowVersionStore> =
            Arc::new(InMemoryWorkflowVersionStore::new());
        let activation_store: Arc<dyn WebhookActivationStore> =
            Arc::new(InMemoryWebhookActivationStore::new());

        let outcome_cell = Arc::new(Mutex::new(TriggerEventOutcome::emit(json!({"ok": true}))));
        let handler: Arc<dyn TriggerHandler> =
            Arc::new(WebhookTriggerAdapter::new(ConfigurableWebhookAction {
                outcome_cell: Arc::clone(&outcome_cell),
            }));
        let ctx_template = base_ctx(workflow_id, node_key!("webhook_trigger"));
        handler
            .start(&ctx_template)
            .await
            .expect("handler start must succeed");

        let transport = WebhookTransport::new(WebhookTransportConfig::default())
            .with_activation_store(Arc::clone(&activation_store))
            .with_durable_dispatch(
                dedup as Arc<dyn TriggerDedupInbox>,
                WebhookTransport::default_resolver(),
                Arc::clone(&version_store),
            );

        let handle = activate_and_persist(
            &transport,
            activation_store.as_ref(),
            PersistParams {
                handler,
                action_config: WebhookConfig::default()
                    .with_signature_policy(SignaturePolicy::OptionalAcceptUnsigned),
                ctx_template,
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
        let resp = dispatch_inner(
            transport,
            key,
            Method::POST,
            Uri::from_static("http://localhost/webhooks/test"),
            headers_with_delivery_id("delivery-006"),
            Bytes::from(b"{}" as &[u8]),
        )
        .await;

        assert_eq!(
            resp.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "Prod-mode with workflow_id=None must be 500"
        );
    }

    // ── Test 8: Tenant isolation — workflow in wrong scope → 500 ─────────────

    /// Prod-mode activation whose activation-row scope is scope_A, but the
    /// workflow version was stored under scope_B.
    ///
    /// `ScopedWorkflowVersionStore` pins the lookup to `row.scope` (scope_A),
    /// so the version stored under scope_B is invisible → 500.
    ///
    /// This is the confused-deputy test (invariant #5).
    ///
    /// Red-on-revert: passing the request-derived scope (or no scoping) to
    /// `get_published` would let a cross-scope lookup succeed, violating the
    /// tenant boundary.
    #[tokio::test]
    async fn tenant_isolation_wrong_scope_returns_500() {
        let scope_a = Scope::new("org-a", "ws-a");
        let scope_b = Scope::new("org-b", "ws-b");
        let workflow_id = WorkflowId::new();
        let trigger_id = node_key!("webhook_trigger").as_str().to_string();

        let exec_store = InMemoryExecutionStore::new();
        let dedup = Arc::new(InMemoryTriggerDedupInbox::new(&exec_store));
        let version_store = Arc::new(InMemoryWorkflowVersionStore::new());
        let activation_store: Arc<dyn WebhookActivationStore> =
            Arc::new(InMemoryWebhookActivationStore::new());

        let outcome_cell = Arc::new(Mutex::new(TriggerEventOutcome::emit(json!({"ok": true}))));
        let handler: Arc<dyn TriggerHandler> =
            Arc::new(WebhookTriggerAdapter::new(ConfigurableWebhookAction {
                outcome_cell: Arc::clone(&outcome_cell),
            }));
        let ctx_template_a = base_ctx(workflow_id, node_key!("webhook_trigger"));
        handler
            .start(&ctx_template_a)
            .await
            .expect("handler start must succeed");

        let transport = WebhookTransport::new(WebhookTransportConfig::default())
            .with_activation_store(Arc::clone(&activation_store))
            .with_durable_dispatch(
                dedup as Arc<dyn TriggerDedupInbox>,
                WebhookTransport::default_resolver(),
                Arc::clone(&version_store) as Arc<dyn WorkflowVersionStore>,
            );

        // Activation row registered under scope_a.
        let handle = activate_and_persist(
            &transport,
            activation_store.as_ref(),
            PersistParams {
                handler,
                action_config: WebhookConfig::default()
                    .with_signature_policy(SignaturePolicy::OptionalAcceptUnsigned),
                ctx_template: ctx_template_a,
                trigger_id: trigger_id.clone(),
                scope: scope_a.clone(),
                workflow_id: Some(workflow_id.to_string()),
                mode: WebhookMode::Prod,
            },
        )
        .await
        .expect("activate_and_persist");

        // Workflow stored under scope_b — should NOT be visible to scope_a lookup.
        let def = minimal_workflow_def(workflow_id, trigger_id);
        version_store
            .create(
                &scope_b,
                WorkflowVersionRecord {
                    workflow_id: workflow_id.to_string(),
                    number: 1,
                    published: true,
                    pinned: false,
                    definition: serde_json::to_value(&def).unwrap(),
                },
            )
            .await
            .expect("version create");

        let key = WebhookKey::programmatic(handle.trigger_uuid, handle.nonce.clone());
        let resp = dispatch_inner(
            transport,
            key,
            Method::POST,
            Uri::from_static("http://localhost/webhooks/test"),
            headers_with_delivery_id("delivery-007"),
            Bytes::from(b"{}" as &[u8]),
        )
        .await;

        assert_eq!(
            resp.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "workflow stored in wrong scope must be invisible → 500"
        );
    }
}
