//! Transport-oriented server runners.
//!
//! This module provides a small composition root that keeps shared startup
//! logic in one place while allowing different ingress transports (REST API,
//! webhook-only, realtime placeholder) to boot as separate binaries.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{Router, http::StatusCode};
use thiserror::Error;

use crate::{
    ApiConfig, ApiConfigError, AppState, app,
    config::IdempotencyBackend,
    middleware::{IdempotencyStore, InMemoryIdempotencyStore},
};

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
    /// `API_IDEMPOTENCY_BACKEND` selects a backend that the current build
    /// cannot satisfy.
    ///
    /// Today this fires when an operator sets
    /// `API_IDEMPOTENCY_BACKEND=postgres` while Phase E (PG-backed store) is
    /// not yet shipped. Per ADR-0048 fail-closed contract, the binary
    /// refuses to boot rather than silently fall back to in-memory dedup.
    #[error(
        "API_IDEMPOTENCY_BACKEND={requested} requires {requirement}; set API_IDEMPOTENCY_BACKEND=memory or land the missing wiring"
    )]
    IdempotencyBackendUnavailable {
        /// Backend the operator requested.
        requested: &'static str,
        /// What is missing for that backend to work.
        requirement: &'static str,
    },
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
            state: default_state(&api_config)?,
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
///
/// Returns an error when `api_config.idempotency.backend` requests a
/// backend that this build cannot satisfy (today: `Postgres` before
/// Phase E lands). The fail-closed contract is documented in ADR-0048.
pub fn default_state(api_config: &ApiConfig) -> Result<AppState, TransportInitError> {
    let workflow_repo = Arc::new(nebula_storage::InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(nebula_storage::repos::InMemoryControlQueueRepo::new());

    let idempotency_store = build_idempotency_store(api_config)?;

    Ok(AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    )
    .with_api_keys(api_config.api_keys.clone())
    .with_idempotency_store(idempotency_store))
}

/// Construct the idempotency store from `api_config.idempotency`.
///
/// `Memory` outside dev mode emits a startup `tracing::warn!` so the
/// "dedup state is lost on restart and across runners" failure mode is
/// visible in operational logs (per ADR-0048).
///
/// `Postgres` returns [`TransportInitError::IdempotencyBackendUnavailable`]
/// until Phase E ships the PG-backed impl — silent fallback to memory is
/// rejected per `feedback_no_shims.md`.
pub fn build_idempotency_store(
    api_config: &ApiConfig,
) -> Result<Arc<dyn IdempotencyStore>, TransportInitError> {
    match api_config.idempotency.backend {
        IdempotencyBackend::Memory => {
            let env_mode = std::env::var("NEBULA_ENV").unwrap_or_default();
            let is_dev = matches!(env_mode.as_str(), "development" | "dev" | "local");
            if !is_dev {
                tracing::warn!(
                    backend = "memory",
                    nebula_env = %env_mode,
                    "idempotency: in-memory store selected — dedup state is lost on restart and across runners"
                );
            }
            if api_config.idempotency.sweep_interval_secs > 0
                && api_config.idempotency.sweep_interval_secs < 60
            {
                tracing::warn!(
                    sweep_interval_secs = api_config.idempotency.sweep_interval_secs,
                    "idempotency: sweep interval < 60s; consider raising it to avoid hot-loop sweeps"
                );
            }
            let store = InMemoryIdempotencyStore::with_ttl_and_capacity(
                Duration::from_secs(api_config.idempotency.ttl_secs),
                api_config.idempotency.max_entries,
            );
            Ok(Arc::new(store))
        },
        IdempotencyBackend::Postgres => Err(TransportInitError::IdempotencyBackendUnavailable {
            requested: "postgres",
            requirement: "Phase E (PgIdempotencyStore + migration 0024)",
        }),
    }
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
