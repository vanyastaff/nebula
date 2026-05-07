//! Webhook trigger endpoint handlers.
//!
//! Special: no standard auth middleware — per-trigger authentication
//! is enforced inside the [`WebhookDispatcher`] via the registered
//! [`crate::webhook::WebhookAuthConfig`].
//!
//! The route is `POST /api/v1/hooks/{org}/{ws}/{trigger_slug}`. On
//! success the handler returns `202 Accepted`: the engine processes
//! the event asynchronously, and the HTTP response carries no
//! workflow output. Failure paths return RFC 9457 `problem+json`.

use std::sync::Arc;

use axum::{
    Extension,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use tracing::{debug, info_span};

use crate::{
    errors::{ApiError, ProblemDetails},
    state::AppState,
    webhook::{
        TriggerCoordinates, WebhookAuthError, WebhookDispatchError, WebhookDispatcher,
        WebhookEnqueueError,
    },
};

/// `POST /api/v1/hooks/{org}/{ws}/{trigger_slug}`.
///
/// Validates per-trigger authentication, enqueues the trigger event
/// into the engine sink, and returns `202 Accepted`. Error mapping:
///
/// | Outcome                                      | Status |
/// |----------------------------------------------|--------|
/// | Sink accepted the event                      | 202    |
/// | No active webhook registered for the slug    | 404    |
/// | Per-trigger auth failed (bad/missing sig)    | 401    |
/// | Operator-side misconfig (empty secret etc.)  | 500    |
/// | Sink unavailable (closed)                    | 500    |
/// | Sink saturated (back-pressure)               | 503    |
#[utoipa::path(
    post,
    path = "/hooks/{org}/{ws}/{trigger_slug}",
    tag = "webhooks",
    security(()),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("trigger_slug" = String, Path, description = "Per-trigger slug registered with the workspace."),
    ),
    request_body(content_type = "*/*", description = "Raw provider payload — opaque bytes."),
    responses(
        (status = 202, description = "Event accepted by the engine sink."),
        (status = 401, description = "Per-trigger authentication failed (missing/invalid signature, missing/invalid bearer token).", body = ProblemDetails),
        (status = 404, description = "No active webhook is registered for this slug.", body = ProblemDetails),
        (status = 500, description = "Operator-side misconfiguration or the sink is unavailable.", body = ProblemDetails),
        (status = 503, description = "Sink saturated (back-pressure).", body = ProblemDetails),
    ),
)]
pub async fn handle_webhook_post(
    State(_state): State<AppState>,
    Extension(dispatcher): Extension<Arc<WebhookDispatcher>>,
    Path((org, ws, trigger_slug)): Path<(String, String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let coordinates = TriggerCoordinates::new(org, ws, trigger_slug);

    let span = info_span!(
        "api.webhook.handle_post",
        org = %coordinates.org,
        workspace = %coordinates.workspace,
        trigger = %coordinates.trigger,
        body_bytes = body.len(),
    );
    let _enter = span.enter();

    match dispatcher
        .dispatch(coordinates.clone(), method, uri, headers, body)
        .await
    {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(err) => {
            debug!(error = %err, "webhook dispatch failed");
            dispatch_error_to_response(err)
        },
    }
}

/// `GET /api/v1/hooks/{org}/{ws}/{trigger_slug}`.
///
/// GET is reserved for webhook providers that issue verification
/// challenges (e.g. Slack URL verification). The slug-routed surface
/// does not yet model such challenges — providers that need them rely
/// on the typed-action transport in [`crate::services::webhook`]. We
/// reply with `405 Method Not Allowed` so the contract is explicit
/// rather than silently 5xx'ing.
#[utoipa::path(
    get,
    path = "/hooks/{org}/{ws}/{trigger_slug}",
    tag = "webhooks",
    security(()),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("trigger_slug" = String, Path, description = "Per-trigger slug registered with the workspace."),
    ),
    responses(
        (status = 405, description = "GET is not yet supported on the slug-routed webhook surface; the response carries the `Allow: POST` header."),
    ),
)]
pub async fn handle_webhook_get(
    State(_state): State<AppState>,
    Path((_org, _ws, _trigger_slug)): Path<(String, String, String)>,
) -> Response {
    let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
    if let Ok(value) = "POST".parse() {
        response
            .headers_mut()
            .insert(axum::http::header::ALLOW, value);
    }
    response
}

fn dispatch_error_to_response(err: WebhookDispatchError) -> Response {
    match err {
        WebhookDispatchError::NotFound { trigger } => ApiError::NotFound(format!(
            "no webhook trigger registered for {}/{}/{}",
            trigger.org, trigger.workspace, trigger.trigger
        ))
        .into_response(),
        WebhookDispatchError::Auth(auth_err) => auth_error_to_response(auth_err),
        WebhookDispatchError::Enqueue(enqueue_err) => enqueue_error_to_response(enqueue_err),
    }
}

fn auth_error_to_response(err: WebhookAuthError) -> Response {
    match err {
        WebhookAuthError::SignatureMissing
        | WebhookAuthError::SignatureInvalid
        | WebhookAuthError::TokenMissing
        | WebhookAuthError::TokenInvalid => ApiError::Unauthorized(err.to_string()).into_response(),
        // SecretNotConfigured is operator misconfiguration (5xx)
        // rather than caller error — fail-closed in the same way as
        // the typed-action transport's `missing_secret_response`.
        WebhookAuthError::SecretNotConfigured => {
            ApiError::Internal(err.to_string()).into_response()
        },
    }
}

fn enqueue_error_to_response(err: WebhookEnqueueError) -> Response {
    match err {
        WebhookEnqueueError::SinkUnavailable { .. } => {
            ApiError::Internal(err.to_string()).into_response()
        },
        WebhookEnqueueError::SinkBackpressure { .. } => {
            ApiError::ServiceUnavailable(err.to_string()).into_response()
        },
    }
}
