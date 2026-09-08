//! `ServerTransport` trait and all three transport implementations.
//!
//! Each impl corresponds to one ingress profile:
//! - [`ApiTransport`]  — full REST API (mirrors the old `nebula-server` binary)
//! - [`WebhookIngressTransport`] — webhook-only ingress (mirrors `nebula-webhook`)
//! - [`RealtimeTransport`] — realtime/WS scaffold (mirrors `nebula-realtime`)
//!
//! The [`Transport`] clap enum lets the single `nebula-server` binary select
//! which profile to run via `--transport` / `NEBULA_TRANSPORT`.

use std::{net::SocketAddr, sync::Arc};

use axum::{Json, Router, http::StatusCode, routing::get};
use clap::ValueEnum;
use serde_json::json;
use url::Url;

use nebula_api::{ApiConfig, AppState, build_app};

use crate::compose::{TransportInitError, health_ok};

/// Clap value-enum for `--transport` / `NEBULA_TRANSPORT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Transport {
    /// Full REST API (same behaviour as the former `nebula-server` binary).
    Api,
    /// Webhook ingress only (same behaviour as the former `nebula-webhook` binary).
    Webhook,
    /// Realtime / WebSocket scaffold (same behaviour as the former `nebula-realtime` binary).
    Realtime,
    /// Run the full REST API transport (default; alias for `api`).
    All,
}

/// A server transport profile selected by a binary target.
pub(crate) trait ServerTransport {
    /// Human-readable transport name for logs.
    fn name(&self) -> &'static str;

    /// Optional env var overriding bind address for this transport.
    fn bind_override_var(&self) -> Option<&'static str> {
        None
    }

    /// Customize default state before router build.
    fn prepare_state(
        &self,
        state: AppState,
        _bind_address: SocketAddr,
    ) -> Result<AppState, TransportInitError> {
        Ok(state)
    }

    /// Build the router for this transport.
    fn build_router(
        &self,
        state: AppState,
        api_config: &ApiConfig,
    ) -> Result<Router, TransportInitError>;
}

// ─── ApiTransport ──────────────────────────────────────────────────────────

/// Full REST API transport.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ApiTransport;

impl ServerTransport for ApiTransport {
    fn name(&self) -> &'static str {
        "api"
    }

    fn bind_override_var(&self) -> Option<&'static str> {
        Some("SERVER_BIND_ADDRESS")
    }

    fn build_router(
        &self,
        state: AppState,
        api_config: &ApiConfig,
    ) -> Result<Router, TransportInitError> {
        Ok(build_app(state, api_config))
    }
}

// ─── WebhookIngressTransport ───────────────────────────────────────────────

/// Webhook ingress transport.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WebhookIngressTransport;

impl ServerTransport for WebhookIngressTransport {
    fn name(&self) -> &'static str {
        "webhook"
    }

    fn bind_override_var(&self) -> Option<&'static str> {
        Some("WEBHOOK_BIND_ADDRESS")
    }

    fn prepare_state(
        &self,
        state: AppState,
        bind_address: SocketAddr,
    ) -> Result<AppState, TransportInitError> {
        if state.webhook_transport.is_some() {
            return Ok(state);
        }

        let base_url = if let Ok(raw) = std::env::var("WEBHOOK_BASE_URL") {
            Url::parse(&raw)
                .map_err(|source| TransportInitError::InvalidWebhookBaseUrl { source })?
        } else {
            Url::parse(&format!("http://{bind_address}"))
                .map_err(|source| TransportInitError::InvalidWebhookBaseUrl { source })?
        };

        let webhook_config = nebula_api::transport::webhook::WebhookTransportConfig {
            base_url,
            ..nebula_api::transport::webhook::WebhookTransportConfig::default()
        };

        // Build the transport, then attach components in order (refcount 1
        // at each step so `Arc::try_unwrap` takes the fast path and never
        // rebuilds the routing map):
        //   1. activation store  (ADR-0096 — token resolution)
        //   2. durable dispatch  (ADR-0095 D1 U-D1.4b — Prod-mode spawning)
        // All builders are called before the transport is distributed to
        // handlers so refcount stays 1 throughout.
        //
        // Rate-limiting: `..default()` leaves `rate_limit_per_minute` and
        // `tenant_rate_limit_per_minute` as `None`, which is correct here.
        // `with_durable_dispatch` installs DEFAULT_PER_TOKEN_RPM /
        // DEFAULT_PER_TENANT_RPM automatically — no composition-root discipline
        // required.  Override the limits by setting them in `webhook_config`
        // before calling the builder.
        let transport = nebula_api::transport::webhook::WebhookTransport::new(webhook_config);
        let transport = if let Some(store) = state.webhook_activation_store.clone() {
            transport.with_activation_store(store)
        } else {
            transport
        };
        let start = state
            .workflow_start
            .as_ref()
            .ok_or(TransportInitError::MissingWorkflowStart)?;
        let transport = transport.with_durable_dispatch(Arc::clone(start));
        Ok(state.with_webhook_transport(transport))
    }

    fn build_router(
        &self,
        state: AppState,
        _api_config: &ApiConfig,
    ) -> Result<Router, TransportInitError> {
        let webhook_transport = state
            .webhook_transport
            .ok_or(TransportInitError::MissingWebhookTransport)?;

        Ok(Router::new()
            .route("/health", get(health_ok))
            .route("/ready", get(health_ok))
            .merge(webhook_transport.router()))
    }
}

/// Activation inputs are supplied by the trusted worker deployment manifest.
/// The server identifies the selected worker artifact set, not its own binary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum WorkerFlavorActivationError {
    #[error("NEBULA_WORKER_ARTIFACT_SET_DIGEST is required from the deployment manifest")]
    MissingArtifactDigest,
    #[error("NEBULA_WORKER_ARTIFACT_SET_DIGEST must contain 64 lowercase hexadecimal characters")]
    InvalidArtifactDigest,
    #[error("core plugin manifest is invalid")]
    Manifest(#[from] nebula_plugin::ManifestError),
    #[error("core plugin registration failed")]
    Plugin(#[from] nebula_plugin::PluginError),
    #[error("worker registry could not be frozen")]
    Freeze(#[from] nebula_plugin::RegistryFreezeError),
}

pub(crate) fn worker_registry(
    artifact_digest: Result<String, std::env::VarError>,
) -> Result<Arc<nebula_plugin::FrozenPluginRegistry>, WorkerFlavorActivationError> {
    let artifact_digest = match artifact_digest {
        Ok(value) => value
            .parse::<nebula_core::ArtifactSetDigest>()
            .map_err(|_| WorkerFlavorActivationError::InvalidArtifactDigest)?,
        Err(std::env::VarError::NotPresent) => {
            return Err(WorkerFlavorActivationError::MissingArtifactDigest);
        },
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(WorkerFlavorActivationError::InvalidArtifactDigest);
        },
    };
    let plugin = nebula_plugin::ResolvedPlugin::from(nebula_plugin_core::CorePlugin::try_new()?)?;
    let mut registry = nebula_plugin::PluginRegistry::new();
    registry.register(Arc::new(plugin))?;
    let frozen = registry.freeze(
        artifact_digest,
        nebula_plugin::RuntimeContractVersion::current(),
    )?;
    Ok(Arc::new(frozen))
}

#[cfg(test)]
fn worker_flavor(
    artifact_digest: Result<String, std::env::VarError>,
) -> Result<nebula_plugin::WorkerFlavorContext, WorkerFlavorActivationError> {
    let registry = worker_registry(artifact_digest)?;
    Ok(nebula_plugin::WorkerFlavorContext::from_registry(&registry))
}

#[cfg(test)]
mod activation_tests {
    use super::*;

    #[test]
    fn worker_release_changes_exact_flavor_without_changing_plugin_keys() {
        let original = worker_flavor(Ok("71".repeat(32))).expect("original release");
        let replay = worker_flavor(Ok("71".repeat(32))).expect("same release");
        let replacement = worker_flavor(Ok("72".repeat(32))).expect("replacement release");
        assert_eq!(original, replay);
        assert_eq!(original.plugin_keys(), replacement.plugin_keys());
        assert_ne!(original.revision_id(), replacement.revision_id());
    }

    #[test]
    fn missing_or_malformed_worker_artifact_identity_cannot_activate_dispatch() {
        assert!(matches!(
            worker_flavor(Err(std::env::VarError::NotPresent)),
            Err(WorkerFlavorActivationError::MissingArtifactDigest)
        ));
        let error = worker_flavor(Ok("secret-canary".to_owned())).expect_err("invalid digest");
        assert!(matches!(
            error,
            WorkerFlavorActivationError::InvalidArtifactDigest
        ));
        assert!(!format!("{error:?} {error}").contains("secret-canary"));
    }
}

// ─── RealtimeTransport ────────────────────────────────────────────────────

/// Realtime transport placeholder.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RealtimeTransport;

impl ServerTransport for RealtimeTransport {
    fn name(&self) -> &'static str {
        "realtime"
    }

    fn bind_override_var(&self) -> Option<&'static str> {
        Some("REALTIME_BIND_ADDRESS")
    }

    fn build_router(
        &self,
        _state: AppState,
        _api_config: &ApiConfig,
    ) -> Result<Router, TransportInitError> {
        Ok(Router::new()
            .route("/health", get(health_ok))
            .route("/ready", get(health_ok))
            .route("/ws", get(ws_not_implemented)))
    }
}

async fn ws_not_implemented() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "error": "realtime transport scaffold is enabled, but websocket upgrade path is not wired yet"
        })),
    )
}
