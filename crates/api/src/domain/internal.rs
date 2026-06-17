//! `/internal/v1/...` routes — ops tooling consumed out-of-band
//! (webhook activation — E3).
//!
//! Internal routes are **not** advertised in OpenAPI; operators
//! discover them via runbooks. The token check lives in
//! [`crate::middleware::internal_auth_middleware`]. Mounted on a
//! plain `axum::Router` (not `OpenApiRouter`) so they never leak
//! into `/api/v1/openapi.json`.

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use serde::Serialize;

use crate::state::AppState;
use crate::transport::webhook::bootstrap_webhook_activations;

/// Body returned by `POST /internal/v1/webhooks/reload`.
#[derive(Debug, Serialize)]
pub struct WebhookReloadReport {
    /// Activations validated from the port store.
    pub loaded: usize,
    /// Rows that surfaced a non-storage failure and were skipped.
    pub skipped: usize,
}

async fn reload_webhooks(State(state): State<AppState>) -> impl IntoResponse {
    let Some(store) = state.webhook_activation_store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "webhook activation store not configured",
        )
            .into_response();
    };
    let Some(secrets) = state.webhook_secret_resolver.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "webhook secret resolver not configured",
        )
            .into_response();
    };
    let Some(ctx_factory) = state.webhook_ctx_factory_b.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "webhook ctx factory not configured",
        )
            .into_response();
    };
    let Some(spec_lookup) = state.webhook_spec_lookup.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "webhook spec lookup not configured",
        )
            .into_response();
    };
    let Some(registry) = state.action_registry.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "action registry not attached",
        )
            .into_response();
    };

    match bootstrap_webhook_activations(
        store.as_ref(),
        registry,
        secrets.as_ref(),
        ctx_factory.as_ref(),
        spec_lookup.as_ref(),
        None,
    )
    .await
    {
        Ok(report) => {
            tracing::info!(
                target: "nebula::api::internal::webhook_reload",
                loaded = report.loaded,
                skipped = report.skipped,
                "webhook port store validated",
            );
            (
                StatusCode::OK,
                Json(WebhookReloadReport {
                    loaded: report.loaded,
                    skipped: report.skipped,
                }),
            )
                .into_response()
        },
        Err(err) => {
            tracing::error!(
                target: "nebula::api::internal::webhook_reload",
                error = %err,
                "webhook reload failed",
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("webhook reload failed: {err}"),
            )
                .into_response()
        },
    }
}

/// Build the `/internal/v1` router.
///
/// Mounted on the main `axum::Router` (not `OpenApiRouter`) by
/// [`crate::app::build_app`] so the spec stays clean.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/internal/v1/webhooks/reload", post(reload_webhooks))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::internal_auth_middleware,
        ))
        .with_state(state)
}
