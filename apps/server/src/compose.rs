//! Transport-oriented server runners.
//!
//! This module provides a small composition root that keeps shared startup
//! logic in one place while allowing different ingress transports (REST API,
//! webhook-only, realtime placeholder) to boot as separate binaries.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use thiserror::Error;

use nebula_api::{
    ApiConfig, ApiConfigError, AppState, app,
    config::{AuthBackendKind, IdempotencyBackend},
    domain::auth::backend::{AuthBackend, InMemoryAuthBackend},
    middleware::{IdempotencyStore, InMemoryIdempotencyStore},
    ports::email::{EchoSink, EmailPort},
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
    /// Failed to construct a transport app context.
    #[error("{0}")]
    #[cfg_attr(
        not(feature = "postgres"),
        expect(
            dead_code,
            reason = "constructed only in the postgres-gated build_pg_idempotency_store / build_pg_auth_backend arms"
        )
    )]
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
    /// `API_AUTH_BACKEND` selects an identity backend that the current
    /// build cannot satisfy.
    ///
    /// Today this fires when an operator sets
    /// `API_AUTH_BACKEND=postgres` without the `nebula-api/postgres`
    /// cargo feature compiled in, or without `DATABASE_URL` reachable.
    /// Mirrors the fail-closed posture of
    /// [`Self::IdempotencyBackendUnavailable`] — silently falling back
    /// to the in-memory identity backend would be a publicly-known
    /// auth-bypass surface in any deployment that thought it had
    /// requested durable identity.
    #[error(
        "API_AUTH_BACKEND={requested} requires {requirement}; set API_AUTH_BACKEND=memory or land the missing wiring"
    )]
    AuthBackendUnavailable {
        /// Backend the operator requested.
        requested: &'static str,
        /// What is missing for that backend to work.
        requirement: &'static str,
    },
    /// The credential-schema port could not be built (ADR-0052 P4 —
    /// first-party credential registration failed; a composition bug).
    /// Carried as a `String` so this crate needs no `nebula-credential`
    /// dependency (the typed `RegisterError` stays inside `nebula-api`).
    #[error("credential-schema port init failed: {0}")]
    CredentialSchemaInit(String),
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
        // Build ONE shared `Arc<dyn EmailPort>` and pass the same Arc
        // to both `AppState::email_port` and the selected auth backend.
        // Today this is the dev `EchoSink`; a future SMTP transport
        // swaps in here without changing either consumer. Forward-compat
        // non-auth email consumers (org invitations, billing notices)
        // read from `state.email_port` and work uniformly regardless of
        // which auth backend is wired.
        let email_port: Arc<dyn EmailPort> = Arc::new(EchoSink::default());
        let auth_backend = build_auth_backend(&api_config, Arc::clone(&email_port)).await?;
        state = state
            .with_auth_backend(auth_backend)
            .with_email_port(email_port);
        let app = transport.build_router(state, &api_config)?;

        tracing::info!(transport = transport.name(), %bind_address, "starting transport");
        app::serve(app, bind_address).await?;
        Ok(())
    }
}

/// Build default local-first state used by transport binaries.
///
/// The execution / workflow / control-queue surface is the scoped
/// storage port: the in-memory adapters wrapped in the `nebula-tenancy`
/// scoping decorators bound to the local-first placeholder scope. One
/// shared execution-store core backs the control queue and journal so a
/// `commit`/`enqueue` is observable through every reader (the same
/// wiring contract the conformance harness uses). A single
/// workflow-version store instance is shared between the workflow-CRUD
/// path and the resume/definition path so a version published via the
/// workflow handlers is readable through the execution accessor.
///
/// The idempotency store is **not** attached here — it is wired
/// asynchronously by [`ServerRuntime::run_transport`] so the PG-backed
/// path can `await` the sqlx pool construction. The Plane-A auth
/// backend follows the same pattern: [`build_auth_backend`] runs in
/// the async context (so the PG arm can `await` the sqlx pool) and
/// `default_state` no longer wires an unconditional in-memory backend
/// — the conditional builder owns the slot now.
pub fn default_state(api_config: &ApiConfig) -> Result<AppState, TransportInitError> {
    // The execution / workflow / control-queue port wiring (the
    // six-handle in-memory-adapter + `nebula-tenancy`-decorator stack) is
    // the single-source-of-truth `AppState::in_memory`. This composition
    // root only layers the deployment-specific ports on top of it
    // (identity backend, credential-schema, api keys) so the wiring
    // contract cannot drift between the binary and the runnable example.

    // Plane-A identity backend is wired asynchronously by
    // [`build_auth_backend`] inside [`ServerRuntime::run_transport`]
    // so the PG-backed arm can `await` the sqlx pool. The selector
    // (`AuthBackendKind::Memory` vs `Postgres`) is honored there with
    // the same fail-closed contract `build_idempotency_store` uses.

    // NOTE: `membership_store` is intentionally LEFT UNWIRED (`None`) in
    // the default local-first composition.
    //
    // Wiring a `MembershipStore` activates RBAC enforcement on every
    // org/workspace route (the `is_some()` guard in
    // `nebula_api::middleware::rbac`). With this default `AuthBackend`
    // empty (no users registered — `register_user` mints a *random*
    // `UserId`), no principal could authenticate as any auto-seeded
    // bootstrap owner, so a seeded store would deadlock EVERY
    // org/workspace route with a 404 (a deployment-level §4.5 false
    // capability: the spec would advertise org member endpoints no real
    // caller can reach). Auto-seeding a fixed admin identity would also
    // be a hardcoded-credential / privileged-by-default surface (canon
    // §12.5) — both are strictly worse than honest degradation.
    //
    // With `membership_store == None`: RBAC's `is_some()` guard stays
    // inert (no spurious 404 — identical to every other route today),
    // and the org member handlers' port-absent path returns an honest
    // **503** (same posture as `me/*` when `auth_backend` is absent, and
    // as Postgres-for-durable-idempotency: the production path is
    // explicitly provisioned, never silently faked). An operator/
    // integrator provisions org membership by wiring a `MembershipStore`
    // whose bootstrap owner is ALSO authenticatable via the wired
    // `AuthBackend` (the library constructor
    // `nebula_api::domain::org::InMemoryMembershipStore::seeded_bootstrap`
    // is the documented entry point). The feature is implemented and
    // tested (`crates/api/tests/org_e2e.rs`) — it is simply not
    // auto-enabled in the un-provisioned default binary, by design.
    // Process-local durability + this provisioning contract are
    // documented in `crates/api/README.md` ("Org membership durability")
    // and `nebula_api::domain::org` module docs (canon §11.6).

    // ADR-0052 P4: wire the credential-schema port (first-party types
    // registered) so the write path validates `data` before persist and
    // the catalog exposes `json_schema()`. The concrete impl lives in
    // `nebula-api` (deny.toml-allow-listed `nebula-credential` consumer),
    // so this composition crate needs no `nebula-credential`/
    // `nebula-schema` dep — just the api constructor.
    let credential_schema =
        nebula_api::ports::credential_schema_registry::try_default_registry_port()
            .map_err(|e| TransportInitError::CredentialSchemaInit(e.to_string()))?;

    Ok(AppState::in_memory(api_config.jwt_secret.clone())
        .with_api_keys(api_config.api_keys.clone())
        .with_credential_schema(credential_schema))
}

/// Construct the Plane-A authentication backend from `api_config.auth`.
///
/// `Memory` builds an in-process [`InMemoryAuthBackend`] wired to the
/// shared `email_port` so verification / reset mails flow through the
/// same transport the rest of the app uses. `Postgres` requires the
/// `nebula-api/postgres` cargo feature **and** a reachable
/// `DATABASE_URL`; either missing component fails closed with
/// [`TransportInitError::AuthBackendUnavailable`] (silent fallback to
/// in-memory would be an undetected auth-bypass for any operator who
/// thought they had requested durable identity).
///
/// Both arms receive the SAME `Arc<dyn EmailPort>` — the in-memory
/// backend drops its built-in default echo sink in favour of the
/// shared transport so callers can introspect deliveries against one
/// known port instead of guessing which sink owns the inbox.
///
/// Today this builder constructs its own `sqlx::Pool<Postgres>`
/// alongside the idempotency pool; consolidating the two onto one
/// shared pool is a follow-up.
pub async fn build_auth_backend(
    api_config: &ApiConfig,
    email_port: Arc<dyn EmailPort>,
) -> Result<Arc<dyn AuthBackend>, TransportInitError> {
    match api_config.auth.backend {
        AuthBackendKind::Memory => Ok(Arc::new(
            InMemoryAuthBackend::new().with_email_port(email_port),
        )),
        AuthBackendKind::Postgres => build_pg_auth_backend(email_port).await,
    }
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

#[cfg(feature = "postgres")]
async fn build_pg_auth_backend(
    email_port: Arc<dyn EmailPort>,
) -> Result<Arc<dyn AuthBackend>, TransportInitError> {
    use nebula_api::domain::auth::backend::PgAuthBackend;
    use sqlx::postgres::PgPoolOptions;

    let url =
        std::env::var("DATABASE_URL").map_err(|_| TransportInitError::AuthBackendUnavailable {
            requested: "postgres",
            requirement: "DATABASE_URL must be set when API_AUTH_BACKEND=postgres",
        })?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .map_err(|err| {
            TransportInitError::ContextFactory(format!(
                "auth: failed to connect to DATABASE_URL for PG-backed backend: {err}"
            ))
        })?;
    tracing::info!(
        backend = "postgres",
        "auth: PG-backed identity backend wired"
    );
    let backend: Arc<dyn AuthBackend> = Arc::new(PgAuthBackend::new(pool, email_port));
    Ok(backend)
}

#[cfg(not(feature = "postgres"))]
async fn build_pg_auth_backend(
    _email_port: Arc<dyn EmailPort>,
) -> Result<Arc<dyn AuthBackend>, TransportInitError> {
    Err(TransportInitError::AuthBackendUnavailable {
        requested: "postgres",
        requirement: "build with `nebula-api/postgres` cargo feature to link sqlx + PgAuthBackend",
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
