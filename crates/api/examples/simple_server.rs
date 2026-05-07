//! Minimal Nebula API server.
//!
//! Set `API_JWT_SECRET` (32+ bytes) before running, or set
//! `NEBULA_ENV=development` to let the loader generate an ephemeral
//! per-process secret. Tokens signed with an ephemeral secret are
//! invalidated on restart.
//!
//! ## Idempotency backends (M3.4 / ADR-0048)
//!
//! - `API_IDEMPOTENCY_BACKEND=memory` (default) — process-local cache.
//!   No restart durability, no cross-runner sharing. Fine for dev.
//! - `API_IDEMPOTENCY_BACKEND=postgres` — durable PG-backed dedup.
//!   Requires `--features postgres` at build time **and** `DATABASE_URL`
//!   at runtime. Without `--features postgres`, this path returns a
//!   typed startup error (fail-closed per ADR-0048; no silent fallback).

use std::{sync::Arc, time::Duration};

use nebula_api::{
    ApiConfig, AppState, app,
    config::IdempotencyBackend,
    middleware::{IdempotencyStore, InMemoryIdempotencyStore},
};
use nebula_storage::{
    InMemoryExecutionRepo, InMemoryWorkflowRepo, repos::InMemoryControlQueueRepo,
};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    // DEMO ONLY — does not honor start / cancel (canon §12.2, ADR-0008 §4).
    //
    // ADR-0008 A2 + A3 landed `nebula_engine::EngineControlDispatch` with
    // full Start / Resume / Restart / Cancel / Terminate dispatch wired into
    // the engine, so a production composition root can now drop this file's
    // repos into an `AppState` and spawn a real `ControlConsumer` bound to a
    // `WorkflowEngine`. This `simple_server.rs` intentionally does not pull
    // in the full engine stack (plugin registry, action runtime, sandbox,
    // metrics, credential / resource managers) — the dedicated `apps/server`
    // composition root is still tracked as a separate follow-up. Until then
    // this example stays DEMO ONLY: `POST /executions` persists the row and
    // enqueues `Start`, but with no consumer wired here the row never
    // transitions to `Running`. `crates/api/tests/knife.rs` exercises the
    // full producer → consumer → engine path end-to-end for both Start
    // (step 3) and Cancel (step 5) regression coverage.
    // `InMemoryControlQueueRepo` also does not persist across restarts; a
    // real deployment additionally requires a Postgres-backed repo.
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());
    let api_config = ApiConfig::from_env()?;

    // Wire the idempotency-store backend per ADR-0048. The cancellation
    // token coordinates graceful shutdown across axum and the optional
    // PG sweep task spawned below.
    let shutdown_token = CancellationToken::new();
    let (idempotency_store, sweep_handle) =
        build_idempotency(&api_config, shutdown_token.clone()).await?;

    // Wire api_keys from ApiConfig so X-API-Key auth is honoured.
    let state = AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    )
    .with_api_keys(api_config.api_keys.clone())
    .with_idempotency_store(idempotency_store);
    let bind_address = api_config.bind_address;
    let app = app::build_app(state, &api_config);

    // Spawn an OS-signal handler that flips the shutdown token. axum's
    // `with_graceful_shutdown` and the sweep task both observe it.
    let signal_token = shutdown_token.clone();
    tokio::spawn(async move {
        wait_for_signal().await;
        tracing::info!("shutdown signal received — flipping shutdown token");
        signal_token.cancel();
    });

    let serve_token = shutdown_token.clone();
    let serve_result = app::serve_with_shutdown(app, bind_address, async move {
        serve_token.cancelled().await;
    })
    .await;

    // Make sure the sweep task gets a chance to exit even if axum returns
    // first (e.g. bind error). On a clean shutdown the OS-signal task has
    // already flipped the token; a defensive flip is a no-op.
    shutdown_token.cancel();
    if let Some(handle) = sweep_handle
        && let Err(err) = handle.await
    {
        tracing::warn!(error = %err, "idempotency sweep task panicked");
    }
    serve_result?;
    Ok(())
}

async fn wait_for_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

/// Build the idempotency store + (optionally) spawn the PG sweep task.
///
/// Returns the store handle and a `JoinHandle` for the sweep task; the
/// handle is `None` for the in-memory backend (moka evicts on its own).
/// The `shutdown_token` is observed by the sweep task so SIGTERM
/// terminates it within one tick of cancellation.
async fn build_idempotency(
    api_config: &ApiConfig,
    shutdown_token: CancellationToken,
) -> Result<
    (
        Arc<dyn IdempotencyStore>,
        Option<tokio::task::JoinHandle<()>>,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    match api_config.idempotency.backend {
        IdempotencyBackend::Memory => {
            warn_memory_outside_dev();
            let store = Arc::new(InMemoryIdempotencyStore::with_ttl_and_capacity(
                Duration::from_secs(api_config.idempotency.ttl_secs),
                api_config.idempotency.max_entries,
            ));
            Ok((store, None))
        },
        IdempotencyBackend::Postgres => build_pg_backend(api_config, shutdown_token).await,
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

#[cfg(feature = "postgres")]
async fn build_pg_backend(
    api_config: &ApiConfig,
    shutdown_token: CancellationToken,
) -> Result<
    (
        Arc<dyn IdempotencyStore>,
        Option<tokio::task::JoinHandle<()>>,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    use nebula_api::middleware::idempotency::StorageBackedIdempotencyStore;
    use nebula_storage::{pg::PgIdempotencyStore, repos::IdempotencyStoreRepo};
    use sqlx::postgres::PgPoolOptions;

    let url = std::env::var("DATABASE_URL").map_err(|_| {
        "API_IDEMPOTENCY_BACKEND=postgres requires DATABASE_URL — set it or fall back to \
         API_IDEMPOTENCY_BACKEND=memory (per ADR-0048 fail-closed contract)"
    })?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await?;
    tracing::info!(backend = "postgres", "idempotency: PG-backed store wired");

    if api_config.idempotency.sweep_interval_secs > 0
        && api_config.idempotency.sweep_interval_secs < 60
    {
        tracing::warn!(
            sweep_interval_secs = api_config.idempotency.sweep_interval_secs,
            "idempotency: sweep interval < 60s; consider raising it to avoid hot-loop sweeps"
        );
    }

    let pg_repo = Arc::new(PgIdempotencyStore::new(pool));
    let sweep_repo = Arc::clone(&pg_repo);
    let interval_secs = api_config.idempotency.sweep_interval_secs;
    let sweep_handle = if interval_secs > 0 {
        let sweep_token = shutdown_token.clone();
        Some(tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(interval_secs));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    () = sweep_token.cancelled() => {
                        tracing::info!("idempotency sweep task: shutdown observed");
                        break;
                    }
                    _ = tick.tick() => {
                        match sweep_repo.evict_expired().await {
                            Ok(rows) => tracing::info!(rows_evicted = rows, "idempotency: sweep cycle"),
                            Err(err) => tracing::warn!(error = %err, "idempotency: sweep failed"),
                        }
                    }
                }
            }
        }))
    } else {
        None
    };

    let store: Arc<dyn IdempotencyStore> = Arc::new(StorageBackedIdempotencyStore::new(
        pg_repo,
        Duration::from_secs(api_config.idempotency.ttl_secs),
    ));
    Ok((store, sweep_handle))
}

#[cfg(not(feature = "postgres"))]
async fn build_pg_backend(
    _api_config: &ApiConfig,
    _shutdown_token: CancellationToken,
) -> Result<
    (
        Arc<dyn IdempotencyStore>,
        Option<tokio::task::JoinHandle<()>>,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    Err(
        "API_IDEMPOTENCY_BACKEND=postgres requires the `postgres` feature: \
         rebuild with `cargo run --example simple_server --features postgres` \
         (per ADR-0048 fail-closed contract)"
            .into(),
    )
}
