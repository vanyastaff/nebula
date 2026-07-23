//! Transport-oriented server runners.
//!
//! This module provides a small composition root that keeps shared startup
//! logic in one place while allowing different ingress transports (REST API,
//! webhook-only, realtime placeholder) to boot as separate binaries.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use thiserror::Error;

use nebula_api::{
    ApiConfig, ApiConfigError, AppState, OAuthIdentityRuntime, OAuthRuntimeBuildError,
    TelemetryGuard, TelemetryInitError, app,
    config::{
        AuthBackendKind, ExecutionBackendKind, IdempotencyBackend, OAuthProvidersConfig,
        SmtpTlsMode,
    },
    domain::auth::backend::{AuthBackend, InMemoryAuthBackend},
    middleware::{IdempotencyStore, InMemoryIdempotencyStore},
    ports::email::{EchoSink, EmailPort},
};
use nebula_credential::{CredentialController, CredentialTenantAuthority};
use nebula_metrics::{MetricsRegistry, OtlpInitError};
use nebula_storage::credential::KeyProvider;

use crate::{
    credential_composition::{compose_first_party_runtime, resolve_first_party_key_provider},
    credential_runtime::{ServerCredentialAuthority, ServerCredentialGateway},
    email::{SmtpEmailPort, SmtpEmailPortBuildError},
    transport::ServerTransport,
};

/// Runtime errors for transport binaries.
#[derive(Debug, Error)]
pub(crate) enum ServerRunError {
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
    /// OTLP metrics pipeline failed to attach to the telemetry guard.
    ///
    /// Surfacing this as a hard error matches the fail-closed policy of the other OTLP
    /// install sites — silent fallback would mean operators who set
    /// `OTEL_EXPORTER_OTLP_ENDPOINT` see no metrics in the collector and no diagnostic in the
    /// startup log.
    #[error("OTLP metrics exporter failed to attach")]
    MetricsExporter(#[source] OtlpInitError),
    /// Telemetry bootstrap (`init_api_telemetry`) failed. Most likely cause is an
    /// unreachable / malformed `OTEL_EXPORTER_OTLP_ENDPOINT` that breaks the OTLP
    /// `SpanExporter` build. Same fail-closed reasoning as [`Self::MetricsExporter`]:
    /// operators who set the env var explicitly want OTLP, so we refuse to silently fall
    /// back to an exporter-less tracer. Carries the typed [`TelemetryInitError`] as the
    /// `source` so the error chain reaches the startup-log formatter intact.
    #[error("telemetry bootstrap failed")]
    Telemetry(#[source] TelemetryInitError),
}

/// Transport-specific initialization failure.
#[derive(Debug, Error)]
pub(crate) enum TransportInitError {
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
    /// The `CredentialService` facade could not be composed (registry
    /// registration, encryption key provider init, dispatch-ops
    /// registration, or the final secure-store build failed). Most common
    /// in production: `NEBULA_CRED_MASTER_KEY` is unset or malformed —
    /// fail closed rather than boot with a weak/absent key. Carried as a
    /// `String` to keep the process-level startup error stable while the
    /// detailed composition error remains apps-owned.
    #[error("credential service init failed: {0}")]
    CredentialServiceInit(String),
    /// An OAuth identity-provider config entry failed boot-time
    /// validation per ADR-0085 REQ-compose-001 Invariant 1.
    ///
    /// Failure cases are empty `client_id` / `client_secret` and a missing or
    /// invalid `ApiConfig::public_url` when any OAuth provider is declared.
    /// Provider endpoints, scopes, and token authentication are fixed runtime
    /// policy rather than operator configuration; runtime-policy construction
    /// failures surface through [`Self::OAuthRuntimeInit`].
    /// `provider` is the snake-case enum string (`"google"` /
    /// `"github"`); `reason` is a short stable
    /// keyword the operator can grep for in the docs.
    #[error(
        "OAuth provider `{provider}` config invalid: {reason}; fix the API_AUTH_OAUTH_<UPPERCASE_PROVIDER>_* env vars (e.g. API_AUTH_OAUTH_GOOGLE_CLIENT_ID) or remove the provider"
    )]
    OAuthProviderConfigInvalid {
        /// Provider name (snake_case OAuthProvider enum variant).
        provider: String,
        /// Stable reason keyword (`client_id_required`,
        /// `client_secret_required`, or `public_url_required`).
        reason: &'static str,
    },
    /// The fixed Plane-A OAuth egress/runtime policy could not be built.
    #[error("OAuth identity runtime initialization failed")]
    OAuthRuntimeInit {
        /// Secret-free fixed constructor failure.
        #[source]
        source: OAuthRuntimeBuildError,
    },
    /// `API_SMTP_HOST` is set but the `SmtpEmailPort` constructor
    /// rejected the resolved config (invalid `from_address` mailbox,
    /// lettre TLS-parameter construction error, etc.).
    ///
    /// Per the fail-closed contract in [`nebula_api::ApiConfig::smtp`]:
    /// silently falling back to `EchoSink` when an operator explicitly
    /// asked for SMTP would swallow verification mails in production
    /// with no diagnostic. We refuse to boot instead.
    #[error("SMTP email transport init failed: {source}")]
    SmtpEmailPortInit {
        /// Underlying `SmtpEmailPortBuildError` from the constructor.
        #[source]
        source: SmtpEmailPortBuildError,
    },
    /// `API_EXECUTION_BACKEND` selects a backend that the current build
    /// cannot satisfy (e.g. `postgres` without the `postgres` feature).
    ///
    /// Mirrors the fail-closed posture of
    /// [`Self::IdempotencyBackendUnavailable`]: silently falling back to
    /// in-memory when the operator asked for durable execution state would
    /// mean execution rows vanish on restart with no diagnostic.
    #[error(
        "API_EXECUTION_BACKEND={requested} requires {requirement}; \
         set API_EXECUTION_BACKEND=memory or land the missing wiring"
    )]
    ExecutionBackendUnavailable {
        /// Backend the operator requested.
        requested: &'static str,
        /// What is missing for that backend to work.
        requirement: &'static str,
    },
    /// The execution-store database (SQLite file or Postgres pool) could
    /// not be opened or the schema DDL failed. Carried as a `String` so the
    /// typed `sqlx::Error` does not escape this crate boundary.
    #[error("execution-store database init failed: {0}")]
    ExecutionDatabase(String),
}

/// The six execution/workflow/control-queue handles wired into `AppState::new`.
///
/// Produced by [`build_execution_stores`] and consumed immediately by
/// [`default_state`]. The three-backend shape (Memory / SQLite / Postgres)
/// is resolved once at startup; downstream code sees only the trait objects.
///
/// `trigger_dedup_inbox` is `Some` on ALL backends:
/// - Memory: shares the same `Arc<Mutex<SharedState>>` as the control queue and journal
///   (ordering invariant — `new(&exec_store)` before `Arc::new(exec_store)`).
/// - SQLite: `SqliteTriggerDedupInbox` wraps the WAL pool.
/// - Postgres: `PgTriggerDedupInbox` wraps the PG pool.
///
/// `WebhookIngressTransport::prepare_state` only installs `with_durable_dispatch` when
/// `trigger_dedup_inbox` is `Some` — returning `None` here silently disables durable
/// webhook dispatch for that backend, which is the defect this `Some` prevents.
pub(crate) struct ExecutionStoreBundle {
    workflow_store: Arc<dyn nebula_storage_port::store::WorkflowStore>,
    workflow_version_store: Arc<dyn nebula_storage_port::store::WorkflowVersionStore>,
    execution_store: Arc<dyn nebula_storage_port::store::ExecutionStore>,
    node_result_store: Arc<dyn nebula_storage_port::store::NodeResultStore>,
    journal_reader: Arc<dyn nebula_storage_port::store::ExecutionJournalReader>,
    control_queue: Arc<dyn nebula_storage_port::store::ControlQueue>,
    trigger_dedup_inbox: Option<Arc<dyn nebula_storage_port::store::TriggerDedupInbox>>,
    /// Resume-token store — must share the same backend pool as `execution_store`
    /// so that tokens minted by the engine (via `TransitionBatch`) are visible to
    /// the `POST /resume` handler's `consume` call.  Using an independent
    /// in-memory store on a durable backend silently breaks the round-trip:
    /// the engine writes to the pool; the producer reads from a different empty store.
    resume_token_store: Arc<dyn nebula_storage_port::store::ResumeTokenStore>,
    /// Resume producer (W-S3d Option B1) — the atomic consume+enqueue seam the
    /// `POST /resume` handler uses. MUST share the SAME backend pool / shared
    /// state as `execution_store` and `control_queue` so the token DELETE and the
    /// `Resume` INSERT commit in one transaction. A mismatched pool would split
    /// the burn and the enqueue across two backends — exactly the durability gap
    /// this seam closes.
    resume_producer: Arc<dyn nebula_storage_port::store::ResumeProducer>,
}

/// Build the execution-store bundle for the configured backend.
///
/// `Memory` resolves immediately (same in-memory adapters as `AppState::in_memory`).
/// `Sqlite` opens a WAL-mode file pool, calls `init_schema`, and wraps the stores.
/// `Postgres` follows the same pattern behind `#[cfg(feature = "postgres")]`; the
/// `#[cfg(not(...))]` twin always returns
/// [`TransportInitError::ExecutionBackendUnavailable`] — never silently falls back
/// to Memory (fail-closed per the feedback_no_shims invariant).
///
/// NodeResult and Checkpoint have no durable implementation and stay in-memory on
/// all backends — they store transient per-execution data (node output slots,
/// stateful action checkpoints); durability is provided by the execution-store
/// state machine's single JSON blob, not by these auxiliary stores. On a crash the
/// reclaim sweep re-delivers the job and the engine re-executes from the last
/// persisted state.
async fn build_execution_stores(
    api_config: &ApiConfig,
) -> Result<ExecutionStoreBundle, TransportInitError> {
    match api_config.execution.backend {
        ExecutionBackendKind::Memory => {
            warn_execution_memory_outside_dev();
            build_memory_execution_stores()
        },
        ExecutionBackendKind::Sqlite => build_sqlite_execution_stores(api_config).await,
        ExecutionBackendKind::Postgres => build_pg_execution_stores(api_config).await,
        // `ExecutionBackendKind` is `#[non_exhaustive]` so a wildcard arm is required
        // by the compiler even though all three current variants are handled above.
        // A new variant added to the enum must be explicitly handled here — the panic
        // ensures the compiler forces an update to this match rather than silently
        // falling back to a wrong backend.
        _ => unreachable!(
            "unrecognised ExecutionBackendKind variant — update build_execution_stores to handle it"
        ),
    }
}

/// In-memory bundle — same wiring as `AppState::in_memory` but returned as a
/// bundle so `default_state` can call `AppState::new(...)` uniformly regardless
/// of backend. `AppState::in_memory` itself is NOT called here to avoid
/// duplicating the trigger-dedup-inbox wiring in the Memory path.
fn build_memory_execution_stores() -> Result<ExecutionStoreBundle, TransportInitError> {
    use nebula_storage::inmem::{
        InMemoryControlQueue, InMemoryExecutionStore, InMemoryJournalReader,
        InMemoryNodeResultStore, InMemoryTriggerDedupInbox, InMemoryWorkflowStore,
        InMemoryWorkflowVersionStore,
    };

    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let journal = InMemoryJournalReader::new(&exec_store);
    // TriggerDedupInbox must share the same `Arc<Mutex<SharedState>>` as the
    // control queue and journal — `new(&exec_store)` must be called BEFORE
    // `Arc::new(exec_store)` moves ownership.
    let trigger_dedup_inbox = InMemoryTriggerDedupInbox::new(&exec_store);
    // Resume-token store shares the same SharedState as the exec_store so that
    // tokens committed via TransitionBatch are visible to the POST /resume handler.
    let resume_token_store = exec_store.resume_token_store();
    // Resume producer shares the SAME SharedState (token map + control queue) so
    // the consume + enqueue happen under one lock.
    let resume_producer = exec_store.resume_producer();
    let node_results = InMemoryNodeResultStore::new();
    let workflow_versions = InMemoryWorkflowVersionStore::new();
    let workflow_store = InMemoryWorkflowStore::new_with_versions(&workflow_versions);

    tracing::info!(
        backend = "memory",
        "execution-stores: in-memory adapters wired"
    );
    Ok(ExecutionStoreBundle {
        workflow_store: Arc::new(workflow_store),
        workflow_version_store: Arc::new(workflow_versions),
        execution_store: Arc::new(exec_store),
        node_result_store: Arc::new(node_results),
        journal_reader: Arc::new(journal),
        control_queue: Arc::new(control_queue),
        trigger_dedup_inbox: Some(Arc::new(trigger_dedup_inbox)),
        resume_token_store: Arc::new(resume_token_store),
        resume_producer: Arc::new(resume_producer),
    })
}

/// SQLite bundle — WAL + single connection + `init_schema` (idempotent DDL).
///
/// Single `max_connections(1)` serialises all writes: `BEGIN IMMEDIATE` CAS +
/// claim-fencing in the store are only correct when one writer owns the WAL lock.
/// `busy_timeout(5s)` prevents instant `SQLITE_BUSY` if a CLI probe briefly holds
/// the write lock. This file is NOT shareable across processes — for multi-process
/// or multi-host deployments operators must use `API_EXECUTION_BACKEND=postgres`.
async fn build_sqlite_execution_stores(
    api_config: &ApiConfig,
) -> Result<ExecutionStoreBundle, TransportInitError> {
    use nebula_storage::InMemoryNodeResultStore;
    use nebula_storage::sqlite::{
        SqliteControlQueue, SqliteExecutionStore, SqliteJournalReader, SqliteResumeProducer,
        SqliteResumeTokenStore, SqliteTriggerDedupInbox, SqliteWorkflowStore,
        SqliteWorkflowVersionStore, init_schema,
    };
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    };

    let db_path = &api_config.execution.db_path;
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .map_err(|err| {
            TransportInitError::ExecutionDatabase(format!(
                "SQLite: failed to open '{db_path}': {err}"
            ))
        })?;

    init_schema(&pool).await.map_err(|err| {
        TransportInitError::ExecutionDatabase(format!(
            "SQLite: schema init failed for '{db_path}': {err}"
        ))
    })?;

    tracing::info!(
        backend = "sqlite",
        db_path = %db_path,
        "execution-stores: SQLite schema applied"
    );
    // NodeResult and Checkpoint have no SQLite implementation — transient,
    // per-execution data that is re-derived from the authoritative execution row
    // on crash-recovery via the reclaim sweep.
    let node_results = Arc::new(InMemoryNodeResultStore::new());
    tracing::warn!(
        "node-result and checkpoint stores are in-memory (not persisted across restarts); \
         crash-recovery re-executes affected nodes via the reclaim sweep — \
         authoritative execution state is the SQLite execution row"
    );
    Ok(ExecutionStoreBundle {
        workflow_store: Arc::new(SqliteWorkflowStore::new(pool.clone())),
        workflow_version_store: Arc::new(SqliteWorkflowVersionStore::new(pool.clone())),
        execution_store: Arc::new(SqliteExecutionStore::new(pool.clone())),
        node_result_store: node_results,
        journal_reader: Arc::new(SqliteJournalReader::new(pool.clone())),
        control_queue: Arc::new(SqliteControlQueue::new(pool.clone())),
        // Durable backends wire the storage-backed TriggerDedupInbox so
        // `WebhookIngressTransport::prepare_state` can install `with_durable_dispatch`.
        // Without this `Some`, the `if let Some(dedup)` guard in prepare_state is never
        // taken and webhook rows are spawned without the durable dedup fence — exactly
        // backwards for a durable backend.
        trigger_dedup_inbox: Some(Arc::new(SqliteTriggerDedupInbox::new(pool.clone()))),
        // Same pool as the execution store: tokens minted by TransitionBatch must be
        // readable by the POST /resume handler's consume call on the same pool.
        resume_token_store: Arc::new(SqliteResumeTokenStore::new(pool.clone())),
        // Same pool again: the producer's token DELETE and control-queue INSERT
        // must commit in one transaction against the execution-store backend.
        resume_producer: Arc::new(SqliteResumeProducer::new(pool)),
    })
}

/// Postgres execution bundle — compiled only with `--features postgres`.
/// The `#[cfg(not(...))]` twin always fails closed.
#[cfg(feature = "postgres")]
async fn build_pg_execution_stores(
    _api_config: &ApiConfig,
) -> Result<ExecutionStoreBundle, TransportInitError> {
    use nebula_storage::InMemoryNodeResultStore;
    use nebula_storage::postgres::{
        PgControlQueue, PgExecutionStore, PgJournalReader, PgResumeProducer, PgResumeTokenStore,
        PgTriggerDedupInbox, PgWorkflowStore, PgWorkflowVersionStore,
        init_schema as pg_init_schema,
    };
    use sqlx::postgres::PgPoolOptions;

    let url = std::env::var("DATABASE_URL").map_err(|_| {
        TransportInitError::ExecutionBackendUnavailable {
            requested: "postgres",
            requirement: "DATABASE_URL must be set when API_EXECUTION_BACKEND=postgres",
        }
    })?;

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .map_err(|err| {
            TransportInitError::ExecutionDatabase(format!(
                "Postgres: failed to connect to DATABASE_URL for execution stores: {err}"
            ))
        })?;

    pg_init_schema(&pool).await.map_err(|err| {
        TransportInitError::ExecutionDatabase(format!(
            "Postgres: execution-store schema init failed: {err}"
        ))
    })?;

    tracing::info!(
        backend = "postgres",
        "execution-stores: Postgres schema applied"
    );
    tracing::warn!(
        "node-result and checkpoint stores are in-memory (not persisted across restarts); \
         crash-recovery re-executes affected nodes via the reclaim sweep — \
         authoritative execution state is the Postgres execution row"
    );
    Ok(ExecutionStoreBundle {
        workflow_store: Arc::new(PgWorkflowStore::new(pool.clone())),
        workflow_version_store: Arc::new(PgWorkflowVersionStore::new(pool.clone())),
        execution_store: Arc::new(PgExecutionStore::new(pool.clone())),
        node_result_store: Arc::new(InMemoryNodeResultStore::new()),
        journal_reader: Arc::new(PgJournalReader::new(pool.clone())),
        control_queue: Arc::new(PgControlQueue::new(pool.clone())),
        // Same rationale as the SQLite arm: durable dispatch in
        // `WebhookIngressTransport::prepare_state` is only installed when `Some`.
        trigger_dedup_inbox: Some(Arc::new(PgTriggerDedupInbox::new(pool.clone()))),
        // Same pool as the execution store — see SQLite arm for rationale.
        resume_token_store: Arc::new(PgResumeTokenStore::new(pool.clone())),
        // Same pool again: the producer's DELETE + control-queue INSERT commit
        // in one transaction against the execution-store backend.
        resume_producer: Arc::new(PgResumeProducer::new(pool)),
    })
}

/// Fail-closed twin: `API_EXECUTION_BACKEND=postgres` without the `postgres` feature.
#[cfg(not(feature = "postgres"))]
async fn build_pg_execution_stores(
    _api_config: &ApiConfig,
) -> Result<ExecutionStoreBundle, TransportInitError> {
    Err(TransportInitError::ExecutionBackendUnavailable {
        requested: "postgres",
        requirement: "build with `nebula-api/postgres` cargo feature to link sqlx + Pg execution stores",
    })
}

fn warn_execution_memory_outside_dev() {
    let env_mode = std::env::var("NEBULA_ENV").unwrap_or_default();
    let is_dev = matches!(env_mode.as_str(), "development" | "dev" | "local");
    if !is_dev {
        tracing::warn!(
            backend = "memory",
            nebula_env = %env_mode,
            component = "execution-stores",
            "execution-stores: in-memory adapters selected — execution state is lost \
             on restart and cannot be shared across processes; \
             set API_EXECUTION_BACKEND=sqlite (single-process durable) or \
             API_EXECUTION_BACKEND=postgres (multi-process) for production"
        );
    }
}

/// Start a server binary for a selected transport profile.
///
/// The caller passes in the [`TelemetryGuard`] returned from `init_api_telemetry`; the
/// runtime attaches the OTLP metrics pipeline against the shared [`MetricsRegistry`] once
/// `AppState` is built and holds the guard until the transport returns so spans and metric
/// batches are flushed deterministically on shutdown.
pub(crate) async fn run_transport<T: ServerTransport>(
    transport: T,
    telemetry_guard: TelemetryGuard,
) -> Result<(), ServerRunError> {
    ServerRuntime::new()
        .run_transport(transport, telemetry_guard)
        .await
}

/// Transport runtime orchestrator for binary composition roots.
pub(crate) struct ServerRuntime;

impl ServerRuntime {
    /// Create a new runtime.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Run a selected transport with this runtime.
    pub(crate) async fn run_transport<T: ServerTransport>(
        &self,
        transport: T,
        mut telemetry_guard: TelemetryGuard,
    ) -> Result<(), ServerRunError> {
        let mut api_config = ApiConfig::from_env()?;
        let metrics_registry = Arc::new(MetricsRegistry::new());
        // Attach the OTLP metrics pipeline against the same registry the API will publish
        // through. The guard owns the pipeline so it shuts down with the trace exporter when
        // `axum::serve` returns. A `None` endpoint silently no-ops, matching the trace path.
        telemetry_guard
            .attach_metrics_exporter(Arc::clone(&metrics_registry))
            .map_err(ServerRunError::MetricsExporter)?;
        // Build the execution-store bundle inside the async context so the SQLite and
        // Postgres paths can `await` pool construction.
        let execution_bundle = build_execution_stores(&api_config).await?;
        let mut state =
            default_state(&api_config, Arc::clone(&metrics_registry), execution_bundle)?;
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
        // Compose concrete credential adapters in the first-party app. The
        // secure-store build spawns the lease reaper, so it must run inside
        // this Tokio context. Key policy is shared with Plane-A identity.
        let key_provider = resolve_first_party_key_provider()
            .map_err(|error| TransportInitError::CredentialServiceInit(error.to_string()))?;
        let credential_runtime =
            compose_first_party_runtime(Arc::clone(&key_provider), Arc::clone(&metrics_registry))
                .await
                .map_err(|error| TransportInitError::CredentialServiceInit(error.to_string()))?;
        let credential_service = Arc::clone(&credential_runtime.service);
        // The webhook execution resolver and authenticated command controller
        // share the same service instance. Only the resolver retains direct
        // execution-plane access; AppState receives the API-owned gateway.
        let webhook_secret_resolver = Arc::new(
            crate::webhook_credential_resolver::CredentialBackedWebhookSecretResolver::new(
                Arc::clone(&credential_service),
            ),
        );
        let credential_authority: Arc<dyn CredentialTenantAuthority> =
            Arc::new(ServerCredentialAuthority::new(
                state.membership_store.clone(),
                state.workspace_resolver.clone(),
            ));
        let credential_controller = Arc::new(CredentialController::new(
            Arc::clone(&credential_service),
            credential_authority,
        ));
        let credential_gateway = Arc::new(ServerCredentialGateway::new(credential_controller));
        state = state
            .with_credential_schema(Arc::clone(&credential_runtime.catalog))
            .with_credential_gateway(credential_gateway)
            .with_webhook_secret_resolver(webhook_secret_resolver);
        // Build ONE shared `Arc<dyn EmailPort>` and pass the same Arc
        // to both `AppState::email_port` and the selected auth backend.
        // `API_SMTP_HOST` unset → dev `EchoSink` (unchanged local-first
        // default); set → production `SmtpEmailPort` (fails CLOSED on
        // malformed config per the policy on `ApiConfig::smtp`).
        // Forward-compat non-auth email consumers (org invitations,
        // billing notices) read from `state.email_port` and work
        // uniformly regardless of which transport is wired.
        let email_port = build_email_port(&api_config)?;
        // Transfer the only owned OAuth credential set into the runtime.
        // `ApiConfig` remains available to router construction, but no
        // duplicate `SecretString` allocation survives composition.
        let oauth_config = std::mem::take(&mut api_config.auth.oauth);
        let auth_backend = build_auth_backend(
            api_config.auth.backend.clone(),
            oauth_config,
            Arc::clone(&email_port),
            Some(Arc::clone(&metrics_registry)),
            key_provider,
        )
        .await?;
        state = state
            .with_auth_backend(auth_backend)
            .with_email_port(email_port);
        let app = transport.build_router(state, &api_config)?;

        tracing::info!(transport = transport.name(), %bind_address, "starting transport");
        let serve_result = app::serve(app, bind_address).await;
        // Keep credential background ownership (notably the reclaim sweep)
        // alive for the entire serving lifecycle, then abort it by Drop.
        drop(credential_runtime);
        serve_result?;
        Ok(())
    }
}

/// Build default server `AppState` from a pre-built execution-store bundle.
///
/// The execution / workflow / control-queue handles come from `execution_bundle`,
/// which was constructed asynchronously by [`build_execution_stores`] before
/// this function is called (so the SQLite and Postgres paths can `await` pool
/// construction). This function is sync: it only wires the remaining
/// deployment-specific ports (credential-schema, api keys, OAuth validation,
/// trigger-store) on top of the already-constructed bundle.
///
/// The idempotency store is **not** attached here — it is wired
/// asynchronously by [`ServerRuntime::run_transport`] so the PG-backed
/// path can `await` the sqlx pool construction. The Plane-A auth
/// backend follows the same pattern: [`build_auth_backend`] runs in
/// the async context (so the PG arm can `await` the sqlx pool) and
/// `default_state` no longer wires an unconditional in-memory backend
/// — the conditional builder owns the slot now.
///
/// `AppState::in_memory` is NOT called here: the bundle already holds the
/// correct store handles (including the shared-core in-memory wiring for the
/// Memory backend), and calling `in_memory` would build a second independent
/// store set that the bundle's stores are not connected to.
pub(crate) fn default_state(
    api_config: &ApiConfig,
    metrics_registry: Arc<MetricsRegistry>,
    execution_bundle: ExecutionStoreBundle,
) -> Result<AppState, TransportInitError> {
    // Plane-A identity backend is wired asynchronously by
    // [`build_auth_backend`] inside [`ServerRuntime::run_transport`]
    // so the PG-backed arm can `await` the sqlx pool. The selector
    // (`AuthBackendKind::Memory` vs `Postgres`) is honored there with
    // the same fail-closed contract `build_idempotency_store` uses.

    // NOTE: `membership_store` is intentionally LEFT UNWIRED (`None`) in
    // the default local-first composition.
    //
    // A `MembershipStore` is required by RBAC on every org/workspace route
    // and by the credential command authority. With this default `AuthBackend`
    // empty (no users registered — `register_user` mints a *random*
    // `UserId`), no principal could authenticate as any auto-seeded
    // bootstrap owner, so a seeded store would deadlock EVERY
    // org/workspace route with a 404 (a deployment-level §4.5 false
    // capability: the spec would advertise org member endpoints no real
    // caller can reach). Auto-seeding a fixed admin identity would also
    // be a hardcoded-credential / privileged-by-default surface (canon
    // §12.5) — both are strictly worse than honest degradation.
    //
    // With `membership_store == None`, every tenant route fails closed with
    // **503** before its handler; direct credential-gateway use also returns
    // authority-unavailable. This matches `me/*` when `auth_backend` is absent and
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

    // Validate Plane-A OAuth providers at boot per ADR-0085
    // REQ-compose-001 Invariant 1. Empty providers map is
    // a no-op; any declared provider triggers credential and `public_url`
    // validation. Fixed provider endpoint policy is constructed later by the
    // OAuth runtime. Fails closed by mapping the typed validation error to
    // `TransportInitError::OAuthProviderConfigInvalid`.
    api_config
        .auth
        .oauth
        .validate_at_load(&api_config.public_url, !cfg!(debug_assertions))
        .map_err(|e| TransportInitError::OAuthProviderConfigInvalid {
            provider: e.provider,
            reason: e.reason,
        })?;

    // Wire the trigger config store (ADR-0096 READ path). The undecorated
    // `InMemoryTriggerStore` is the correct local-first backing —
    // `TriggerStoreSpecLookup` applies `ScopedTriggerStore` per call so
    // tenant isolation is structural. The same Arc is shared between the
    // AppState trigger-store slot and the spec-lookup so they see the same
    // rows in tests and dev.
    let trigger_store = Arc::new(nebula_storage::inmem::InMemoryTriggerStore::new());
    let trigger_spec_lookup = Arc::new(
        nebula_api::transport::webhook::TriggerStoreSpecLookup::new(Arc::clone(&trigger_store) as _),
    );

    // Destructure the bundle so each handle is passed into `AppState::new`
    // positionally, matching its parameter order. The `trigger_dedup_inbox`
    // is wired via `with_trigger_dedup_inbox` only when the bundle provides
    // one (Memory backend); durable backends leave the slot empty (the engine
    // uses the storage-level IdempotencyGuard instead).
    let ExecutionStoreBundle {
        workflow_store,
        workflow_version_store,
        execution_store,
        node_result_store,
        journal_reader,
        control_queue,
        trigger_dedup_inbox,
        resume_token_store,
        resume_producer,
    } = execution_bundle;

    let mut state = AppState::new(
        workflow_store,
        workflow_version_store,
        execution_store,
        node_result_store,
        journal_reader,
        control_queue,
        api_config.jwt_secret.clone(),
    )
    .with_api_keys(api_config.api_keys.clone())
    .with_metrics_registry(metrics_registry)
    // Public URL is required for Plane-A OAuth `redirect_uri`
    // derivation per ADR-0085 D-3. Boot-time validation above rejects
    // empty or relative values when
    // `auth.oauth.providers` is non-empty.
    .with_public_url(api_config.public_url.clone())
    .with_trigger_store(trigger_store)
    .with_webhook_spec_lookup(trigger_spec_lookup);

    if let Some(inbox) = trigger_dedup_inbox {
        state = state.with_trigger_dedup_inbox(inbox);
    }

    // W-S3d: wire the resume-token store and rate-limiter components.
    //
    // The store is taken from `execution_bundle` — constructed from the same backend
    // pool (SQLite/Postgres) or shared in-memory state (Memory) as the execution store.
    // This ensures tokens minted by the engine's TransitionBatch on the durable pool
    // are readable by this process's POST /resume handler.  A standalone in-memory store
    // here would be a silent breakage: the engine writes to the pool, the handler reads
    // from an empty process-local store, and every valid resume 404s.
    //
    // `SystemClock` is the production clock; tests inject a `MockClock` via
    // `with_resume_handler_components`.
    let resume_components =
        nebula_api::transport::webhook::ResumeHandlerComponents::with_defaults();
    state = state
        .with_resume_token_store(resume_token_store)
        .with_resume_producer(resume_producer)
        .with_resume_handler_components(resume_components);

    Ok(state)
}

/// Build the shared `Arc<dyn EmailPort>` from `api_config.smtp`.
///
/// `None` keeps the dev `EchoSink` (`API_SMTP_HOST` unset). `Some`
/// instantiates an [`SmtpEmailPort`] and fails CLOSED on construction
/// errors (invalid `from_address`, malformed TLS parameters) so an
/// operator who set `API_SMTP_HOST` never silently boots with the
/// in-process echo sink. `SmtpTlsMode::None` emits a startup
/// `tracing::warn!` because plaintext SMTP is a dev-only posture.
pub(crate) fn build_email_port(
    api_config: &ApiConfig,
) -> Result<Arc<dyn EmailPort>, TransportInitError> {
    if let Some(smtp_cfg) = api_config.smtp.as_ref() {
        if matches!(smtp_cfg.tls, SmtpTlsMode::None) {
            tracing::warn!(
                host = %smtp_cfg.host,
                port = smtp_cfg.port,
                "smtp: TLS disabled (API_SMTP_TLS_MODE=none) — credentials and mail bodies travel in plaintext; this is acceptable only for in-cluster dev"
            );
        }
        let port = SmtpEmailPort::new(smtp_cfg)
            .map_err(|source| TransportInitError::SmtpEmailPortInit { source })?;
        tracing::info!(
            host = %smtp_cfg.host,
            port = smtp_cfg.port,
            tls = ?smtp_cfg.tls,
            authenticated = smtp_cfg.username.is_some(),
            "email: SMTP transport wired"
        );
        Ok(Arc::new(port))
    } else {
        tracing::info!("email: EchoSink (dev) wired — set API_SMTP_HOST to enable SMTP");
        Ok(Arc::new(EchoSink::default()))
    }
}

/// Construct the Plane-A authentication backend from an owned OAuth config.
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
pub(crate) async fn build_auth_backend(
    backend_kind: AuthBackendKind,
    oauth_config: OAuthProvidersConfig,
    email_port: Arc<dyn EmailPort>,
    metrics_registry: Option<Arc<MetricsRegistry>>,
    key_provider: Arc<dyn KeyProvider>,
) -> Result<Arc<dyn AuthBackend>, TransportInitError> {
    let oauth_runtime = OAuthIdentityRuntime::from_config(oauth_config)
        .map_err(|source| TransportInitError::OAuthRuntimeInit { source })?
        .map(Arc::new);
    match backend_kind {
        AuthBackendKind::Memory => {
            let backend = InMemoryAuthBackend::new()
                .with_email_port(email_port)
                .with_metrics(metrics_registry);
            let backend = match oauth_runtime {
                Some(runtime) => backend.with_oauth_runtime(runtime),
                None => backend,
            };
            Ok(Arc::new(backend))
        },
        AuthBackendKind::Postgres => {
            build_pg_auth_backend(email_port, metrics_registry, oauth_runtime, key_provider).await
        },
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
pub(crate) async fn build_idempotency_store(
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
    metrics_registry: Option<Arc<MetricsRegistry>>,
    oauth_runtime: Option<Arc<OAuthIdentityRuntime>>,
    key_provider: Arc<dyn KeyProvider>,
) -> Result<Arc<dyn AuthBackend>, TransportInitError> {
    use nebula_api::domain::auth::backend::PgAuthBackend;
    use nebula_storage::{identity_secret::IdentitySecretCodec, pg::PgIdentitySecretMigrator};
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
    let identity_secrets = Arc::new(IdentitySecretCodec::new(key_provider).map_err(|error| {
        TransportInitError::ContextFactory(format!(
            "auth: identity secret codec initialization failed: {error}"
        ))
    })?);
    let _migration_report =
        PgIdentitySecretMigrator::new(pool.clone(), Arc::clone(&identity_secrets))
            .run()
            .await
            .map_err(|error| {
                TransportInitError::ContextFactory(format!(
                    "auth: identity secret migration failed: {error}"
                ))
            })?;
    let backend = PgAuthBackend::new(pool, email_port, metrics_registry, identity_secrets);
    let backend = match oauth_runtime {
        Some(runtime) => backend.with_oauth_runtime(runtime),
        None => backend,
    };
    let backend: Arc<dyn AuthBackend> = Arc::new(backend);
    Ok(backend)
}

#[cfg(not(feature = "postgres"))]
async fn build_pg_auth_backend(
    _email_port: Arc<dyn EmailPort>,
    _metrics_registry: Option<Arc<MetricsRegistry>>,
    _oauth_runtime: Option<Arc<OAuthIdentityRuntime>>,
    _key_provider: Arc<dyn KeyProvider>,
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

    use super::{ServerRunError, build_execution_stores, parse_bind_address, resolve_bind_address};

    /// Red→green proof that the SQLite backend wires `trigger_dedup_inbox: Some(...)`.
    ///
    /// Without the fix this test fails because `build_sqlite_execution_stores` returned
    /// `trigger_dedup_inbox: None`, which causes `WebhookIngressTransport::prepare_state`
    /// to skip `with_durable_dispatch` — breaking prod webhook spawning on a durable backend.
    #[tokio::test]
    async fn sqlite_execution_bundle_wires_trigger_dedup_inbox() {
        use nebula_api::config::{ExecutionBackendKind, ExecutionStoreConfig};

        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let (_, db_path) = tmp.keep().expect("persist tempfile");

        let mut cfg = nebula_api::ApiConfig::for_test();
        cfg.execution = ExecutionStoreConfig {
            backend: ExecutionBackendKind::Sqlite,
            db_path: db_path.to_string_lossy().into_owned(),
        };

        let bundle = build_execution_stores(&cfg)
            .await
            .expect("sqlite bundle must build");

        // The inbox MUST be Some so WebhookIngressTransport::prepare_state
        // installs with_durable_dispatch. A None here silently disables durable
        // webhook dispatch for operators running API_EXECUTION_BACKEND=sqlite.
        assert!(
            bundle.trigger_dedup_inbox.is_some(),
            "sqlite bundle must provide a TriggerDedupInbox for durable webhook dispatch"
        );

        // Clean up temp db file.
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
        let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    }

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
