//! Transport-oriented server runners.
//!
//! This module provides a small composition root that keeps shared startup
//! logic in one place while allowing different ingress transports (REST API,
//! webhook-only, realtime placeholder) to boot as separate binaries.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use thiserror::Error;

use nebula_api::{
    ApiConfig, ApiConfigError, AppState, app,
    config::IdempotencyBackend,
    middleware::{IdempotencyStore, InMemoryIdempotencyStore},
};

use crate::transport::ServerTransport;

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
    /// Failed to construct a transport app context — used by the
    /// bootstrap membership-store seed (always) and the postgres-gated
    /// idempotency store construction.
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

/// Start a server binary for a selected transport profile.
pub async fn run_transport<T: ServerTransport>(transport: T) -> Result<(), ServerRunError> {
    ServerRuntime::new().run_transport(transport).await
}

/// Transport runtime orchestrator for binary composition roots.
pub struct ServerRuntime;

impl ServerRuntime {
    /// Create a new runtime.
    pub fn new() -> Self {
        Self
    }

    /// Run a selected transport with this runtime.
    pub async fn run_transport<T: ServerTransport>(
        &self,
        transport: T,
    ) -> Result<(), ServerRunError> {
        let api_config = ApiConfig::from_env()?;
        let mut state = default_state(&api_config)?;
        let bind_address =
            resolve_bind_address(transport.bind_override_var(), api_config.bind_address)?;
        state = transport.prepare_state(state, bind_address)?;
        // Attach the idempotency store inside the async context so the
        // PG-backed path can await sqlx pool construction. Memory-backed
        // builds resolve immediately; PG-backed builds also fail closed
        // here when the feature is missing or `DATABASE_URL` is unset
        // (per ADR-0048).
        let idempotency_store = build_idempotency_store(&api_config).await?;
        state = state.with_idempotency_store(idempotency_store);
        let app = transport.build_router(state, &api_config)?;

        tracing::info!(transport = transport.name(), %bind_address, "starting transport");
        app::serve(app, bind_address).await?;
        Ok(())
    }
}

/// Build default local-first state used by transport binaries.
///
/// The idempotency store is **not** attached here — it is wired
/// asynchronously by [`ServerRuntime::run_transport`] so the PG-backed
/// path can `await` the sqlx pool construction.
pub fn default_state(api_config: &ApiConfig) -> Result<AppState, TransportInitError> {
    let workflow_repo = Arc::new(nebula_storage::InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(nebula_storage::repos::InMemoryControlQueueRepo::new());

    // Plane-A identity backend. `InMemoryAuthBackend` is the
    // production-quality default (Argon2id passwords, RFC 6238 TOTP,
    // SHA-256 PAT lookup) — the same §4.5-honest "the real impl is the
    // in-memory one" posture the control queue uses
    // (`InMemoryControlQueueRepo`). There is no storage-backed
    // alternative to wire: `nebula_storage` ships no implementation of
    // `UserRepo` / `PatRepo` / `SessionRepo` (see
    // `nebula_storage::repos` module docs — those traits are
    // definition-only). Wiring this makes the `me/*` profile + PAT
    // endpoints work end-to-end; without it they fail closed with 503.
    let auth_backend = nebula_api::domain::auth::backend::InMemoryAuthBackend::new().into_arc();

    // Membership store (RBAC role index). Same §4.5-honest posture: the
    // in-memory impl *is* the real backing (no storage-backed membership
    // adapter exists). Wiring it ACTIVATES RBAC enforcement on every
    // org/workspace route, so it MUST be seeded with a bootstrap org
    // owner — otherwise the gate dead-locks (every member-add requires a
    // pre-existing admin, the classic chicken-and-egg). The seed is the
    // root-of-trust first admin; it is process-local and lost on restart
    // (canon §11.6 — documented in
    // `nebula_api::domain::org::membership` + `crates/api/README.md`).
    // Override the seeded identities via `NEBULA_BOOTSTRAP_ORG_ID` /
    // `NEBULA_BOOTSTRAP_OWNER_ID` so an operator can pin a real admin;
    // the defaults are deterministic so a fresh dev/test server is
    // immediately usable.
    let membership_store = build_bootstrap_membership_store()?;

    Ok(AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    )
    .with_api_keys(api_config.api_keys.clone())
    .with_auth_backend(auth_backend)
    .with_membership_store(membership_store))
}

/// Default deterministic bootstrap org / owner identities.
///
/// Fixed prefixed-ULIDs so a fresh local-first server has a usable RBAC
/// gate out of the box. Operators pin a real admin via the
/// `NEBULA_BOOTSTRAP_ORG_ID` / `NEBULA_BOOTSTRAP_OWNER_ID` env vars.
const DEFAULT_BOOTSTRAP_ORG_ID: &str = "org_00000000000000000000000001";
const DEFAULT_BOOTSTRAP_OWNER_ID: &str = "usr_00000000000000000000000001";

/// Build the seeded in-memory [`MembershipStore`].
///
/// The seed grants `OrgOwner` on the bootstrap org to the bootstrap
/// principal. A malformed env override fails closed
/// ([`TransportInitError::ContextFactory`]) rather than silently falling
/// back to an unseeded (dead-locked) gate — same fail-closed contract as
/// the idempotency backend selector (`feedback_no_shims`). The
/// `nebula_core` id parsing lives in the API tier
/// (`InMemoryMembershipStore::seeded_bootstrap`) per ADR-0047 §3.
fn build_bootstrap_membership_store()
-> Result<Arc<nebula_api::domain::org::InMemoryMembershipStore>, TransportInitError> {
    let org_raw = std::env::var("NEBULA_BOOTSTRAP_ORG_ID")
        .unwrap_or_else(|_| DEFAULT_BOOTSTRAP_ORG_ID.to_owned());
    let owner_raw = std::env::var("NEBULA_BOOTSTRAP_OWNER_ID")
        .unwrap_or_else(|_| DEFAULT_BOOTSTRAP_OWNER_ID.to_owned());

    let store =
        nebula_api::domain::org::InMemoryMembershipStore::seeded_bootstrap(&org_raw, &owner_raw)
            .map_err(|e| TransportInitError::ContextFactory(e.to_string()))?;

    tracing::info!(
        bootstrap_org = %org_raw,
        bootstrap_owner = %owner_raw,
        "membership store seeded with bootstrap org owner (process-local; canon §11.6)"
    );

    Ok(store.into_arc())
}

/// Construct the idempotency store from `api_config.idempotency`.
///
/// `Memory` outside dev mode emits a startup `tracing::warn!` so the
/// "dedup state is lost on restart and across runners" failure mode is
/// visible in operational logs (per ADR-0048).
///
/// `Postgres` requires the `nebula-api/postgres` cargo feature **and** a
/// reachable `DATABASE_URL`; either missing component fails closed with
/// [`TransportInitError::IdempotencyBackendUnavailable`] (silent
/// fallback to memory is rejected per `feedback_no_shims.md`).
pub async fn build_idempotency_store(
    api_config: &ApiConfig,
) -> Result<Arc<dyn IdempotencyStore>, TransportInitError> {
    match api_config.idempotency.backend {
        IdempotencyBackend::Memory => {
            warn_memory_outside_dev();
            warn_short_sweep_interval(api_config.idempotency.sweep_interval_secs);
            let store = InMemoryIdempotencyStore::with_ttl_and_capacity(
                Duration::from_secs(api_config.idempotency.ttl_secs),
                api_config.idempotency.max_entries,
            );
            Ok(Arc::new(store))
        },
        IdempotencyBackend::Postgres => build_pg_idempotency_store(api_config).await,
    }
}

fn warn_memory_outside_dev() {
    let env_mode = std::env::var("NEBULA_ENV").unwrap_or_default();
    let is_dev = matches!(env_mode.as_str(), "development" | "dev" | "local");
    if !is_dev {
        tracing::warn!(
            backend = "memory",
            nebula_env = %env_mode,
            "idempotency: in-memory store selected — dedup state is lost on restart and across runners"
        );
    }
}

fn warn_short_sweep_interval(sweep_interval_secs: u64) {
    if sweep_interval_secs > 0 && sweep_interval_secs < 60 {
        tracing::warn!(
            sweep_interval_secs,
            "idempotency: sweep interval < 60s; consider raising it to avoid hot-loop sweeps"
        );
    }
}

#[cfg(feature = "postgres")]
async fn build_pg_idempotency_store(
    api_config: &ApiConfig,
) -> Result<Arc<dyn IdempotencyStore>, TransportInitError> {
    use nebula_storage::pg::PgIdempotencyStore;
    use sqlx::postgres::PgPoolOptions;

    use nebula_api::middleware::idempotency::StorageBackedIdempotencyStore;

    let url = std::env::var("DATABASE_URL").map_err(|_| {
        TransportInitError::IdempotencyBackendUnavailable {
            requested: "postgres",
            requirement: "DATABASE_URL must be set when API_IDEMPOTENCY_BACKEND=postgres",
        }
    })?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .map_err(|err| {
            TransportInitError::ContextFactory(format!(
                "idempotency: failed to connect to DATABASE_URL for PG-backed store: {err}"
            ))
        })?;
    warn_short_sweep_interval(api_config.idempotency.sweep_interval_secs);
    tracing::info!(backend = "postgres", "idempotency: PG-backed store wired");
    let pg_repo = Arc::new(PgIdempotencyStore::new(pool));
    let store: Arc<dyn IdempotencyStore> = Arc::new(StorageBackedIdempotencyStore::new(
        pg_repo,
        Duration::from_secs(api_config.idempotency.ttl_secs),
    ));
    Ok(store)
}

#[cfg(not(feature = "postgres"))]
async fn build_pg_idempotency_store(
    _api_config: &ApiConfig,
) -> Result<Arc<dyn IdempotencyStore>, TransportInitError> {
    Err(TransportInitError::IdempotencyBackendUnavailable {
        requested: "postgres",
        requirement: "build with `nebula-api/postgres` cargo feature to link sqlx + PgIdempotencyStore",
    })
}

pub(crate) fn resolve_bind_address(
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

pub(crate) fn parse_bind_address(
    var_name: &'static str,
    raw: &str,
) -> Result<SocketAddr, ServerRunError> {
    raw.parse::<SocketAddr>()
        .map_err(|source| ServerRunError::InvalidBindAddress { var_name, source })
}

// Used by transport impls (webhook, websocket) for health/ready routes.
pub(crate) async fn health_ok() -> axum::http::StatusCode {
    axum::http::StatusCode::OK
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
