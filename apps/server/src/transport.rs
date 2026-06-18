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
pub enum Transport {
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
pub trait ServerTransport {
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
pub struct ApiTransport;

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
pub struct WebhookIngressTransport;

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
        let transport = if let Some(dedup) = state.trigger_dedup_inbox.clone() {
            let resolver = nebula_api::transport::webhook::WebhookTransport::default_resolver();
            let version_store = Arc::clone(&state.workflow_version_store);
            transport.with_durable_dispatch(dedup, resolver, version_store)
        } else {
            transport
        };
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

// ─── RealtimeTransport ────────────────────────────────────────────────────

/// Realtime transport placeholder.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealtimeTransport;

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
