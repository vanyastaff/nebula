//! Transport-oriented server runners.
//!
//! This module provides a small composition root that keeps shared startup
//! logic in one place while allowing different ingress transports (REST API,
//! webhook-only, realtime placeholder) to boot as separate binaries.

use std::{net::SocketAddr, sync::Arc};

use axum::{Router, http::StatusCode};
use thiserror::Error;

use crate::{ApiConfig, ApiConfigError, AppState, app};

mod api;
mod webhook;
mod websocket;

pub use api::ApiTransport;
pub use webhook::WebhookIngressTransport;
pub use websocket::RealtimeTransport;

/// Runtime errors for transport binaries.
#[derive(Debug, Error)]
pub enum ServerRunError {
    /// API configuration cannot be loaded from environment.
    #[error("failed to load API config")]
    Config(#[from] ApiConfigError),
    /// Address override is present but invalid.
    #[error("{var_name} invalid")]
    InvalidBindAddress {
        /// Environment variable name.
        var_name: &'static str,
        /// Parse error.
        source: std::net::AddrParseError,
    },
    /// Current transport cannot be started with available state/config.
    #[error("{0}")]
    Transport(#[from] TransportInitError),
    /// Listener/runtime error from axum server.
    #[error("server failed")]
    Io(#[from] std::io::Error),
}

/// Transport-specific initialization failure.
#[derive(Debug, Error)]
pub enum TransportInitError {
    /// Webhook transport was not attached to `AppState`.
    #[error(
        "webhook transport is not configured; attach it with AppState::with_webhook_transport before running nebula-webhook"
    )]
    MissingWebhookTransport,
    /// `WEBHOOK_BASE_URL` failed to parse.
    #[error("WEBHOOK_BASE_URL invalid")]
    InvalidWebhookBaseUrl {
        /// Parse error from `url`.
        source: url::ParseError,
    },
    /// Failed to construct a transport app context.
    #[error("{0}")]
    ContextFactory(String),
}

/// Shared application context passed through transport bootstrapping.
#[derive(Clone)]
pub struct AppContext {
    /// Parsed API configuration.
    pub api_config: ApiConfig,
    /// App state for handlers and middleware.
    pub state: AppState,
}

/// Factory for constructing [`AppContext`] at process startup.
pub trait AppContextFactory {
    /// Build a context from loaded API configuration.
    fn build_context(&self, api_config: ApiConfig) -> Result<AppContext, TransportInitError>;
}

/// Default local-first context factory used by transport binaries.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultAppContextFactory;

impl AppContextFactory for DefaultAppContextFactory {
    fn build_context(&self, api_config: ApiConfig) -> Result<AppContext, TransportInitError> {
        Ok(AppContext {
            state: default_state(&api_config),
            api_config,
        })
    }
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

/// Start a server binary for a selected transport profile.
pub async fn run_transport<T: ServerTransport>(transport: T) -> Result<(), ServerRunError> {
    ServerRuntime::default().run_transport(transport).await
}

/// Transport runtime orchestrator for binary composition roots.
pub struct ServerRuntime<F = DefaultAppContextFactory> {
    context_factory: F,
}

impl Default for ServerRuntime<DefaultAppContextFactory> {
    fn default() -> Self {
        Self {
            context_factory: DefaultAppContextFactory,
        }
    }
}

impl<F: AppContextFactory> ServerRuntime<F> {
    /// Build a runtime with a custom app-context factory.
    #[must_use]
    pub fn new(context_factory: F) -> Self {
        Self { context_factory }
    }

    /// Run a selected transport with this runtime.
    pub async fn run_transport<T: ServerTransport>(
        &self,
        transport: T,
    ) -> Result<(), ServerRunError> {
        let api_config = ApiConfig::from_env()?;
        let mut context = self.context_factory.build_context(api_config)?;
        let bind_address = resolve_bind_address(
            transport.bind_override_var(),
            context.api_config.bind_address,
        )?;
        context.state = transport.prepare_state(context.state, bind_address)?;
        let app = transport.build_router(context.state, &context.api_config)?;

        tracing::info!(transport = transport.name(), %bind_address, "starting transport");
        app::serve(app, bind_address).await?;
        Ok(())
    }
}

/// Build default local-first state used by transport binaries.
pub fn default_state(api_config: &ApiConfig) -> AppState {
    let workflow_repo = Arc::new(nebula_storage::InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(nebula_storage::repos::InMemoryControlQueueRepo::new());

    AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    )
    .with_api_keys(api_config.api_keys.clone())
}

fn resolve_bind_address(
    override_env: Option<&'static str>,
    fallback: SocketAddr,
) -> Result<SocketAddr, ServerRunError> {
    if let Some(var_name) = override_env
        && let Ok(raw) = std::env::var(var_name)
    {
        return parse_bind_address(var_name, &raw);
    }

    Ok(fallback)
}

fn parse_bind_address(var_name: &'static str, raw: &str) -> Result<SocketAddr, ServerRunError> {
    raw.parse::<SocketAddr>()
        .map_err(|source| ServerRunError::InvalidBindAddress { var_name, source })
}

pub(super) async fn health_ok() -> StatusCode {
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use super::{ServerRunError, parse_bind_address, resolve_bind_address};

    #[test]
    fn parse_bind_address_accepts_valid_socket_address() {
        let value =
            parse_bind_address("REALTIME_BIND_ADDRESS", "127.0.0.1:49999").expect("must parse");
        assert_eq!(value, SocketAddr::from(([127, 0, 0, 1], 49999)));
    }

    #[test]
    fn resolve_bind_address_returns_fallback_without_override() {
        let fallback = SocketAddr::from(([127, 0, 0, 1], 2));
        let value = resolve_bind_address(Some("UNSET_BIND"), fallback).expect("must fallback");
        assert_eq!(value, fallback);
    }

    #[test]
    fn parse_bind_address_rejects_invalid_override() {
        let key = "WEBHOOK_BIND_ADDRESS";
        let error = parse_bind_address(key, "invalid").expect_err("invalid override must fail");
        match error {
            ServerRunError::InvalidBindAddress { var_name, .. } => {
                assert_eq!(var_name, key);
            },
            other => panic!("unexpected error: {other}"),
        }
    }
}
