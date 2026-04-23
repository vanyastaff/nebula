use std::net::SocketAddr;

use axum::{Router, routing::get};
use url::Url;

use super::{ServerTransport, TransportInitError, health_ok};
use crate::{ApiConfig, AppState};

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

        let webhook_config = crate::services::webhook::WebhookTransportConfig {
            base_url,
            ..crate::services::webhook::WebhookTransportConfig::default()
        };

        Ok(
            state.with_webhook_transport(crate::services::webhook::WebhookTransport::new(
                webhook_config,
            )),
        )
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
