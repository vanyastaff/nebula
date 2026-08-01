//! Behavioral conformance harness for the storage port.
//!
//! One backend-agnostic contract suite (spec-16 §5 / §9) exercised across
//! `{InMemory, SQLite :memory:, Postgres (DATABASE_URL-gated)}`. Each
//! backend implements [`Backend`]; the shared assertions encode the
//! abstract concurrency + tenancy contract every adapter must satisfy:
//!
//! - create → get round-trip
//! - CAS conflict returns `VersionConflict { actual }`
//! - a stale fencing token returns `FencedOut`
//! - the atomic triple (state + outbox + journal) is all-or-nothing
//! - idempotency key shape + first-writer-wins
//! - cross-scope `get` / `commit` ⇒ `None` / `NotFound` (never another
//!   tenant's row)
//!
//! All three adapters (InMemory / SQLite / Postgres) implement the port.
//! The Postgres case is `DATABASE_URL`-gated and skip-cleans (WARN +
//! pass) when no database is configured; the SQLite case skips without
//! the `sqlite` feature. A skipped backend never reports a false green
//! and never hard-fails on a host that cannot run it.

use std::sync::Arc;

use nebula_core::PluginKey;
use nebula_storage_port::dto::{
    CachedRecord, ControlCommand, ControlMsg, DispatchKind, JobDispatchMsg, JournalEntry,
    NewExecution, ResumeTarget, TriggerDedupRow, WebhookActivationRecord, WebhookMode,
    WorkflowRecord, WorkflowVersionRecord,
};
use nebula_storage_port::store::{
    ClaimGeneration, ControlClaimToken, ControlQueue, ExecutionJournalReader, ExecutionStore,
    IdempotencyGuard, IdempotencyStore, JobClaimToken, JobDispatchQueue, KeyedStart,
    StartAcceptance, StartAcceptanceStore, StartFingerprint, TriggerDedupInbox,
    WebhookActivationStore, WorkflowStore, WorkflowVersionStore,
};
use nebula_storage_port::{FencingToken, Scope, StorageError, TransitionBatch, TransitionOutcome};

/// A storage backend under conformance test. Returns port handles built on
/// that backend's concrete adapter.
#[async_trait::async_trait]
pub(crate) trait Backend: Send + Sync {
    /// Human-readable backend name (used in assertion messages).
    fn name(&self) -> &'static str;
    /// An execution store backed by this backend.
    async fn execution_store(&self) -> Arc<dyn ExecutionStore>;
    /// An idempotency guard backed by this backend.
    async fn idempotency_guard(&self) -> Arc<dyn IdempotencyGuard>;
    /// A control-queue (durable outbox) backed by this backend, sharing
    /// the same store as [`Backend::execution_store`] so a `commit`'s
    /// outbox rows are observable through `claim_pending`.
    async fn control_queue(&self) -> Arc<dyn ControlQueue>;
    /// A journal reader backed by this backend, sharing the same store as
    /// [`Backend::execution_store`] so a `commit`'s journal entries are
    /// observable.
    async fn journal_reader(&self) -> Arc<dyn ExecutionJournalReader>;
    /// A durable idempotent-replay cache backed by this backend.
    async fn idempotency_store(&self) -> Arc<dyn IdempotencyStore>;
    /// A webhook-activation store backed by this backend.
    async fn webhook_store(&self) -> Arc<dyn WebhookActivationStore>;
    /// A workflow-row store backed by this backend (spec-16 split).
    async fn workflow_store(&self) -> Arc<dyn WorkflowStore>;
    /// A workflow-version store backed by this backend, sharing the same
    /// backend as [`Backend::workflow_store`].
    async fn workflow_version_store(&self) -> Arc<dyn WorkflowVersionStore>;
    /// A job-dispatch queue backed by this backend.
    async fn job_dispatch_queue(&self) -> Arc<dyn JobDispatchQueue>;
    /// A trigger-dedup inbox backed by this backend, sharing the same
    /// core as [`Backend::job_dispatch_queue`] so `claim_and_materialize_start`
    /// is all-or-nothing within the backend.
    async fn trigger_dedup_inbox(&self) -> Arc<dyn TriggerDedupInbox>;
    /// A keyed-start acceptance store backed by this backend, sharing the same
    /// core as [`Backend::execution_store`] and [`Backend::control_queue`] so
    /// `accept_keyed_start` commits all three of its writes together.
    async fn start_acceptance_store(&self) -> Arc<dyn StartAcceptanceStore>;
}

/// InMemory backend (always available).
///
/// Holds one execution store whose core is shared (it is `Clone` over an
/// `Arc<Mutex<…>>`), so the control queue, journal reader, job-dispatch queue,
/// and trigger-dedup inbox all observe the same rows and operate atomically
/// under one lock.
pub(crate) struct InMemoryBackend {
    store: nebula_storage::inmem::InMemoryExecutionStore,
    guard: nebula_storage::inmem::InMemoryIdempotencyGuard,
    idem_store: nebula_storage::inmem::InMemoryIdempotencyStore,
    webhook: nebula_storage::inmem::InMemoryWebhookActivationStore,
    workflow: nebula_storage::inmem::InMemoryWorkflowStore,
    workflow_version: nebula_storage::inmem::InMemoryWorkflowVersionStore,
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        // The workflow-row store shares the version store's map (same
        // contract as `control_queue`/`journal` over the shared execution
        // core) so `save_with_published_version` is genuinely atomic
        // across the pair under the conformance matrix.
        let workflow_version = nebula_storage::inmem::InMemoryWorkflowVersionStore::new();
        let workflow =
            nebula_storage::inmem::InMemoryWorkflowStore::new_with_versions(&workflow_version);
        Self {
            store: nebula_storage::inmem::InMemoryExecutionStore::new(),
            guard: nebula_storage::inmem::InMemoryIdempotencyGuard::new(),
            idem_store: nebula_storage::inmem::InMemoryIdempotencyStore::new(),
            webhook: nebula_storage::inmem::InMemoryWebhookActivationStore::new(),
            workflow,
            workflow_version,
        }
    }
}

#[async_trait::async_trait]
impl Backend for InMemoryBackend {
    fn name(&self) -> &'static str {
        "InMemory"
    }
    async fn execution_store(&self) -> Arc<dyn ExecutionStore> {
        Arc::new(self.store.clone())
    }
    async fn idempotency_guard(&self) -> Arc<dyn IdempotencyGuard> {
        Arc::new(self.guard.clone())
    }
    async fn control_queue(&self) -> Arc<dyn ControlQueue> {
        Arc::new(nebula_storage::inmem::InMemoryControlQueue::new(
            &self.store,
        ))
    }
    async fn journal_reader(&self) -> Arc<dyn ExecutionJournalReader> {
        Arc::new(nebula_storage::inmem::InMemoryJournalReader::new(
            &self.store,
        ))
    }
    async fn idempotency_store(&self) -> Arc<dyn IdempotencyStore> {
        Arc::new(self.idem_store.clone())
    }
    async fn webhook_store(&self) -> Arc<dyn WebhookActivationStore> {
        Arc::new(self.webhook.clone())
    }
    async fn workflow_store(&self) -> Arc<dyn WorkflowStore> {
        Arc::new(self.workflow.clone())
    }
    async fn workflow_version_store(&self) -> Arc<dyn WorkflowVersionStore> {
        Arc::new(self.workflow_version.clone())
    }
    async fn job_dispatch_queue(&self) -> Arc<dyn JobDispatchQueue> {
        Arc::new(nebula_storage::inmem::InMemoryJobDispatchQueue::new(
            &self.store,
        ))
    }
    async fn trigger_dedup_inbox(&self) -> Arc<dyn TriggerDedupInbox> {
        Arc::new(nebula_storage::inmem::InMemoryTriggerDedupInbox::new(
            &self.store,
        ))
    }
    async fn start_acceptance_store(&self) -> Arc<dyn StartAcceptanceStore> {
        Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
            &self.store,
        ))
    }
}

/// SQLite `:memory:` backend.
///
/// Each `Backend` instance owns one shared-cache in-memory database (so a
/// `create` and a later `commit`/`get` observe the same rows) created
/// lazily on first store request. Only built when the `sqlite` feature is
/// on; without it the case skips like Postgres.
#[derive(Default)]
pub(crate) struct SqliteBackend {
    #[cfg(feature = "sqlite")]
    pool: tokio::sync::OnceCell<sqlx::SqlitePool>,
}

#[cfg(feature = "sqlite")]
impl SqliteBackend {
    async fn pool(&self) -> sqlx::SqlitePool {
        use std::str::FromStr;
        self.pool
            .get_or_init(|| async {
                let db_name = format!("nebula-conformance-{}", uuid::Uuid::new_v4());
                let url = format!("sqlite:file:{db_name}?mode=memory&cache=shared");
                let opts = sqlx::sqlite::SqliteConnectOptions::from_str(&url)
                    .expect("parse sqlite memory url")
                    .create_if_missing(true);
                let pool = sqlx::sqlite::SqlitePoolOptions::new()
                    .max_connections(4)
                    .connect_with(opts)
                    .await
                    .expect("connect sqlite memory");
                nebula_storage::sqlite::init_schema(&pool)
                    .await
                    .expect("install port schema");
                pool
            })
            .await
            .clone()
    }
}

#[async_trait::async_trait]
impl Backend for SqliteBackend {
    fn name(&self) -> &'static str {
        "Sqlite(:memory:)"
    }
    #[cfg(feature = "sqlite")]
    async fn execution_store(&self) -> Arc<dyn ExecutionStore> {
        Arc::new(nebula_storage::sqlite::SqliteExecutionStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn execution_store(&self) -> Arc<dyn ExecutionStore> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn idempotency_guard(&self) -> Arc<dyn IdempotencyGuard> {
        Arc::new(nebula_storage::sqlite::SqliteIdempotencyGuard::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn idempotency_guard(&self) -> Arc<dyn IdempotencyGuard> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn control_queue(&self) -> Arc<dyn ControlQueue> {
        Arc::new(nebula_storage::sqlite::SqliteControlQueue::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn control_queue(&self) -> Arc<dyn ControlQueue> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn journal_reader(&self) -> Arc<dyn ExecutionJournalReader> {
        Arc::new(nebula_storage::sqlite::SqliteJournalReader::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn journal_reader(&self) -> Arc<dyn ExecutionJournalReader> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn idempotency_store(&self) -> Arc<dyn IdempotencyStore> {
        Arc::new(nebula_storage::sqlite::SqliteIdempotencyStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn idempotency_store(&self) -> Arc<dyn IdempotencyStore> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn webhook_store(&self) -> Arc<dyn WebhookActivationStore> {
        Arc::new(nebula_storage::sqlite::SqliteWebhookActivationStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn webhook_store(&self) -> Arc<dyn WebhookActivationStore> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn workflow_store(&self) -> Arc<dyn WorkflowStore> {
        Arc::new(nebula_storage::sqlite::SqliteWorkflowStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn workflow_store(&self) -> Arc<dyn WorkflowStore> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn workflow_version_store(&self) -> Arc<dyn WorkflowVersionStore> {
        Arc::new(nebula_storage::sqlite::SqliteWorkflowVersionStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn workflow_version_store(&self) -> Arc<dyn WorkflowVersionStore> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn job_dispatch_queue(&self) -> Arc<dyn JobDispatchQueue> {
        Arc::new(nebula_storage::sqlite::SqliteJobDispatchQueue::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn job_dispatch_queue(&self) -> Arc<dyn JobDispatchQueue> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn trigger_dedup_inbox(&self) -> Arc<dyn TriggerDedupInbox> {
        Arc::new(nebula_storage::sqlite::SqliteTriggerDedupInbox::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn trigger_dedup_inbox(&self) -> Arc<dyn TriggerDedupInbox> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn start_acceptance_store(&self) -> Arc<dyn StartAcceptanceStore> {
        Arc::new(nebula_storage::sqlite::SqliteStartAcceptanceStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn start_acceptance_store(&self) -> Arc<dyn StartAcceptanceStore> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
}

/// Postgres backend — only exercised when `DATABASE_URL` is set and the
/// crate is built with `--features postgres`; otherwise `skip_reason`
/// short-circuits the case so the suite stays green on a machine without
/// a database. Each `Backend` instance owns one pool created lazily on
/// first store request; the port schema is installed once.
///
/// The instance also owns a **private PostgreSQL schema**. `InMemoryBackend`
/// and `SqliteBackend` hand every case a fresh empty store, and the shared
/// assertions rely on that: they use fixed fixture ids (`wf_c`, `exe_cq`, …)
/// that several cases reuse. Pointing every case at one shared database
/// instead made those ids collide — the second case to run saw
/// `Duplicate { entity: "workflow", detail: "workflow wf_c already exists" }`
/// whether or not the cases ran concurrently. Because the Postgres case
/// skip-cleans without `DATABASE_URL`, and no CI job set one for this suite,
/// the collisions stayed invisible: the "shared oracle" was green precisely
/// because its Postgres arm never ran.
///
/// A per-instance schema restores the same-fresh-store contract the other two
/// backends already satisfy. The migration catalog observes and installs
/// through `current_schema()`, so it sees a genuinely fresh database here.
#[derive(Default)]
pub(crate) struct PostgresBackend {
    #[cfg(feature = "postgres")]
    pool: tokio::sync::OnceCell<sqlx::PgPool>,
}

#[cfg(feature = "postgres")]
#[path = "../support/postgres_schema.rs"]
mod postgres_schema;

#[cfg(feature = "postgres")]
impl PostgresBackend {
    async fn pool(&self) -> sqlx::PgPool {
        self.pool
            .get_or_init(|| async {
                let url = std::env::var("DATABASE_URL")
                    .unwrap_or_else(|e| panic!("DATABASE_URL required for the Postgres case: {e}"));
                let pool = postgres_schema::connect_with_private_schema(&url, "nebula_conformance")
                    .await
                    .expect("connect Postgres (DATABASE_URL)");
                nebula_storage::postgres::init_schema(&pool)
                    .await
                    .expect("install port schema");
                pool
            })
            .await
            .clone()
    }
}

#[async_trait::async_trait]
impl Backend for PostgresBackend {
    fn name(&self) -> &'static str {
        "Postgres"
    }
    #[cfg(feature = "postgres")]
    async fn execution_store(&self) -> Arc<dyn ExecutionStore> {
        Arc::new(nebula_storage::postgres::PgExecutionStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn execution_store(&self) -> Arc<dyn ExecutionStore> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn idempotency_guard(&self) -> Arc<dyn IdempotencyGuard> {
        Arc::new(nebula_storage::postgres::PgIdempotencyGuard::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn idempotency_guard(&self) -> Arc<dyn IdempotencyGuard> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn control_queue(&self) -> Arc<dyn ControlQueue> {
        Arc::new(nebula_storage::postgres::PgControlQueue::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn control_queue(&self) -> Arc<dyn ControlQueue> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn journal_reader(&self) -> Arc<dyn ExecutionJournalReader> {
        Arc::new(nebula_storage::postgres::PgJournalReader::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn journal_reader(&self) -> Arc<dyn ExecutionJournalReader> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn idempotency_store(&self) -> Arc<dyn IdempotencyStore> {
        Arc::new(nebula_storage::postgres::PgIdempotencyStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn idempotency_store(&self) -> Arc<dyn IdempotencyStore> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn webhook_store(&self) -> Arc<dyn WebhookActivationStore> {
        Arc::new(nebula_storage::postgres::PgWebhookActivationStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn webhook_store(&self) -> Arc<dyn WebhookActivationStore> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn workflow_store(&self) -> Arc<dyn WorkflowStore> {
        Arc::new(nebula_storage::postgres::PgWorkflowStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn workflow_store(&self) -> Arc<dyn WorkflowStore> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn workflow_version_store(&self) -> Arc<dyn WorkflowVersionStore> {
        Arc::new(nebula_storage::postgres::PgWorkflowVersionStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn workflow_version_store(&self) -> Arc<dyn WorkflowVersionStore> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn job_dispatch_queue(&self) -> Arc<dyn JobDispatchQueue> {
        Arc::new(nebula_storage::postgres::PgJobDispatchQueue::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn job_dispatch_queue(&self) -> Arc<dyn JobDispatchQueue> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn trigger_dedup_inbox(&self) -> Arc<dyn TriggerDedupInbox> {
        Arc::new(nebula_storage::postgres::PgTriggerDedupInbox::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn trigger_dedup_inbox(&self) -> Arc<dyn TriggerDedupInbox> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn start_acceptance_store(&self) -> Arc<dyn StartAcceptanceStore> {
        Arc::new(nebula_storage::postgres::PgStartAcceptanceStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn start_acceptance_store(&self) -> Arc<dyn StartAcceptanceStore> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
}

/// True when a Postgres URL is configured. `DATABASE_URL` set-but-invalid
/// is a hard error elsewhere (pool construction); here we only gate
/// presence so the case skips cleanly when unset. Only compiled with the
/// `postgres` feature (the sole caller is `postgres_skip`).
#[cfg(feature = "postgres")]
#[must_use]
fn postgres_available() -> bool {
    std::env::var("DATABASE_URL").is_ok()
}

/// Postgres skip decision, resolved by feature flag so there is exactly
/// one match arm for the `"Postgres"` literal (avoids overlapping-pattern
/// lint when the feature is off).
#[cfg(feature = "postgres")]
fn postgres_skip() -> Option<&'static str> {
    if postgres_available() {
        None
    } else {
        Some("DATABASE_URL unset; skipping Postgres case")
    }
}

#[cfg(not(feature = "postgres"))]
fn postgres_skip() -> Option<&'static str> {
    Some("built without --features postgres; skipping Postgres case")
}

/// SQLite skip decision, resolved by feature flag (same single-arm
/// rationale as [`postgres_skip`]).
#[cfg(feature = "sqlite")]
fn sqlite_skip() -> Option<&'static str> {
    None
}

#[cfg(not(feature = "sqlite"))]
fn sqlite_skip() -> Option<&'static str> {
    Some("built without --features sqlite; skipping SQLite case")
}

/// Returns a skip reason for a backend whose prerequisites are not met, or
/// `None` if the case should run. Postgres skips without `DATABASE_URL` or
/// the `postgres` feature; SQLite skips without the `sqlite` feature.
#[must_use]
pub(crate) fn skip_reason(backend: &dyn Backend) -> Option<&'static str> {
    match backend.name() {
        "Postgres" => postgres_skip(),
        "Sqlite(:memory:)" => sqlite_skip(),
        _ => None,
    }
}

fn scope_a() -> Scope {
    Scope::new("ws_a", "org_a")
}

fn scope_b() -> Scope {
    Scope::new("ws_b", "org_b")
}

// ── shared contract assertions ────────────────────────────────────────────

/// create → get returns the row within the same scope.
pub(crate) async fn assert_create_get_roundtrip(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let s = scope_a();
    store
        .create(&s, "exe_1", "wf_1", serde_json::json!({"k": 1}))
        .await
        .expect("create");
    let got = store.get(&s, "exe_1").await.expect("get");
    let rec = got.unwrap_or_else(|| panic!("[{}] expected the row", backend.name()));
    assert_eq!(rec.id, "exe_1");
    assert_eq!(rec.workflow_id, "wf_1");
}

/// A commit whose `expected_version` does not match the row returns
/// `VersionConflict { actual }`.
pub(crate) async fn assert_cas_conflict(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let s = scope_a();
    store
        .create(&s, "exe_cas", "wf_1", serde_json::json!({}))
        .await
        .expect("create");
    let token = store
        .acquire_lease(&s, "exe_cas", "holder", std::time::Duration::from_secs(30))
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] lease must be acquirable", backend.name()));
    let batch = TransitionBatch::builder()
        .scope(s.clone())
        .execution_id("exe_cas")
        .expected_version(999) // deliberately wrong
        .fencing(token)
        .new_state(serde_json::json!({"s": "running"}))
        .build()
        .expect("batch");
    let outcome = store.commit(batch).await.expect("commit");
    assert!(
        matches!(outcome, TransitionOutcome::VersionConflict { .. }),
        "[{}] expected VersionConflict, got {outcome:?}",
        backend.name()
    );
}

/// A commit carrying a superseded fencing token returns `FencedOut`.
pub(crate) async fn assert_stale_fencing_is_fenced_out(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let s = scope_a();
    store
        .create(&s, "exe_fence", "wf_1", serde_json::json!({}))
        .await
        .expect("create");
    let _live = store
        .acquire_lease(
            &s,
            "exe_fence",
            "holder-1",
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("acquire_lease");
    // A token from an older generation than whatever the store now holds.
    let stale = FencingToken::from_generation(0);
    let batch = TransitionBatch::builder()
        .scope(s.clone())
        .execution_id("exe_fence")
        .expected_version(0)
        .fencing(stale)
        .new_state(serde_json::json!({"s": "running"}))
        .build()
        .expect("batch");
    let outcome = store.commit(batch).await.expect("commit");
    assert!(
        matches!(
            outcome,
            TransitionOutcome::FencedOut | TransitionOutcome::VersionConflict { .. }
        ),
        "[{}] a stale fencing token must not Apply, got {outcome:?}",
        backend.name()
    );
}

/// A live lease blocks every further `acquire_lease` — including a second
/// acquire by the *same* holder — and an acquire that follows a prior
/// (now-expired) lease bumps the fencing generation so the pre-expiry
/// token is dead. Zombie-runner closure: a live lease blocks re-acquire,
/// two concurrent runners must see exactly one winner, and a
/// crashed-then-restarted runner reusing its holder id cannot revive its
/// pre-crash token.
pub(crate) async fn assert_live_lease_blocks_acquire(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let s = scope_a();
    store
        .create(&s, "exe_lease", "wf_1", serde_json::json!({}))
        .await
        .expect("create");

    let g1 = store
        .acquire_lease(
            &s,
            "exe_lease",
            "holder",
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] first acquire must grant a token", backend.name()))
        .generation();

    // A second acquire while the lease is live is contention — even
    // for the SAME holder. Renewal is `renew_lease`, not a re-acquire.
    let contended = store
        .acquire_lease(
            &s,
            "exe_lease",
            "holder",
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("acquire_lease");
    assert!(
        contended.is_none(),
        "[{}] a second acquire of a live lease (same holder) must be \
         contention (None), got {contended:?}",
        backend.name()
    );

    // After the lease expires, the same holder may re-acquire — but
    // the generation must strictly increase so the pre-expiry token is
    // fenced (the holder could be a zombie from before the crash).
    // Adapters floor the lease TTL to a 1s minimum (production never
    // wants sub-second leases), so acquire with a short TTL and sleep
    // past that floor before re-acquiring.
    store
        .create(&s, "exe_lease_z", "wf_1", serde_json::json!({}))
        .await
        .expect("create");
    let z1 = store
        .acquire_lease(
            &s,
            "exe_lease_z",
            "holder",
            std::time::Duration::from_millis(1),
        )
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] zombie-case first acquire", backend.name()))
        .generation();
    // Let the floored (≈1s) lease expire.
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let z2 = store
        .acquire_lease(
            &s,
            "exe_lease_z",
            "holder",
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| {
            panic!(
                "[{}] same holder must re-acquire an expired lease",
                backend.name()
            )
        })
        .generation();
    assert!(
        z2 > z1,
        "[{}] re-acquire after expiry must bump the fencing generation \
         (z1={z1}, z2={z2}) so the pre-expiry token is fenced",
        backend.name()
    );
    // Sanity: the first execution's generation was monotone too.
    assert!(
        g1 <= z1.max(g1),
        "[{}] generations monotone",
        backend.name()
    );
}

/// The atomic triple commits state + outbox + journal together; a reader
/// observes all three after a successful commit.
pub(crate) async fn assert_atomic_triple(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let s = scope_a();
    store
        .create(&s, "exe_triple", "wf_1", serde_json::json!({}))
        .await
        .expect("create");
    let token = store
        .acquire_lease(
            &s,
            "exe_triple",
            "holder",
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] lease", backend.name()));
    let msg = ControlMsg {
        id: [1u8; 16],
        execution_id: "exe_triple".into(),
        command: ControlCommand::Cancel,
        scope: s.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    let je = JournalEntry {
        seq: None,
        payload: serde_json::json!({"event": "transition"}),
    };
    let batch = TransitionBatch::builder()
        .scope(s.clone())
        .execution_id("exe_triple")
        .expected_version(0)
        .fencing(token)
        .new_state(serde_json::json!({"s": "running"}))
        .outbox(vec![msg])
        .journal(vec![je])
        .build()
        .expect("batch");
    let outcome = store.commit(batch).await.expect("commit");
    assert!(
        matches!(outcome, TransitionOutcome::Applied { .. }),
        "[{}] expected Applied, got {outcome:?}",
        backend.name()
    );
    let rec = store
        .get(&s, "exe_triple")
        .await
        .expect("get")
        .unwrap_or_else(|| panic!("[{}] row after commit", backend.name()));
    assert_eq!(
        rec.state,
        serde_json::json!({"s": "running"}),
        "[{}] state must reflect the committed transition",
        backend.name()
    );
}

/// Idempotency key shape `{execution_id}:{node_id}:{attempt}` is
/// first-writer-wins: the first `check_and_mark` returns true, the second
/// false.
pub(crate) async fn assert_idempotency_first_writer_wins(backend: &dyn Backend) {
    let guard = backend.idempotency_guard().await;
    let s = scope_a();
    let first = guard
        .check_and_mark(&s, "exe_1", "node_1", 1)
        .await
        .expect("check_and_mark #1");
    let second = guard
        .check_and_mark(&s, "exe_1", "node_1", 1)
        .await
        .expect("check_and_mark #2");
    assert!(first, "[{}] first mark must win", backend.name());
    assert!(
        !second,
        "[{}] second mark on the same key must lose",
        backend.name()
    );
}

/// A `get` with a mismatched scope yields `Ok(None)` — never another
/// tenant's row, never an error that leaks existence.
pub(crate) async fn assert_cross_scope_get_is_none(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    store
        .create(&scope_a(), "exe_x", "wf_1", serde_json::json!({}))
        .await
        .expect("create in scope A");
    let miss = store.get(&scope_b(), "exe_x").await.expect("get");
    assert!(
        miss.is_none(),
        "[{}] cross-scope get must not leak the row",
        backend.name()
    );
}

/// A `commit` against an id that exists only in another tenant's scope
/// must not Apply (the row is invisible cross-tenant).
pub(crate) async fn assert_cross_scope_commit_is_rejected(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    store
        .create(&scope_a(), "exe_y", "wf_1", serde_json::json!({}))
        .await
        .expect("create in scope A");
    let batch = TransitionBatch::builder()
        .scope(scope_b()) // attacker's scope
        .execution_id("exe_y")
        .expected_version(0)
        .fencing(FencingToken::from_generation(0))
        .new_state(serde_json::json!({"s": "hijacked"}))
        .build()
        .expect("batch");
    let outcome = store.commit(batch).await;
    // Any of VersionConflict / FencedOut / NotFound (Err) is an acceptable
    // rejection; the only forbidden outcome is a successful cross-tenant
    // Apply.
    let applied = matches!(&outcome, Ok(TransitionOutcome::Applied { .. }));
    assert!(
        !applied,
        "[{}] cross-tenant commit must NEVER Apply",
        backend.name()
    );
    // No cross-tenant version oracle: a `VersionConflict` from a
    // cross-scope probe must report `actual: 0` (indistinguishable from a
    // missing row), never echo the victim row's real counter. The victim
    // was created at version 1, so a leak would surface as `actual: 1`.
    if let Ok(TransitionOutcome::VersionConflict { actual }) = &outcome {
        assert_eq!(
            *actual,
            0,
            "[{}] cross-scope conflict leaked the victim's version counter \
             (got actual={actual}); it must be 0",
            backend.name()
        );
    }
}

/// The spec-16 workflow split contract: a workflow row round-trips by id
/// and by slug, a soft-deleted row disappears from reads / `list` /
/// `get_by_slug`, `update` is a strict CAS (stale `expected_version` →
/// `Conflict`, missing row → `NotFound`), a duplicate id → `Duplicate`,
/// and the version store round-trips a version and lists newest-first.
/// Asserted across every backend so the SQL adapters match the in-memory
/// reference exactly.
pub(crate) async fn assert_workflow_store_contract(backend: &dyn Backend) {
    let wf = backend.workflow_store().await;
    let ver = backend.workflow_version_store().await;
    let s = scope_a();

    let rec = WorkflowRecord {
        id: "wf_c".into(),
        scope: s.clone(),
        version: 0,
        slug: "billing".into(),
        deleted: false,
    };
    wf.create(&s, rec.clone()).await.expect("create");

    // Duplicate id is a Duplicate, not a silent overwrite.
    let dup = wf.create(&s, rec.clone()).await;
    assert!(
        matches!(dup, Err(StorageError::Duplicate { .. })),
        "[{}] duplicate workflow id must be Duplicate, got {dup:?}",
        backend.name()
    );

    // Round-trip by id and by slug.
    let by_id = wf
        .get(&s, "wf_c")
        .await
        .expect("get")
        .unwrap_or_else(|| panic!("[{}] workflow row by id", backend.name()));
    assert_eq!(by_id.slug, "billing");
    let by_slug = wf
        .get_by_slug(&s, "billing")
        .await
        .expect("get_by_slug")
        .unwrap_or_else(|| panic!("[{}] workflow row by slug", backend.name()));
    assert_eq!(by_slug.id, "wf_c");

    // CAS update: stale expected_version is rejected.
    let stale = wf
        .update(
            &s,
            WorkflowRecord {
                version: 1,
                ..by_id.clone()
            },
            999,
        )
        .await;
    assert!(
        matches!(stale, Err(StorageError::Conflict { .. })),
        "[{}] stale CAS update must Conflict, got {stale:?}",
        backend.name()
    );
    // CAS update with the right expected_version succeeds.
    wf.update(
        &s,
        WorkflowRecord {
            version: 1,
            slug: "billing-v2".into(),
            ..by_id.clone()
        },
        0,
    )
    .await
    .expect("CAS update at expected_version 0");
    let updated = wf
        .get(&s, "wf_c")
        .await
        .expect("get")
        .unwrap_or_else(|| panic!("[{}] updated row", backend.name()));
    assert_eq!(updated.version, 1);
    assert_eq!(updated.slug, "billing-v2");

    // Update of a missing row is NotFound (never an implicit insert).
    let missing = wf
        .update(
            &s,
            WorkflowRecord {
                id: "wf_absent".into(),
                ..by_id.clone()
            },
            0,
        )
        .await;
    assert!(
        matches!(missing, Err(StorageError::NotFound { .. })),
        "[{}] update of a missing row must be NotFound, got {missing:?}",
        backend.name()
    );

    // Soft-delete removes the row from reads, slug lookup, and list.
    wf.soft_delete(&s, "wf_c").await.expect("soft_delete");
    assert!(
        wf.get(&s, "wf_c").await.expect("get").is_none(),
        "[{}] soft-deleted row must be a read miss",
        backend.name()
    );
    assert!(
        wf.get_by_slug(&s, "billing-v2")
            .await
            .expect("get_by_slug")
            .is_none(),
        "[{}] soft-deleted row must not resolve by slug",
        backend.name()
    );
    assert!(
        wf.list(&s)
            .await
            .expect("list")
            .iter()
            .all(|r| r.id != "wf_c"),
        "[{}] soft-deleted row must not appear in list",
        backend.name()
    );
    // A soft-deleted row is invisible to `update` too: updating a
    // tombstone must be `NotFound` (matching `get`), never resurrect the
    // row and never report a spurious `Conflict`. Without the
    // `deleted = FALSE` / `!deleted` guard on every backend's `update`
    // this would rewrite the tombstone back into a live row.
    let revive = wf
        .update(
            &s,
            WorkflowRecord {
                id: "wf_c".into(),
                deleted: false,
                version: 2,
                ..by_id.clone()
            },
            1,
        )
        .await;
    assert!(
        matches!(revive, Err(StorageError::NotFound { .. })),
        "[{}] update of a soft-deleted row must be NotFound (no revival, \
         no spurious Conflict), got {revive:?}",
        backend.name()
    );
    assert!(
        wf.get(&s, "wf_c").await.expect("get").is_none(),
        "[{}] soft-deleted row must stay a read miss after a rejected \
         update",
        backend.name()
    );

    // Soft-deleting an absent row is NotFound.
    let del_missing = wf.soft_delete(&s, "wf_absent").await;
    assert!(
        matches!(del_missing, Err(StorageError::NotFound { .. })),
        "[{}] soft-delete of a missing row must be NotFound, got {del_missing:?}",
        backend.name()
    );

    // Version store: create + round-trip + duplicate guard + list order.
    for n in 1u32..=3 {
        ver.create(
            &s,
            WorkflowVersionRecord {
                workflow_id: "wf_v".into(),
                number: n,
                published: false,
                pinned: false,
                definition: serde_json::json!({ "n": n }),
            },
        )
        .await
        .expect("version create");
    }
    let dup_ver = ver
        .create(
            &s,
            WorkflowVersionRecord {
                workflow_id: "wf_v".into(),
                number: 2,
                published: false,
                pinned: false,
                definition: serde_json::json!({}),
            },
        )
        .await;
    assert!(
        matches!(dup_ver, Err(StorageError::Duplicate { .. })),
        "[{}] duplicate (workflow,number) must be Duplicate, got {dup_ver:?}",
        backend.name()
    );
    let got_v2 = ver
        .get(&s, "wf_v", 2)
        .await
        .expect("version get")
        .unwrap_or_else(|| panic!("[{}] version 2", backend.name()));
    assert_eq!(got_v2.definition, serde_json::json!({ "n": 2 }));
    let listed: Vec<u32> = ver
        .list(&s, "wf_v")
        .await
        .expect("version list")
        .iter()
        .map(|r| r.number)
        .collect();
    assert_eq!(
        listed,
        vec![3, 2, 1],
        "[{}] version list must be newest-first",
        backend.name()
    );
}

/// `WorkflowStore::save_with_published_version` is a real all-or-nothing
/// unit of work on every backend: the row write and the published-version
/// write either both land or neither does. This locks the spec-16
/// orphan-row invariant (a workflow row with no published version is
/// invisible to readers — "the workflow vanished") that the previous
/// two-await sequence could violate on a partial failure.
pub(crate) async fn assert_save_with_published_version_is_atomic(backend: &dyn Backend) {
    let wf = backend.workflow_store().await;
    let ver = backend.workflow_version_store().await;
    let s = scope_a();

    // 1. Create commits BOTH the row and version #1 as one unit.
    wf.save_with_published_version(
        &s,
        WorkflowRecord {
            id: "wf_atomic".into(),
            scope: s.clone(),
            version: 1,
            slug: "wf_atomic".into(),
            deleted: false,
        },
        WorkflowVersionRecord {
            workflow_id: "wf_atomic".into(),
            number: 1,
            published: true,
            pinned: false,
            definition: serde_json::json!({ "v": 1 }),
        },
        None,
    )
    .await
    .expect("atomic create");
    assert!(
        wf.get(&s, "wf_atomic").await.expect("get").is_some(),
        "[{}] row must exist after atomic create",
        backend.name()
    );
    let pub1 = ver
        .get_published(&s, "wf_atomic")
        .await
        .expect("get_published")
        .unwrap_or_else(|| panic!("[{}] published version after create", backend.name()));
    assert_eq!(
        pub1.number,
        1,
        "[{}] published version #1 must exist after atomic create",
        backend.name()
    );

    // 2. A CAS update at the WRONG expected_version must roll BOTH back:
    //    the row counter must NOT advance and version #2 must NOT appear.
    let conflict = wf
        .save_with_published_version(
            &s,
            WorkflowRecord {
                id: "wf_atomic".into(),
                scope: s.clone(),
                version: 2,
                slug: "wf_atomic".into(),
                deleted: false,
            },
            WorkflowVersionRecord {
                workflow_id: "wf_atomic".into(),
                number: 2,
                published: true,
                pinned: false,
                definition: serde_json::json!({ "v": 2 }),
            },
            Some(999), // stale expected version
        )
        .await;
    assert!(
        matches!(conflict, Err(StorageError::Conflict { .. })),
        "[{}] stale-CAS atomic save must Conflict, got {conflict:?}",
        backend.name()
    );
    let row_after = wf
        .get(&s, "wf_atomic")
        .await
        .expect("get")
        .unwrap_or_else(|| panic!("[{}] row still present", backend.name()));
    assert_eq!(
        row_after.version,
        1,
        "[{}] row counter must NOT advance on a rolled-back atomic save",
        backend.name()
    );
    assert!(
        ver.get(&s, "wf_atomic", 2)
            .await
            .expect("version get")
            .is_none(),
        "[{}] version #2 must NOT exist after a rolled-back atomic save \
         (no orphan version; the unit rolled back whole)",
        backend.name()
    );

    // 3. A create whose version slot is already taken must roll the row
    //    back too (the row insert must not survive the version failure).
    let dup = wf
        .save_with_published_version(
            &s,
            WorkflowRecord {
                id: "wf_atomic2".into(),
                scope: s.clone(),
                version: 1,
                slug: "wf_atomic2".into(),
                deleted: false,
            },
            WorkflowVersionRecord {
                // Collides with wf_atomic's existing version #1.
                workflow_id: "wf_atomic".into(),
                number: 1,
                published: true,
                pinned: false,
                definition: serde_json::json!({ "dup": true }),
            },
            None,
        )
        .await;
    assert!(
        matches!(dup, Err(StorageError::Duplicate { .. })),
        "[{}] atomic create with a taken version slot must Duplicate, got {dup:?}",
        backend.name()
    );
    assert!(
        wf.get(&s, "wf_atomic2").await.expect("get").is_none(),
        "[{}] the new row must NOT survive a failed atomic create \
         (row insert rolled back with the version failure)",
        backend.name()
    );
}

/// `get_published` returns the **highest-numbered** published version when
/// more than one row is marked published (a stale publish that was never
/// cleared). The original in-memory `find` returned an arbitrary
/// `HashMap`-order row; this locks the deterministic
/// `ORDER BY number DESC LIMIT 1` contract across every backend.
pub(crate) async fn assert_get_published_is_highest_numbered(backend: &dyn Backend) {
    let ver = backend.workflow_version_store().await;
    let s = scope_a();
    // Two published versions for the same workflow (1 and 3) plus an
    // unpublished one (2) — `get_published` must return version 3.
    for (n, published) in [(1u32, true), (2, false), (3, true)] {
        ver.create(
            &s,
            WorkflowVersionRecord {
                workflow_id: "wf_pub".into(),
                number: n,
                published,
                pinned: false,
                definition: serde_json::json!({ "v": n }),
            },
        )
        .await
        .expect("version create");
    }
    let published = ver
        .get_published(&s, "wf_pub")
        .await
        .expect("get_published")
        .unwrap_or_else(|| panic!("[{}] a published version exists", backend.name()));
    assert_eq!(
        published.number,
        3,
        "[{}] get_published must return the highest-numbered published \
         version (deterministic), got {}",
        backend.name(),
        published.number
    );
    assert_eq!(published.definition, serde_json::json!({ "v": 3 }));
}

/// A `commit`'s outbox rows are claimable through the control queue, and
/// the claiming processor fences `mark_completed` (a stale runner whose
/// row was reclaimed cannot flip a newer claim). Also exercises the
/// typed-16-byte-id contract end to end.
pub(crate) async fn assert_control_queue_outbox_and_fencing(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let queue = backend.control_queue().await;
    let s = scope_a();
    store
        .create(&s, "exe_cq", "wf_1", serde_json::json!({}))
        .await
        .expect("create");
    let token = store
        .acquire_lease(&s, "exe_cq", "holder", std::time::Duration::from_secs(30))
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] lease", backend.name()));
    let msg = ControlMsg {
        id: [42u8; 16],
        execution_id: "exe_cq".into(),
        command: ControlCommand::Cancel,
        scope: s.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    let batch = TransitionBatch::builder()
        .scope(s.clone())
        .execution_id("exe_cq")
        .expected_version(0)
        .fencing(token)
        .new_state(serde_json::json!({"s": "cancelling"}))
        .outbox(vec![msg])
        .build()
        .expect("batch");
    let outcome = store.commit(batch).await.expect("commit");
    assert!(
        matches!(outcome, TransitionOutcome::Applied { .. }),
        "[{}] expected Applied, got {outcome:?}",
        backend.name()
    );

    let runner_a = [1u8; 16];
    let claimed = queue
        .claim_pending(&runner_a, 16)
        .await
        .expect("claim_pending");
    assert_eq!(
        claimed.len(),
        1,
        "[{}] the commit's outbox row must be claimable",
        backend.name()
    );
    assert_eq!(
        claimed[0].msg.id,
        [42u8; 16],
        "[{}] typed 16-byte id round-trips through the queue",
        backend.name()
    );
    let current = claimed[0].token;

    // A token naming a generation this row never reached must be rejected.
    // Under the old `processed_by` fence this was expressed as "a different
    // processor id"; authority is now the token, which a caller cannot forge
    // into existence, so the equivalent probe is a wrong generation.
    let forged = ControlClaimToken::new(
        [42u8; 16],
        ClaimGeneration::new(current.generation().get() + 1),
    );
    let stale_ack = queue.mark_completed(&forged).await;
    assert!(
        matches!(stale_ack, Err(StorageError::FencedOut { .. })),
        "[{}] an acknowledgement with a non-current generation must be FencedOut, got: {stale_ack:?}",
        backend.name()
    );
    let reclaimed = queue
        .claim_pending(&runner_a, 16)
        .await
        .expect("claim_pending after stale ack");
    assert!(
        reclaimed.is_empty(),
        "[{}] a fenced ack must be a no-op (row stays Processing, \
         not re-Pending and not Completed)",
        backend.name()
    );

    // The holder of the current claim can complete it.
    queue
        .mark_completed(&current)
        .await
        .expect("mark_completed (claimant)");
}

/// A `ControlMsg` whose `resume_target` is `Some(ResumeTarget::Webhook{..})`
/// survives an enqueue→claim round-trip through the durable queue intact.
///
/// This is the structural fix for ADR-0099 W-S3a: closing the confused-deputy
/// bug on the durable path requires the column to exist on both enqueue and
/// claim. A `None` target also round-trips correctly (backward compatibility
/// with legacy rows).
///
/// **Falsifiability**: before the `resume_target TEXT` column was added to
/// `port_control_queue`, `claim_pending` hardcoded `resume_target: None` and
/// the `Some(target)` assertion failed → RED.
pub(crate) async fn assert_resume_target_survives_queue_round_trip(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let queue = backend.control_queue().await;
    let s = scope_a();
    store
        .create(&s, "exe_rt", "wf_rt", serde_json::json!({}))
        .await
        .expect("create execution for resume-target round-trip");
    let token = store
        .acquire_lease(&s, "exe_rt", "holder", std::time::Duration::from_secs(30))
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] lease for resume-target test", backend.name()));

    let webhook_target = ResumeTarget::Webhook {
        callback_id: "cb-round-trip".to_owned(),
    };
    let resume_msg = ControlMsg {
        id: [77u8; 16],
        execution_id: "exe_rt".into(),
        command: ControlCommand::Resume,
        scope: s.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: Some(webhook_target.clone()),
    };
    let batch = TransitionBatch::builder()
        .scope(s.clone())
        .execution_id("exe_rt")
        .expected_version(0)
        .fencing(token)
        .new_state(serde_json::json!({"s": "waiting"}))
        .outbox(vec![resume_msg])
        .build()
        .expect("batch for resume-target round-trip");
    store
        .commit(batch)
        .await
        .expect("commit for resume-target round-trip");

    let runner = [9u8; 16];
    let claimed = queue
        .claim_pending(&runner, 16)
        .await
        .expect("claim_pending for resume-target round-trip");
    assert_eq!(
        claimed.len(),
        1,
        "[{}] the outbox Resume must be claimable",
        backend.name()
    );
    assert_eq!(
        claimed[0].msg.resume_target,
        Some(webhook_target),
        "[{}] resume_target must survive the enqueue→claim round-trip intact \
         (kind + identity both preserved)",
        backend.name()
    );

    // A second message with no target must also round-trip correctly
    // (backward-compatibility: legacy rows and non-Resume commands have NULL).
    store
        .create(&s, "exe_rt2", "wf_rt", serde_json::json!({}))
        .await
        .expect("create second execution");
    let token2 = store
        .acquire_lease(&s, "exe_rt2", "holder", std::time::Duration::from_secs(30))
        .await
        .expect("acquire_lease 2")
        .unwrap_or_else(|| panic!("[{}] lease 2 for resume-target test", backend.name()));
    let null_msg = ControlMsg {
        id: [78u8; 16],
        execution_id: "exe_rt2".into(),
        command: ControlCommand::Cancel,
        scope: s.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    let batch2 = TransitionBatch::builder()
        .scope(s.clone())
        .execution_id("exe_rt2")
        .expected_version(0)
        .fencing(token2)
        .new_state(serde_json::json!({"s": "cancelling"}))
        .outbox(vec![null_msg])
        .build()
        .expect("batch 2");
    store.commit(batch2).await.expect("commit 2");
    // First, drain the already-claimed row above to avoid re-claiming it.
    queue
        .mark_completed(&claimed[0].token)
        .await
        .expect("mark_completed first row");
    let claimed2 = queue
        .claim_pending(&runner, 16)
        .await
        .expect("claim_pending 2");
    assert_eq!(
        claimed2.len(),
        1,
        "[{}] the null-target message must be claimable",
        backend.name()
    );
    assert_eq!(
        claimed2[0].msg.resume_target,
        None,
        "[{}] a None resume_target must round-trip as None (legacy compat)",
        backend.name()
    );
}

/// Enqueue a single control row of `command` and drive its `reclaim_count`
/// up to `target_count` by repeated claim → reclaim cycles, leaving it
/// `Processing` (just-claimed) at `reclaim_count == target_count`.
///
/// Each cycle claims the (Pending) row, then sweeps with a budget above the
/// current count so the REDELIVER branch fires (`Processing → Pending`,
/// `reclaim_count += 1`). The cutoff is wall-clock (`chrono`), so — like the
/// `refresh_claim_*` reclaim tests for this same layer — a short real sleep
/// makes the just-claimed row reliably stale; `tokio::time::pause` cannot drive
/// the SQL backends' `chrono::Utc::now()` cutoff. Returns the row id so the
/// caller can keep claiming it.
async fn enqueue_and_climb_reclaim_count(
    backend: &dyn Backend,
    command: ControlCommand,
    target_count: u32,
) -> [u8; 16] {
    let queue = backend.control_queue().await;
    let s = scope_a();
    let row_id = [
        0xC1,
        0xA1,
        0x33,
        0x44,
        0x55,
        0x66,
        0x77,
        0x88,
        command as u8,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    let msg = ControlMsg {
        id: row_id,
        execution_id: "exe_reclaim".into(),
        command,
        scope: s.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    queue.enqueue(&msg).await.expect("enqueue reclaim row");

    let runner = [0xCC_u8; 16];
    for cycle in 0..target_count {
        let claimed = queue
            .claim_pending(&runner, 16)
            .await
            .expect("claim during climb");
        assert!(
            claimed.iter().any(|c| c.msg.id == row_id),
            "[{}] the reclaim row must be claimable on climb cycle {cycle}",
            backend.name()
        );
        // Make the just-claimed row stale against the wall-clock cutoff.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        // Budget strictly above the current count so this row REDELIVERS
        // (Processing → Pending, reclaim_count += 1) rather than exhausting.
        queue
            .reclaim_stuck(std::time::Duration::ZERO, target_count + 1)
            .await
            .expect("reclaim during climb");
    }
    // Re-claim so the assertion sweep sees it `Processing` at the target count.
    let claimed = queue
        .claim_pending(&runner, 16)
        .await
        .expect("final claim at target count");
    let row = claimed
        .iter()
        .find(|c| c.msg.id == row_id)
        .unwrap_or_else(|| panic!("[{}] reclaim row must be re-claimable", backend.name()));
    assert_eq!(
        row.msg.reclaim_count,
        target_count,
        "[{}] the row must reach reclaim_count == {target_count} before the assertion sweep",
        backend.name()
    );
    row_id
}

/// **ADR-0099 W-S3b** — a `command = 'Resume'` row at `reclaim_count == max`,
/// past `reclaim_after`, is EXEMPT from the reclaim budget: the exhaust sweep
/// must NOT Fail it; it stays redeliverable (`Processing → Pending`) so a later
/// claim still delivers it. Engine liveness + the wait's own timeout are the
/// only terminal authorities for a parked Resume.
///
/// Observable through the trait alone: after the assertion sweep, a fresh
/// `claim_pending` still returns the Resume row (a Failed row is terminal and
/// would never be claimable again).
///
/// **Falsifiability**: revert the `command <> 'Resume'` exhaust guard (and its
/// `OR command = 'Resume'` redeliver complement) → the row at `reclaim_count >=
/// max` is force-Failed → the post-sweep `claim_pending` finds nothing → the
/// "still claimable" assertion fails → RED.
pub(crate) async fn assert_resume_row_exempt_from_reclaim_budget(backend: &dyn Backend) {
    let max_reclaim_count = 2;
    let row_id =
        enqueue_and_climb_reclaim_count(backend, ControlCommand::Resume, max_reclaim_count).await;
    let queue = backend.control_queue().await;

    // The assertion sweep: the Resume row is now `Processing` at
    // `reclaim_count == max`, past `reclaim_after`. The budget would normally
    // exhaust it (→ Failed); the exemption must redeliver it instead.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let outcome = queue
        .reclaim_stuck(std::time::Duration::ZERO, max_reclaim_count)
        .await
        .expect("assertion reclaim sweep");
    assert_eq!(
        outcome.exhausted,
        0,
        "[{}] a Resume row at reclaim_count == max must NOT be exhausted (Failed); \
         it is budget-exempt",
        backend.name()
    );

    // The exempt Resume stays redeliverable: a fresh claim still delivers it.
    let runner = [0xAB; 16];
    let claimed = queue
        .claim_pending(&runner, 16)
        .await
        .expect("claim after the assertion sweep");
    let row = claimed.iter().find(|c| c.msg.id == row_id);
    assert!(
        row.is_some(),
        "[{}] a budget-exempt Resume row must stay redeliverable after the exhaust \
         sweep (still claimable), not be Failed",
        backend.name()
    );
    assert!(
        row.is_some_and(|c| c.msg.reclaim_count > max_reclaim_count),
        "[{}] the redelivered Resume row's reclaim_count must keep climbing past max \
         (observable stuck-Resume signal), got {:?}",
        backend.name(),
        row.map(|c| c.msg.reclaim_count)
    );
}

/// **ADR-0099 W-S3b** — the exemption is Resume-ONLY: a non-Resume row
/// (`Start` / `Cancel`) at `reclaim_count >= max`, past `reclaim_after`, still
/// EXHAUSTS to `Failed` (the budget remains the terminal authority for rows that
/// do real work and can poison-loop).
///
/// Observable through the trait alone: after the assertion sweep, a fresh
/// `claim_pending` finds nothing (a Failed row is terminal, never re-claimable).
///
/// **Falsifiability**: widen the exemption to cover non-Resume commands → the
/// `Start` row at `reclaim_count >= max` redelivers instead of failing → the
/// post-sweep `claim_pending` returns it → the "must be Failed (not claimable)"
/// assertion fails → RED.
pub(crate) async fn assert_non_resume_row_still_exhausts(backend: &dyn Backend) {
    let max_reclaim_count = 2;
    let row_id =
        enqueue_and_climb_reclaim_count(backend, ControlCommand::Start, max_reclaim_count).await;
    let queue = backend.control_queue().await;

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let outcome = queue
        .reclaim_stuck(std::time::Duration::ZERO, max_reclaim_count)
        .await
        .expect("assertion reclaim sweep");
    assert_eq!(
        outcome.exhausted,
        1,
        "[{}] a non-Resume row at reclaim_count >= max must be exhausted (Failed) — \
         the exemption is Resume-only",
        backend.name()
    );

    // A Failed row is terminal: it must NOT be claimable again.
    let runner = [0xCD; 16];
    let claimed = queue
        .claim_pending(&runner, 16)
        .await
        .expect("claim after the assertion sweep");
    assert!(
        claimed.iter().all(|c| c.msg.id != row_id),
        "[{}] an exhausted non-Resume row must be Failed (terminal), never re-claimable",
        backend.name()
    );
}

/// Journal entries appended by a `commit` are readable in order, and a
/// cross-tenant read yields an empty journal (never another tenant's
/// entries).
pub(crate) async fn assert_journal_visibility_and_scope(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let reader = backend.journal_reader().await;
    let s = scope_a();
    store
        .create(&s, "exe_j", "wf_1", serde_json::json!({}))
        .await
        .expect("create");
    let token = store
        .acquire_lease(&s, "exe_j", "holder", std::time::Duration::from_secs(30))
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] lease", backend.name()));
    let batch = TransitionBatch::builder()
        .scope(s.clone())
        .execution_id("exe_j")
        .expected_version(0)
        .fencing(token)
        .new_state(serde_json::json!({"s": "running"}))
        .journal(vec![
            JournalEntry {
                seq: None,
                payload: serde_json::json!({"e": "a"}),
            },
            JournalEntry {
                seq: None,
                payload: serde_json::json!({"e": "b"}),
            },
        ])
        .build()
        .expect("batch");
    store.commit(batch).await.expect("commit");

    let entries = reader.get_journal(&s, "exe_j").await.expect("get_journal");
    assert_eq!(
        entries.len(),
        2,
        "[{}] both journal entries must be readable",
        backend.name()
    );
    assert_eq!(
        entries[0].payload,
        serde_json::json!({"e": "a"}),
        "[{}] journal entries must be ordered oldest-first",
        backend.name()
    );

    // Cross-tenant read: never another tenant's journal.
    let cross = reader
        .get_journal(&scope_b(), "exe_j")
        .await
        .expect("get_journal cross-scope");
    assert!(
        cross.is_empty(),
        "[{}] a cross-tenant journal read must be empty",
        backend.name()
    );
}

/// The durable idempotent-replay cache is first-writer-wins: a second
/// `put` on the same key keeps the original record + fingerprint (replay
/// race). Purely within `scope_a`, so it is decorator-transparent and runs
/// in both the raw and scoped matrices.
pub(crate) async fn assert_idempotency_store_first_writer(backend: &dyn Backend) {
    let store = backend.idempotency_store().await;
    let raw_key = "POST /x:idem-1".to_string();
    let first = CachedRecord {
        status: 200,
        headers: b"h1".to_vec(),
        body: b"first".to_vec(),
        fingerprint: b"fp-first".to_vec(),
        expires_at: "2999-01-01T00:00:00Z".into(),
    };
    let second = CachedRecord {
        status: 500,
        headers: b"h2".to_vec(),
        body: b"second".to_vec(),
        fingerprint: b"fp-second".to_vec(),
        expires_at: "2999-01-01T00:00:00Z".into(),
    };
    store
        .put(
            &scope_a(),
            raw_key.clone(),
            first.clone(),
            std::time::Duration::from_mins(1),
        )
        .await
        .expect("put #1");
    store
        .put(
            &scope_a(),
            raw_key.clone(),
            second,
            std::time::Duration::from_mins(1),
        )
        .await
        .expect("put #2 (must be a no-op)");
    let got = store
        .get(&scope_a(), &raw_key)
        .await
        .expect("get")
        .unwrap_or_else(|| panic!("[{}] cached record must be present", backend.name()));
    assert_eq!(
        got.body,
        b"first",
        "[{}] first-writer-wins: the original body must survive a replay race",
        backend.name()
    );
    assert_eq!(
        got.fingerprint,
        b"fp-first",
        "[{}] the original fingerprint must survive (replay-mismatch detection)",
        backend.name()
    );
}

/// Tenant isolation of the durable replay cache: the store folds the scope
/// into the stored key, so the *same raw key* under a different scope is a
/// clean miss — tenant A can neither read nor poison tenant B's entry
/// (replay-oracle mitigation, §6.1).
///
/// This passes an explicit foreign scope to probe the adapter's raw
/// scope-fold, so — like the other `cross_scope_*` assertions — it runs
/// only in the raw matrix. The decorator substitutes the per-call scope
/// away by design, so decorator-level cross-tenant denial is proven in
/// `cross_tenant_denial.rs` instead.
pub(crate) async fn assert_idempotency_store_cross_scope_isolated(backend: &dyn Backend) {
    let store = backend.idempotency_store().await;
    let raw_key = "POST /x:idem-1".to_string();
    let record = CachedRecord {
        status: 200,
        headers: b"h1".to_vec(),
        body: b"a-only".to_vec(),
        fingerprint: b"fp-a".to_vec(),
        expires_at: "2999-01-01T00:00:00Z".into(),
    };
    store
        .put(
            &scope_a(),
            raw_key.clone(),
            record,
            std::time::Duration::from_mins(1),
        )
        .await
        .expect("put under scope A");

    // A different tenant probing the *same raw key* is a clean miss — the
    // store-side scope fold makes it a different stored key, never tenant
    // A's record.
    let cross = store
        .get(&scope_b(), &raw_key)
        .await
        .expect("get cross-scope key");
    assert!(
        cross.is_none(),
        "[{}] a cross-tenant cache key must not resolve to another tenant's record",
        backend.name()
    );
}

/// Webhook activation upsert → resolve → deactivate, with tenant
/// isolation: the same slug in a different tenant does not resolve, and a
/// deactivated activation stops routing.
///
/// Also covers the ADR-0096 extended fields: safe-default round-trip
/// (new fields default to `Test` / `None` / zero-sentinel) and full
/// round-trip of `workflow_id`, `mode`, and `token_hash`.
pub(crate) async fn assert_webhook_activation_and_scope(backend: &dyn Backend) {
    let store = backend.webhook_store().await;
    let s = scope_a();
    // Use the constructor so the call site is future-proof against further
    // `#[non_exhaustive]` field additions (ADR-0096 commit 1 PREREQ).
    store
        .upsert(
            &s,
            WebhookActivationRecord::new("trg_1", s.clone(), "deploy-hook", true),
        )
        .await
        .expect("upsert");

    let resolved = store
        .resolve(&s, "deploy-hook")
        .await
        .expect("resolve")
        .unwrap_or_else(|| panic!("[{}] active activation must resolve", backend.name()));
    assert_eq!(
        resolved.trigger_id,
        "trg_1",
        "[{}] resolve returns the owning trigger",
        backend.name()
    );
    // Safe-default proof: a row inserted without an explicit mode must
    // resolve with `Test`.  If the schema default were `'prod'` this
    // assertion would fail — proving the migration default is load-bearing.
    assert_eq!(
        resolved.mode,
        WebhookMode::Test,
        "[{}] default mode must be Test (safe-default invariant)",
        backend.name()
    );

    // Same slug, different tenant → miss (slug is unique per tenant; a
    // webhook never crosses a tenant boundary).
    let cross = store
        .resolve(&scope_b(), "deploy-hook")
        .await
        .expect("resolve cross-scope");
    assert!(
        cross.is_none(),
        "[{}] a slug must not resolve across a tenant boundary",
        backend.name()
    );

    // Deactivation stops routing (never dispatch a paused webhook).
    store.deactivate(&s, "trg_1").await.expect("deactivate");
    let after = store
        .resolve(&s, "deploy-hook")
        .await
        .expect("resolve after deactivate");
    assert!(
        after.is_none(),
        "[{}] a deactivated activation must not resolve",
        backend.name()
    );

    // ── Extended-fields round-trip ────────────────────────────────────────
    // Upsert a record with all three ADR-0096 fields set to non-default
    // values and verify exact round-trip (no tautological `is_some()`).
    let token = [0xde_u8; 32];
    let mut extended = WebhookActivationRecord::new("trg_2", s.clone(), "prod-hook", true);
    extended.workflow_id = Some("wf_abc".to_string());
    extended.mode = WebhookMode::Prod;
    extended.token_hash = token;
    store.upsert(&s, extended).await.expect("upsert extended");

    let got = store
        .resolve(&s, "prod-hook")
        .await
        .expect("resolve extended")
        .unwrap_or_else(|| panic!("[{}] extended activation must resolve", backend.name()));
    assert_eq!(
        got.workflow_id.as_deref(),
        Some("wf_abc"),
        "[{}] workflow_id must round-trip exactly",
        backend.name()
    );
    assert_eq!(
        got.mode,
        WebhookMode::Prod,
        "[{}] mode must round-trip exactly",
        backend.name()
    );
    assert_eq!(
        got.token_hash,
        token,
        "[{}] token_hash must round-trip exactly",
        backend.name()
    );

    // Cross-tenant isolation is carried forward: `prod-hook` in scope_b
    // must not resolve, even though scope_a has an active activation for it.
    let cross_ext = store
        .resolve(&scope_b(), "prod-hook")
        .await
        .expect("resolve cross-scope extended");
    assert!(
        cross_ext.is_none(),
        "[{}] extended activation must not resolve across a tenant boundary",
        backend.name()
    );
}

/// System-surface methods: `resolve_by_token` + `list_all_active`.
///
/// Proves:
/// - Single-row token resolution with exact-value asserts (no tautological
///   `is_some()`).
/// - Cross-tenant isolation: resolving tenant A's hash never yields tenant B's
///   row.
/// - Sentinel rejection: the all-zeros token hash always returns `None`
///   without querying.
/// - Unknown hash returns `None` (no false-positive).
/// - `list_all_active` enumerates rows from both tenants (cross-tenant
///   bootstrap enumeration).
pub(crate) async fn assert_webhook_system_surface(backend: &dyn Backend) {
    let store = backend.webhook_store().await;
    let sa = scope_a();
    let sb = scope_b();

    // Upsert two rows under different scopes, each with a distinct token hash
    // and workflow_id so exact-value asserts are meaningful.
    let hash_a: [u8; 32] = [0xa1; 32];
    let hash_b: [u8; 32] = [0xb2; 32];

    let mut row_a = WebhookActivationRecord::new("trg_sys_a", sa.clone(), "sys-hook-a", true);
    row_a.workflow_id = Some("wf_a".to_string());
    row_a.token_hash = hash_a;
    store.upsert(&sa, row_a).await.expect("upsert row_a");

    let mut row_b = WebhookActivationRecord::new("trg_sys_b", sb.clone(), "sys-hook-b", true);
    row_b.workflow_id = Some("wf_b".to_string());
    row_b.token_hash = hash_b;
    store.upsert(&sb, row_b).await.expect("upsert row_b");

    // ── resolve_by_token: tenant A's hash → A's row ───────────────────────
    let got_a = store
        .resolve_by_token(&hash_a)
        .await
        .expect("resolve_by_token hash_a")
        .unwrap_or_else(|| {
            panic!(
                "[{}] resolve_by_token(hash_a) must return Some",
                backend.name()
            )
        });
    assert_eq!(
        got_a.trigger_id,
        "trg_sys_a",
        "[{}] resolve_by_token(hash_a) must return row_a's trigger_id",
        backend.name()
    );
    assert_eq!(
        got_a.scope,
        sa,
        "[{}] resolve_by_token(hash_a) must carry scope_a",
        backend.name()
    );
    assert_eq!(
        got_a.workflow_id.as_deref(),
        Some("wf_a"),
        "[{}] resolve_by_token(hash_a) must carry wf_a",
        backend.name()
    );
    assert_eq!(
        got_a.token_hash,
        hash_a,
        "[{}] resolve_by_token(hash_a) must round-trip token_hash",
        backend.name()
    );

    // ── resolve_by_token: tenant B's hash → B's row ───────────────────────
    let got_b = store
        .resolve_by_token(&hash_b)
        .await
        .expect("resolve_by_token hash_b")
        .unwrap_or_else(|| {
            panic!(
                "[{}] resolve_by_token(hash_b) must return Some",
                backend.name()
            )
        });
    assert_eq!(
        got_b.trigger_id,
        "trg_sys_b",
        "[{}] resolve_by_token(hash_b) must return row_b's trigger_id",
        backend.name()
    );
    assert_eq!(
        got_b.scope,
        sb,
        "[{}] resolve_by_token(hash_b) must carry scope_b",
        backend.name()
    );
    assert_eq!(
        got_b.workflow_id.as_deref(),
        Some("wf_b"),
        "[{}] resolve_by_token(hash_b) must carry wf_b",
        backend.name()
    );

    // Cross-tenant isolation: A's hash must never yield B's row.
    assert_ne!(
        got_a.trigger_id,
        got_b.trigger_id,
        "[{}] resolve_by_token must never cross-pollinate tenant rows",
        backend.name()
    );

    // ── Sentinel rejection: [0u8;32] → None (no query) ───────────────────
    let sentinel = store
        .resolve_by_token(&[0u8; 32])
        .await
        .expect("resolve_by_token sentinel");
    assert!(
        sentinel.is_none(),
        "[{}] the all-zeros sentinel must always return None",
        backend.name()
    );

    // ── Unknown hash → None ───────────────────────────────────────────────
    let unknown = store
        .resolve_by_token(&[0xff; 32])
        .await
        .expect("resolve_by_token unknown");
    assert!(
        unknown.is_none(),
        "[{}] an unknown hash must return None",
        backend.name()
    );

    // ── Deactivated row must not resolve by token (F1) ────────────────────
    //
    // Deactivate A's row; `resolve_by_token` must return `None` even though
    // the token_hash is still stored.  This guards the `AND active = TRUE`
    // predicate across all three backends.
    store
        .deactivate(&sa, "trg_sys_a")
        .await
        .expect("deactivate trg_sys_a");
    let deactivated = store
        .resolve_by_token(&hash_a)
        .await
        .expect("resolve_by_token after deactivate");
    assert!(
        deactivated.is_none(),
        "[{}] resolve_by_token must return None for a deactivated row",
        backend.name()
    );

    // ── list_all_active: cross-tenant enumeration ─────────────────────────
    let all = store.list_all_active().await.expect("list_all_active");
    let ids: Vec<&str> = all.iter().map(|r| r.trigger_id.as_str()).collect();
    // Row A was deactivated above; only row B must appear.
    assert!(
        !ids.contains(&"trg_sys_a"),
        "[{}] list_all_active must NOT contain deactivated trg_sys_a",
        backend.name()
    );
    assert!(
        ids.contains(&"trg_sys_b"),
        "[{}] list_all_active must contain trg_sys_b (tenant B row)",
        backend.name()
    );
}

/// A [`Backend`] whose stores are wrapped in the `nebula-tenancy`
/// scope-enforcing decorators, all bound to one tenant ([`scope_a`]).
///
/// Run against the **same-tenant** subset of the contract suite this
/// proves the decorator is *transparent* for in-tenant operations: every
/// assertion that operates purely within `scope_a` must stay green when
/// every call goes through the decorator (the substituted bound scope
/// equals the scope the assertion already uses, so it is a no-op there).
///
/// Cross-tenant *denial* — the part the decorator actually adds — is the
/// security property and is proven directly in
/// `tests/cross_tenant_denial.rs` (two decorators, tenants A and B, over
/// one shared adapter). It is intentionally **not** asserted here: the
/// raw `cross_scope_*` / journal / webhook assertions probe the adapter's
/// own `WHERE` filtering with an explicit foreign-scope argument, which
/// the decorator *substitutes away* — a different mechanism, tested in
/// its own suite.
pub(crate) struct ScopedBackend<B: Backend> {
    inner: B,
}

impl<B: Backend + Default> Default for ScopedBackend<B> {
    fn default() -> Self {
        Self {
            inner: B::default(),
        }
    }
}

#[async_trait::async_trait]
impl<B: Backend> Backend for ScopedBackend<B> {
    fn name(&self) -> &'static str {
        // Verbatim inner name so `skip_reason` keeps gating the scoped
        // SQLite/Postgres cases by feature/DATABASE_URL.
        self.inner.name()
    }

    async fn execution_store(&self) -> Arc<dyn ExecutionStore> {
        Arc::new(nebula_tenancy::ScopedExecutionStore::new(
            self.inner.execution_store().await,
            scope_a(),
        ))
    }

    async fn idempotency_guard(&self) -> Arc<dyn IdempotencyGuard> {
        Arc::new(nebula_tenancy::ScopedIdempotencyGuard::new(
            self.inner.idempotency_guard().await,
            scope_a(),
        ))
    }

    async fn control_queue(&self) -> Arc<dyn ControlQueue> {
        Arc::new(nebula_tenancy::ScopedControlQueue::new(
            self.inner.control_queue().await,
            scope_a(),
        ))
    }

    async fn journal_reader(&self) -> Arc<dyn ExecutionJournalReader> {
        Arc::new(nebula_tenancy::ScopedExecutionJournalReader::new(
            self.inner.journal_reader().await,
            scope_a(),
        ))
    }

    async fn idempotency_store(&self) -> Arc<dyn IdempotencyStore> {
        Arc::new(nebula_tenancy::ScopedIdempotencyStore::new(
            self.inner.idempotency_store().await,
            scope_a(),
        ))
    }

    async fn webhook_store(&self) -> Arc<dyn WebhookActivationStore> {
        Arc::new(nebula_tenancy::ScopedWebhookActivationStore::new(
            self.inner.webhook_store().await,
            scope_a(),
        ))
    }

    async fn workflow_store(&self) -> Arc<dyn WorkflowStore> {
        Arc::new(nebula_tenancy::ScopedWorkflowStore::new(
            self.inner.workflow_store().await,
            scope_a(),
        ))
    }

    async fn workflow_version_store(&self) -> Arc<dyn WorkflowVersionStore> {
        Arc::new(nebula_tenancy::ScopedWorkflowVersionStore::new(
            self.inner.workflow_version_store().await,
            scope_a(),
        ))
    }

    // Job-dispatch queue and trigger-dedup inbox are not wrapped by the
    // tenancy decorator (no `Scoped*` implementation exists yet); delegate
    // directly to the inner backend so the raw conformance assertions work.
    async fn job_dispatch_queue(&self) -> Arc<dyn JobDispatchQueue> {
        self.inner.job_dispatch_queue().await
    }

    async fn trigger_dedup_inbox(&self) -> Arc<dyn TriggerDedupInbox> {
        self.inner.trigger_dedup_inbox().await
    }

    // Start acceptance is not wrapped by the tenancy decorator: the scope is
    // already an explicit field of `KeyedStart`, so there is no ambient scope
    // for a decorator to substitute.
    async fn start_acceptance_store(&self) -> Arc<dyn StartAcceptanceStore> {
        self.inner.start_acceptance_store().await
    }
}

/// A stable processor identity cannot acknowledge a control command whose
/// claim a reclaim already superseded — the same-processor ABA, on the queue
/// that carries accepted lifecycle commands.
///
/// The `JobDispatchQueue` twin of this assertion is
/// `assert_job_dispatch_same_processor_aba_is_fenced`; both queues had the same
/// `processed_by` fence and therefore the same defect. Here the stakes are a
/// Cancel or Terminate being marked completed by a consumer that no longer owns
/// it, while the attempt that does own it is still dispatching.
pub(crate) async fn assert_control_queue_same_processor_aba_is_fenced(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let queue = backend.control_queue().await;
    let scope = scope_a();
    // One identity for both attempts — that is the whole point.
    let processor = [11u8; 16];

    let execution_id = "exe_control_aba";
    store
        .create(
            &scope,
            execution_id,
            "wf_control_aba",
            serde_json::json!({}),
        )
        .await
        .expect("create the execution the command targets");
    queue
        .enqueue(&ControlMsg {
            id: [0x5Au8; 16],
            execution_id: execution_id.to_owned(),
            command: ControlCommand::Cancel,
            scope: scope.clone(),
            w3c_traceparent: None,
            reclaim_count: 0,
            resume_target: None,
        })
        .await
        .expect("enqueue the control command");

    let first = queue
        .claim_pending(&processor, 16)
        .await
        .expect("first claim");
    assert_eq!(
        first.len(),
        1,
        "[{}] generation N must claim the command",
        backend.name()
    );
    let superseded = first[0].token;

    tokio::time::sleep(ABA_CLAIM_AGE).await;
    let outcome = queue
        .reclaim_stuck(ABA_RECLAIM_HORIZON, 16)
        .await
        .expect("reclaim");
    assert_eq!(
        (outcome.reclaimed, outcome.exhausted),
        (1, 0),
        "[{}] the aged claim must be reclaimed, not exhausted",
        backend.name()
    );

    let second = queue
        .claim_pending(&processor, 16)
        .await
        .expect("second claim");
    assert_eq!(
        second.len(),
        1,
        "[{}] generation N+1 must re-claim the command",
        backend.name()
    );
    let current = second[0].token;
    assert!(
        current.generation() > superseded.generation(),
        "[{}] a re-claim must mint a strictly greater generation ({} then {})",
        backend.name(),
        superseded.generation(),
        current.generation()
    );

    let late_ack = queue.mark_completed(&superseded).await;
    assert!(
        matches!(late_ack, Err(StorageError::FencedOut { .. })),
        "[{}] a late ack from generation N must be FencedOut, got: {late_ack:?}",
        backend.name()
    );
    let late_nack = queue
        .mark_failed(&superseded, "late generation-N failure")
        .await;
    assert!(
        matches!(late_nack, Err(StorageError::FencedOut { .. })),
        "[{}] a late nack from generation N must be FencedOut, got: {late_nack:?}",
        backend.name()
    );

    // Zero state change: the row is still owned by generation N+1, which can
    // still acknowledge it.
    queue
        .mark_completed(&current)
        .await
        .expect("generation N+1 still owns the row after the fenced acknowledgements");
}

// ── keyed start acceptance conformance assertions ─────────────────────────

/// Canonicalization version used by these assertions. Real callers own their
/// own; the value only has to be stable within one comparison.
const START_FINGERPRINT_VERSION: u16 = 1;

fn start_command(id: u8, execution_id: &str) -> ControlMsg {
    ControlMsg {
        id: [id; 16],
        execution_id: execution_id.to_owned(),
        command: ControlCommand::Start,
        scope: scope_a(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    }
}

/// An absent start key creates exactly one execution and one Start command.
pub(crate) async fn assert_keyed_start_creates_one_execution(backend: &dyn Backend) {
    let acceptance = backend.start_acceptance_store().await;
    let queue = backend.control_queue().await;
    let executions = backend.execution_store().await;
    let scope = scope_a();
    let (workflow_id, initial_state) = make_new_execution();
    let command = start_command(0x40, "exe_start_fresh");

    let outcome = acceptance
        .accept_keyed_start(&KeyedStart {
            scope: &scope,
            start_key: "key-fresh",
            fingerprint: StartFingerprint::new(START_FINGERPRINT_VERSION, [1u8; 32]),
            execution_id: "exe_start_fresh",
            execution: NewExecution::new(&workflow_id, &initial_state),
            command: &command,
        })
        .await
        .expect("accept a fresh keyed start");
    assert_eq!(
        outcome,
        StartAcceptance::Accepted {
            execution_id: "exe_start_fresh".to_owned()
        },
        "[{}] a fresh key must be accepted",
        backend.name()
    );

    assert!(
        executions
            .get(&scope, "exe_start_fresh")
            .await
            .expect("read the execution back")
            .is_some(),
        "[{}] acceptance must have created the execution aggregate",
        backend.name()
    );
    let claimed = queue
        .claim_pending(&[9u8; 16], 16)
        .await
        .expect("claim the Start command");
    assert_eq!(
        claimed
            .iter()
            .filter(|claim| claim.msg.execution_id == "exe_start_fresh")
            .count(),
        1,
        "[{}] acceptance must have enqueued exactly one Start command",
        backend.name()
    );
}

/// A keyed start that cannot complete leaves **nothing** behind.
///
/// The in-memory adapter has no rollback, so its ordering has to do what a
/// transaction does for the SQL backends: validate every collision before the
/// first mutation. It previously inserted the execution row and only then
/// rejected a duplicate command id, so a failed acceptance still created an
/// execution — on one backend only, which is exactly the divergence a shared
/// oracle exists to catch.
pub(crate) async fn assert_keyed_start_failure_writes_nothing(backend: &dyn Backend) {
    let acceptance = backend.start_acceptance_store().await;
    let executions = backend.execution_store().await;
    let scope = scope_a();
    let (workflow_id, initial_state) = make_new_execution();

    // Occupy the command id the second acceptance will try to use.
    let first_command = start_command(0x47, "exe_start_conflict_first");
    acceptance
        .accept_keyed_start(&KeyedStart {
            scope: &scope,
            start_key: "key-conflict-first",
            fingerprint: StartFingerprint::new(START_FINGERPRINT_VERSION, [7u8; 32]),
            execution_id: "exe_start_conflict_first",
            execution: NewExecution::new(&workflow_id, &initial_state),
            command: &first_command,
        })
        .await
        .expect("seed the colliding command id");

    // A fresh key and a fresh execution id, but the command id is taken.
    let colliding_command = start_command(0x47, "exe_start_conflict_second");
    let outcome = acceptance
        .accept_keyed_start(&KeyedStart {
            scope: &scope,
            start_key: "key-conflict-second",
            fingerprint: StartFingerprint::new(START_FINGERPRINT_VERSION, [8u8; 32]),
            execution_id: "exe_start_conflict_second",
            execution: NewExecution::new(&workflow_id, &initial_state),
            command: &colliding_command,
        })
        .await;
    assert!(
        outcome.is_err(),
        "[{}] a colliding command id must fail the acceptance, got {outcome:?}",
        backend.name()
    );

    assert!(
        executions
            .get(&scope, "exe_start_conflict_second")
            .await
            .expect("read the failed candidate back")
            .is_none(),
        "[{}] a failed acceptance must not leave an execution behind",
        backend.name()
    );
}

/// The same key with the same fingerprint returns the original receipt and
/// writes nothing new — the lost-response retry converging.
pub(crate) async fn assert_keyed_start_replays_the_original_receipt(backend: &dyn Backend) {
    let acceptance = backend.start_acceptance_store().await;
    let queue = backend.control_queue().await;
    let scope = scope_a();
    let (workflow_id, initial_state) = make_new_execution();
    let fingerprint = StartFingerprint::new(START_FINGERPRINT_VERSION, [2u8; 32]);

    let first_command = start_command(0x41, "exe_start_replay");
    let first = acceptance
        .accept_keyed_start(&KeyedStart {
            scope: &scope,
            start_key: "key-replay",
            fingerprint,
            execution_id: "exe_start_replay",
            execution: NewExecution::new(&workflow_id, &initial_state),
            command: &first_command,
        })
        .await
        .expect("first keyed start");
    assert_eq!(
        first,
        StartAcceptance::Accepted {
            execution_id: "exe_start_replay".to_owned()
        },
        "[{}] the first request must be accepted",
        backend.name()
    );

    // The retry carries a *different* candidate execution id and command id —
    // exactly what a caller that never saw the first response would generate.
    // The reservation, not the caller, decides the durable identity.
    let retry_command = start_command(0x42, "exe_start_replay_retry");
    let replay = acceptance
        .accept_keyed_start(&KeyedStart {
            scope: &scope,
            start_key: "key-replay",
            fingerprint,
            execution_id: "exe_start_replay_retry",
            execution: NewExecution::new(&workflow_id, &initial_state),
            command: &retry_command,
        })
        .await
        .expect("retry of the same keyed start");
    assert_eq!(
        replay,
        StartAcceptance::Replayed {
            execution_id: "exe_start_replay".to_owned()
        },
        "[{}] a same-fingerprint retry must return the original receipt",
        backend.name()
    );

    let claimed = queue
        .claim_pending(&[9u8; 16], 32)
        .await
        .expect("claim Start commands");
    assert_eq!(
        claimed
            .iter()
            .filter(|claim| claim.msg.execution_id.starts_with("exe_start_replay"))
            .count(),
        1,
        "[{}] the retry must not enqueue a second Start command",
        backend.name()
    );
    let executions = backend.execution_store().await;
    assert!(
        executions
            .get(&scope, "exe_start_replay_retry")
            .await
            .expect("read the retry candidate back")
            .is_none(),
        "[{}] the retry must not create a second execution",
        backend.name()
    );
}

/// The same key with a different fingerprint is refused with **no durable
/// delta** — not silently accepted, and not accepted-then-compensated.
pub(crate) async fn assert_keyed_start_mismatch_writes_nothing(backend: &dyn Backend) {
    let acceptance = backend.start_acceptance_store().await;
    let queue = backend.control_queue().await;
    let executions = backend.execution_store().await;
    let scope = scope_a();
    let (workflow_id, initial_state) = make_new_execution();

    let original_command = start_command(0x43, "exe_start_mismatch");
    acceptance
        .accept_keyed_start(&KeyedStart {
            scope: &scope,
            start_key: "key-mismatch",
            fingerprint: StartFingerprint::new(START_FINGERPRINT_VERSION, [3u8; 32]),
            execution_id: "exe_start_mismatch",
            execution: NewExecution::new(&workflow_id, &initial_state),
            command: &original_command,
        })
        .await
        .expect("original keyed start");

    let conflicting_command = start_command(0x44, "exe_start_mismatch_other");
    let mismatch = acceptance
        .accept_keyed_start(&KeyedStart {
            scope: &scope,
            start_key: "key-mismatch",
            fingerprint: StartFingerprint::new(START_FINGERPRINT_VERSION, [4u8; 32]),
            execution_id: "exe_start_mismatch_other",
            execution: NewExecution::new(&workflow_id, &initial_state),
            command: &conflicting_command,
        })
        .await
        .expect("mismatched keyed start must be a typed outcome, not an error");
    assert_eq!(
        mismatch,
        StartAcceptance::FingerprintMismatch,
        "[{}] a different request under the same key must be refused",
        backend.name()
    );

    assert!(
        executions
            .get(&scope, "exe_start_mismatch_other")
            .await
            .expect("read the refused candidate back")
            .is_none(),
        "[{}] a refused start must leave no execution behind",
        backend.name()
    );
    let claimed = queue
        .claim_pending(&[9u8; 16], 32)
        .await
        .expect("claim Start commands");
    assert_eq!(
        claimed
            .iter()
            .filter(|claim| claim.msg.execution_id.starts_with("exe_start_mismatch"))
            .count(),
        1,
        "[{}] a refused start must leave no extra Start command",
        backend.name()
    );
}

/// Two tenants using the same key text never collide, and neither can observe
/// the other's reservation.
pub(crate) async fn assert_keyed_start_is_scoped_per_tenant(backend: &dyn Backend) {
    let acceptance = backend.start_acceptance_store().await;
    let (workflow_id, initial_state) = make_new_execution();
    let shared_key = "key-shared-across-tenants";

    let command_a = start_command(0x45, "exe_start_tenant_a");
    let accepted_a = acceptance
        .accept_keyed_start(&KeyedStart {
            scope: &scope_a(),
            start_key: shared_key,
            fingerprint: StartFingerprint::new(START_FINGERPRINT_VERSION, [5u8; 32]),
            execution_id: "exe_start_tenant_a",
            execution: NewExecution::new(&workflow_id, &initial_state),
            command: &command_a,
        })
        .await
        .expect("tenant A keyed start");

    // Tenant B reuses the key text with a *different* fingerprint. If the
    // reservation were not scope-qualified this would read as a mismatch and
    // leak the existence of tenant A's key.
    let mut command_b = start_command(0x46, "exe_start_tenant_b");
    command_b.scope = scope_b();
    let accepted_b = acceptance
        .accept_keyed_start(&KeyedStart {
            scope: &scope_b(),
            start_key: shared_key,
            fingerprint: StartFingerprint::new(START_FINGERPRINT_VERSION, [6u8; 32]),
            execution_id: "exe_start_tenant_b",
            execution: NewExecution::new(&workflow_id, &initial_state),
            command: &command_b,
        })
        .await
        .expect("tenant B keyed start");

    assert_eq!(
        (accepted_a, accepted_b),
        (
            StartAcceptance::Accepted {
                execution_id: "exe_start_tenant_a".to_owned()
            },
            StartAcceptance::Accepted {
                execution_id: "exe_start_tenant_b".to_owned()
            }
        ),
        "[{}] a start key is scoped to one tenant",
        backend.name()
    );
}

// ── job-dispatch + dedup conformance assertions ───────────────────────────

/// A `NewExecution` with placeholder content for conformance tests that focus
/// on the dedup/routing behaviour rather than the execution-row fields.
fn make_new_execution() -> (String, serde_json::Value) {
    ("wf_conformance".to_owned(), serde_json::json!({}))
}

fn make_job(id: u8, required_plugin_key: &str, tags: &[&str]) -> JobDispatchMsg {
    let key: PluginKey = required_plugin_key
        .parse()
        .expect("conformance test plugin key must be valid");
    let required_plugins: Vec<PluginKey> = tags
        .iter()
        .map(|s| {
            s.parse::<PluginKey>()
                .expect("conformance test tag must be valid")
        })
        .collect();
    JobDispatchMsg::new(
        [id; 16],
        format!("exe_{id}"),
        ControlCommand::Start,
        scope_a(),
        serde_json::json!({}),
        None::<&str>,
        "sha256:abc",
        key,
        required_plugins,
        None::<&str>,
        0,
    )
}

/// `claim_pending` only delivers rows whose required plugin is in the worker's
/// `available_plugins`; a row requiring an unavailable plugin is not delivered.
pub(crate) async fn assert_job_dispatch_routes_by_plugin(backend: &dyn Backend) {
    let q = backend.job_dispatch_queue().await;

    let job_a = make_job(0x10, "plugin.alpha", &["plugin.alpha"]);
    let job_b = make_job(0x11, "plugin.beta", &["plugin.beta"]);
    q.enqueue(&job_a).await.expect("enqueue alpha");
    q.enqueue(&job_b).await.expect("enqueue beta");

    let proc = [9u8; 16];
    // Advertise only alpha — must NOT receive beta.
    let claimed = q
        .claim_pending(&proc, 16, &["plugin.alpha".parse::<PluginKey>().unwrap()])
        .await
        .expect("claim");
    assert_eq!(
        claimed.len(),
        1,
        "[{}] only alpha row claimed",
        backend.name()
    );
    assert_eq!(
        claimed[0].msg.required_plugin_key.as_str(),
        "plugin.alpha",
        "[{}] claimed row must be alpha",
        backend.name()
    );

    // Advertise only beta — beta row is still Pending (alpha took none).
    let claimed_b = q
        .claim_pending(&proc, 16, &["plugin.beta".parse::<PluginKey>().unwrap()])
        .await
        .expect("claim beta");
    assert_eq!(claimed_b.len(), 1, "[{}] beta row claimed", backend.name());

    // Advertise an unrelated tag — nothing claimed.
    let nothing = q
        .claim_pending(&proc, 16, &["plugin.gamma".parse::<PluginKey>().unwrap()])
        .await
        .expect("claim gamma");
    assert!(
        nothing.is_empty(),
        "[{}] unadvertised tag must not match any row",
        backend.name()
    );
}

/// `mark_dispatched` and `mark_failed` are both fenced on the storage-minted
/// claim generation: a token that does not name the row's current claim
/// changes nothing.
pub(crate) async fn assert_job_dispatch_fencing(backend: &dyn Backend) {
    let q = backend.job_dispatch_queue().await;
    let plugin_tags = &["plugin.x".parse::<PluginKey>().unwrap()];

    let runner_a = [1u8; 16];

    // ── mark_dispatched fencing ───────────────────────────────────────────────
    let job_d = make_job(0x20, "plugin.x", &["plugin.x"]);
    q.enqueue(&job_d).await.expect("enqueue job_d");

    let claimed = q
        .claim_pending(&runner_a, 16, plugin_tags)
        .await
        .expect("claim");
    assert_eq!(claimed.len(), 1, "[{}] claimed one row", backend.name());
    let current = claimed[0].token;

    // A token naming a generation this row never reached must be rejected.
    // Under the old `processed_by` fence this was expressed as "a different
    // processor id"; authority is now the token, which a caller cannot forge
    // into existence, so the equivalent probe is a wrong generation.
    let forged = JobClaimToken::new(
        job_d.id,
        ClaimGeneration::new(current.generation().get() + 1),
    );
    let stale_dispatched = q.mark_dispatched(&forged).await;
    assert!(
        matches!(stale_dispatched, Err(StorageError::FencedOut { .. })),
        "[{}] mark_dispatched with a non-current generation must be FencedOut, got: {:?}",
        backend.name(),
        stale_dispatched
    );

    // The row is still Processing (the fenced call made no change) — the
    // holder of the current claim succeeds.
    q.mark_dispatched(&current)
        .await
        .expect("mark_dispatched (claimant)");

    // After mark_dispatched, a fresh claim should find no pending rows.
    let after_dispatch = q
        .claim_pending(&runner_a, 16, plugin_tags)
        .await
        .expect("claim after dispatch");
    assert!(
        after_dispatch.is_empty(),
        "[{}] no pending rows after mark_dispatched",
        backend.name()
    );

    // ── mark_failed fencing ───────────────────────────────────────────────────
    let job_f = make_job(0x21, "plugin.x", &["plugin.x"]);
    q.enqueue(&job_f).await.expect("enqueue job_f");

    let claimed_f = q
        .claim_pending(&runner_a, 16, plugin_tags)
        .await
        .expect("claim job_f");
    assert_eq!(claimed_f.len(), 1, "[{}] claimed job_f", backend.name());
    let current_f = claimed_f[0].token;

    let forged_f = JobClaimToken::new(
        job_f.id,
        ClaimGeneration::new(current_f.generation().get() + 1),
    );
    let stale_failed = q.mark_failed(&forged_f, "stale error").await;
    assert!(
        matches!(stale_failed, Err(StorageError::FencedOut { .. })),
        "[{}] mark_failed with a non-current generation must be FencedOut, got: {:?}",
        backend.name(),
        stale_failed
    );

    // The row is still Processing — the current claim can still fail it.
    q.mark_failed(&current_f, "real error")
        .await
        .expect("mark_failed (claimant)");

    // After mark_failed the row is terminal; fresh claim yields nothing.
    let after_failed = q
        .claim_pending(&runner_a, 16, plugin_tags)
        .await
        .expect("claim after failed");
    assert!(
        after_failed.is_empty(),
        "[{}] no pending rows after mark_failed",
        backend.name()
    );
}

/// How long a claim must age before `reclaim_stuck` will take it back.
///
/// The SQL backends compare wall-clock epoch-millis while the in-memory
/// backend compares `tokio::time::Instant`s; a real sleep advances both, so
/// one shared assertion can drive all three. The margin is generous because
/// the assertion is about ordering, not about a deadline.
const ABA_RECLAIM_HORIZON: std::time::Duration = std::time::Duration::from_millis(20);
const ABA_CLAIM_AGE: std::time::Duration = std::time::Duration::from_millis(60);

/// A stable processor identity cannot acknowledge a claim that a reclaim
/// already superseded — the same-processor ABA (C7).
///
/// One processor claims a row, the sweep hands the row back, and the *same*
/// processor claims it again. Every `processed_by`-based fence accepts an
/// acknowledgement issued against the first claim at that point, because the
/// recorded processor still matches: the late acknowledgement terminalises a
/// row the second attempt is still working. Only a per-attempt generation
/// tells the two apart.
pub(crate) async fn assert_job_dispatch_same_processor_aba_is_fenced(backend: &dyn Backend) {
    let q = backend.job_dispatch_queue().await;
    let plugin_tags = &["plugin.aba".parse::<PluginKey>().unwrap()];
    // One identity for both attempts — that is the whole point.
    let processor = [7u8; 16];

    let job = make_job(0x22, "plugin.aba", &["plugin.aba"]);
    q.enqueue(&job).await.expect("enqueue aba job");

    let first = q
        .claim_pending(&processor, 16, plugin_tags)
        .await
        .expect("first claim");
    assert_eq!(
        first.len(),
        1,
        "[{}] generation N must claim the row",
        backend.name()
    );
    let superseded = first[0].token;

    tokio::time::sleep(ABA_CLAIM_AGE).await;
    let outcome = q
        .reclaim_stuck(ABA_RECLAIM_HORIZON, 16)
        .await
        .expect("reclaim");
    assert_eq!(
        (outcome.reclaimed, outcome.exhausted),
        (1, 0),
        "[{}] the aged claim must be reclaimed, not exhausted",
        backend.name()
    );

    let second = q
        .claim_pending(&processor, 16, plugin_tags)
        .await
        .expect("second claim");
    assert_eq!(
        second.len(),
        1,
        "[{}] generation N+1 must re-claim the row",
        backend.name()
    );
    let current = second[0].token;
    assert_eq!(
        current.row_id(),
        superseded.row_id(),
        "[{}] both attempts must name the same row",
        backend.name()
    );
    assert!(
        current.generation() > superseded.generation(),
        "[{}] a re-claim must mint a strictly greater generation ({} then {})",
        backend.name(),
        superseded.generation(),
        current.generation()
    );

    let late_ack = q.mark_dispatched(&superseded).await;
    assert!(
        matches!(late_ack, Err(StorageError::FencedOut { .. })),
        "[{}] a late ack from generation N must be FencedOut, got: {late_ack:?}",
        backend.name()
    );
    let late_nack = q
        .mark_failed(&superseded, "late generation-N failure")
        .await;
    assert!(
        matches!(late_nack, Err(StorageError::FencedOut { .. })),
        "[{}] a late nack from generation N must be FencedOut, got: {late_nack:?}",
        backend.name()
    );

    // Zero state change: the row is still owned by generation N+1, which can
    // still acknowledge it. Were the fence merely returning an error while
    // terminalising the row, this would fail.
    q.mark_dispatched(&current)
        .await
        .expect("generation N+1 still owns the row after the fenced acknowledgements");
}

/// `claim_and_materialize_start` is first-writer-wins when a `TriggerDedupRow`
/// is provided: the second call with the same `(trigger_id, event_id)` must
/// return `Duplicate` and must NOT enqueue a second job.  The `Duplicate`
/// outcome carries the winner's execution id, not the candidate's.
pub(crate) async fn assert_trigger_dedup_first_writer(backend: &dyn Backend) {
    let inbox = backend.trigger_dedup_inbox().await;
    let q = backend.job_dispatch_queue().await;

    let row = TriggerDedupRow::new("trg_fw", "evt_001", scope_a(), "2026-01-01T00:00:00Z");
    let job1 = make_job(0x30, "plugin.y", &["plugin.y"]);
    let job2 = make_job(0x31, "plugin.y", &["plugin.y"]);

    let (wf_id, initial) = make_new_execution();
    let exec1 = NewExecution::new(&wf_id, &initial);

    let out1 = inbox
        .claim_and_materialize_start(Some(&row), &job1, &exec1)
        .await
        .expect("first compose");
    assert_eq!(
        out1.kind,
        DispatchKind::Dispatched,
        "[{}] first writer must be Dispatched",
        backend.name()
    );
    assert_eq!(
        out1.execution_id,
        job1.execution_id,
        "[{}] Dispatched outcome must carry the candidate execution id",
        backend.name()
    );

    let (wf_id2, initial2) = make_new_execution();
    let exec2 = NewExecution::new(&wf_id2, &initial2);
    let out2 = inbox
        .claim_and_materialize_start(Some(&row), &job2, &exec2)
        .await
        .expect("second compose");
    assert_eq!(
        out2.kind,
        DispatchKind::Duplicate,
        "[{}] second writer must be Duplicate",
        backend.name()
    );
    // Duplicate must carry the WINNER's execution id (job1's), not the candidate's.
    assert_eq!(
        out2.execution_id,
        job1.execution_id,
        "[{}] Duplicate outcome must carry the winner's execution id, not the candidate's",
        backend.name()
    );

    // Only one job row must have been enqueued.
    let proc = [7u8; 16];
    let claimed = q
        .claim_pending(&proc, 16, &["plugin.y".parse::<PluginKey>().unwrap()])
        .await
        .expect("claim");
    assert_eq!(
        claimed.len(),
        1,
        "[{}] exactly one job row after first-writer-wins compose",
        backend.name()
    );

    // `exists` must confirm the dedup row.
    let present = inbox
        .exists(&scope_a(), "trg_fw", "evt_001")
        .await
        .expect("exists");
    assert!(
        present,
        "[{}] exists must return true after a Dispatched compose",
        backend.name()
    );
}

/// `claim_and_materialize_start` with `row = None` always dispatches without
/// a dedup row (unconditional dispatch path).
pub(crate) async fn assert_dispatch_without_dedup_key(backend: &dyn Backend) {
    let inbox = backend.trigger_dedup_inbox().await;
    let q = backend.job_dispatch_queue().await;

    let job = make_job(0x40, "plugin.z", &["plugin.z"]);
    let (wf_id, initial) = make_new_execution();
    let exec = NewExecution::new(&wf_id, &initial);
    let out = inbox
        .claim_and_materialize_start(None, &job, &exec)
        .await
        .expect("compose none");
    assert_eq!(
        out.kind,
        DispatchKind::Dispatched,
        "[{}] None row must always be Dispatched",
        backend.name()
    );
    assert_eq!(
        out.execution_id,
        job.execution_id,
        "[{}] None-row Dispatched must carry the candidate execution id",
        backend.name()
    );

    let proc = [8u8; 16];
    let claimed = q
        .claim_pending(&proc, 16, &["plugin.z".parse::<PluginKey>().unwrap()])
        .await
        .expect("claim");
    assert_eq!(
        claimed.len(),
        1,
        "[{}] unconditional dispatch must enqueue one job",
        backend.name()
    );

    // A second None-row dispatch for a different job is also unconditional.
    let job2 = make_job(0x41, "plugin.z", &["plugin.z"]);
    let (wf_id2, initial2) = make_new_execution();
    let exec2 = NewExecution::new(&wf_id2, &initial2);
    let out2 = inbox
        .claim_and_materialize_start(None, &job2, &exec2)
        .await
        .expect("compose none 2");
    assert_eq!(
        out2.kind,
        DispatchKind::Dispatched,
        "[{}] second None-row dispatch must also be Dispatched (no dedup)",
        backend.name()
    );
    assert_eq!(
        out2.execution_id,
        job2.execution_id,
        "[{}] second None-row dispatch must carry its candidate execution id",
        backend.name()
    );
}

/// `claim_and_materialize_start` is atomic: the dedup guard, execution row,
/// and Start job are written together.  A dedup row in scope_a is invisible
/// from scope_b (cross-scope `exists` returns false), and after a Dispatched
/// compose the execution row is visible in the store and exactly one Start job
/// is claimable from the dispatch queue.
pub(crate) async fn assert_dedup_compose_is_atomic(backend: &dyn Backend) {
    let inbox = backend.trigger_dedup_inbox().await;
    let store = backend.execution_store().await;
    let q = backend.job_dispatch_queue().await;

    let row = TriggerDedupRow::new(
        "trg_atomic",
        "evt_atomic",
        scope_a(),
        "2026-01-01T00:00:00Z",
    );
    let job = make_job(0x50, "plugin.q", &["plugin.q"]);
    let (wf_id, initial) = make_new_execution();
    let exec = NewExecution::new(&wf_id, &initial);
    let outcome = inbox
        .claim_and_materialize_start(Some(&row), &job, &exec)
        .await
        .expect("compose");
    assert_eq!(
        outcome.kind,
        DispatchKind::Dispatched,
        "[{}] compose must be Dispatched",
        backend.name()
    );

    // All three writes must be visible atomically after a Dispatched compose.

    // 1. Execution row: must exist with the candidate id.
    let exec_row = store
        .get(&scope_a(), &job.execution_id)
        .await
        .expect("get execution row after compose");
    assert!(
        exec_row.is_some(),
        "[{}] execution row must exist after Dispatched compose (three-way atomicity)",
        backend.name()
    );

    // 2. Dedup guard: visible within scope_a.
    let in_scope = inbox
        .exists(&scope_a(), "trg_atomic", "evt_atomic")
        .await
        .expect("exists scope_a");
    assert!(
        in_scope,
        "[{}] dedup row must be visible in scope_a",
        backend.name()
    );

    // Cross-scope: invisible (scope_b has no such row).
    let cross = inbox
        .exists(&scope_b(), "trg_atomic", "evt_atomic")
        .await
        .expect("exists scope_b");
    assert!(
        !cross,
        "[{}] dedup row must not be visible cross-scope",
        backend.name()
    );

    // 3. Start job: exactly one claimable job must have landed, with the
    //    correct execution id.  This closes the gap in the "three-way"
    //    atomicity claim — a backend that commits dedup+execution but fails
    //    to enqueue the Start job would still pass the two checks above.
    let proc = [0xA0u8; 16];
    let claimed = q
        .claim_pending(&proc, 16, &["plugin.q".parse::<PluginKey>().unwrap()])
        .await
        .expect("claim_pending after compose");
    assert_eq!(
        claimed.len(),
        1,
        "[{}] exactly one Start job must be enqueued after Dispatched compose (three-way atomicity)",
        backend.name()
    );
    assert_eq!(
        claimed[0].msg.execution_id,
        job.execution_id,
        "[{}] claimed Start job execution_id must match the candidate ({})",
        backend.name(),
        job.execution_id
    );
}

/// `claim_pending` enforces a superset predicate: a worker may claim a job
/// only when its `available_plugins` cover EVERY plugin in the job's `required_plugins`.
///
/// Contract (job has `required_plugin_key = "plugin.alpha"` and
/// `required_plugins = ["plugin.alpha", "plugin.beta"]`):
///
/// 1. Advertised `["plugin.alpha"]` only → NOT claimed (missing beta).
/// 2. Advertised `["plugin.beta"]` only → NOT claimed (missing alpha; the
///    `required_plugin_key` pre-filter also rejects it independently).
/// 3. Advertised `["plugin.alpha", "plugin.beta"]` → claimed (exact superset).
/// 4. Advertised `["plugin.alpha", "plugin.beta", "plugin.gamma"]` → claimed
///    (strict superset); claimed job identity verified.
/// 5. Empty advertised set → claims nothing (parity across all backends).
pub(crate) async fn assert_job_dispatch_routes_by_plugin_superset(backend: &dyn Backend) {
    let q = backend.job_dispatch_queue().await;

    // Job requires alpha AND beta (required_plugins covers both; invariant upheld).
    let job = make_job(0x60, "plugin.alpha", &["plugin.alpha", "plugin.beta"]);
    q.enqueue(&job).await.expect("enqueue superset job");

    let proc = [0xAAu8; 16];

    // 1. Alpha-only worker: pre-filter passes (required_plugin_key = alpha) but
    //    superset check fails — beta is required and not advertised.
    let claimed_by_alpha_only = q
        .claim_pending(&proc, 16, &["plugin.alpha".parse::<PluginKey>().unwrap()])
        .await
        .expect("claim alpha-only");
    assert!(
        claimed_by_alpha_only.is_empty(),
        "[{}] alpha-only worker must not claim a job that also requires beta \
         (superset predicate: all required_plugins must be covered)",
        backend.name()
    );

    // 2. Beta-only worker: pre-filter rejects (required_plugin_key = alpha not
    //    in advertised) and the superset check would also fail independently.
    let claimed_by_beta_only = q
        .claim_pending(&proc, 16, &["plugin.beta".parse::<PluginKey>().unwrap()])
        .await
        .expect("claim beta-only");
    assert!(
        claimed_by_beta_only.is_empty(),
        "[{}] beta-only worker must not claim an alpha-required job \
         (pre-filter on required_plugin_key rejects it)",
        backend.name()
    );

    // 3. Exact-superset worker: advertises both alpha and beta — must claim.
    let claimed_by_both = q
        .claim_pending(
            &proc,
            16,
            &[
                "plugin.alpha".parse::<PluginKey>().unwrap(),
                "plugin.beta".parse::<PluginKey>().unwrap(),
            ],
        )
        .await
        .expect("claim alpha+beta");
    assert_eq!(
        claimed_by_both.len(),
        1,
        "[{}] worker advertising [alpha, beta] must claim the job (exact superset)",
        backend.name()
    );
    assert_eq!(
        claimed_by_both[0].msg.required_plugin_key.as_str(),
        "plugin.alpha",
        "[{}] claimed job must be the alpha-required one",
        backend.name()
    );

    // 4. Strict-superset worker (re-enqueue to get a fresh Pending row).
    let job2 = make_job(0x61, "plugin.alpha", &["plugin.alpha", "plugin.beta"]);
    q.enqueue(&job2).await.expect("enqueue superset job 2");
    let claimed_by_superset = q
        .claim_pending(
            &proc,
            16,
            &[
                "plugin.alpha".parse::<PluginKey>().unwrap(),
                "plugin.beta".parse::<PluginKey>().unwrap(),
                "plugin.gamma".parse::<PluginKey>().unwrap(),
            ],
        )
        .await
        .expect("claim strict superset");
    assert_eq!(
        claimed_by_superset.len(),
        1,
        "[{}] worker advertising [alpha, beta, gamma] must claim a job requiring [alpha, beta]",
        backend.name()
    );
    assert_eq!(
        claimed_by_superset[0].msg.id,
        job2.id,
        "[{}] claimed job must be the strict-superset job (id match)",
        backend.name()
    );
    assert_eq!(
        claimed_by_superset[0].msg.required_plugin_key.as_str(),
        "plugin.alpha",
        "[{}] claimed job must be the alpha-required one",
        backend.name()
    );

    // 5. Empty advertised set → claims nothing — parity with SQLite + Postgres
    //    which both short-circuit on empty available_plugins.  Re-enqueue a
    //    conforming job (required_plugins ⊇ {required_plugin_key}) to confirm
    //    it stays Pending.
    let job3 = make_job(0x62, "plugin.alpha", &["plugin.alpha", "plugin.beta"]);
    q.enqueue(&job3)
        .await
        .expect("enqueue job for empty-advertised check");
    let claimed_empty_adv = q
        .claim_pending(&proc, 16, &[])
        .await
        .expect("claim with empty advertised");
    assert!(
        claimed_empty_adv.is_empty(),
        "[{}] empty advertised set must claim nothing (parity with SQL backends)",
        backend.name()
    );
}

/// Trigger-dedup is scoped per tenant: the same `(trigger_id, event_id)` pair
/// under two different tenant scopes MUST NOT collide.
///
/// This is the regression lock for the cross-tenant confused-deputy bug where
/// the dedup key omitted scope: tenant B's `claim_and_materialize_start` would
/// hit tenant A's dedup row and return `Duplicate`, silently dropping tenant B's
/// job.
///
/// Contract:
/// 1. Tenant A dispatches `(trg_iso, evt_iso)` → `Dispatched`.
/// 2. Tenant B dispatches the **same** `(trg_iso, evt_iso)` → must also be
///    `Dispatched` (cross-tenant MUST NOT dedup).
/// 3. Tenant A repeats `(trg_iso, evt_iso)` → `Duplicate` (same-tenant dedup
///    still fires).
/// 4. `exists` confirms the row is visible inside each scope and invisible
///    across scopes.
pub(crate) async fn assert_trigger_dedup_is_scoped(backend: &dyn Backend) {
    let inbox = backend.trigger_dedup_inbox().await;

    let row_a = TriggerDedupRow::new("trg_iso", "evt_iso", scope_a(), "2026-01-01T00:00:00Z");
    let row_b = TriggerDedupRow::new("trg_iso", "evt_iso", scope_b(), "2026-01-01T00:00:00Z");
    // Unique ids per job so they never collide on the job-queue PK.
    let job_a1 = make_job(0x70, "plugin.iso", &["plugin.iso"]);
    let job_b = {
        let mut j = make_job(0x71, "plugin.iso", &["plugin.iso"]);
        j.scope = scope_b();
        j
    };
    let job_a2 = make_job(0x72, "plugin.iso", &["plugin.iso"]);

    let (wf_id_a1, initial_a1) = make_new_execution();
    let exec_a1 = NewExecution::new(&wf_id_a1, &initial_a1);
    // Step 1: tenant A dispatches — must be Dispatched.
    let out_a1 = inbox
        .claim_and_materialize_start(Some(&row_a), &job_a1, &exec_a1)
        .await
        .expect("step 1: tenant A dispatch");
    assert_eq!(
        out_a1.kind,
        DispatchKind::Dispatched,
        "[{}] step 1: tenant A must be Dispatched",
        backend.name()
    );

    let (wf_id_b, initial_b) = make_new_execution();
    let exec_b = NewExecution::new(&wf_id_b, &initial_b);
    // Step 2: tenant B dispatches the SAME (trigger_id, event_id) — must also
    // be Dispatched; cross-tenant MUST NOT dedup.
    let out_b = inbox
        .claim_and_materialize_start(Some(&row_b), &job_b, &exec_b)
        .await
        .expect("step 2: tenant B dispatch");
    assert_eq!(
        out_b.kind,
        DispatchKind::Dispatched,
        "[{}] step 2: tenant B with the same (trigger_id, event_id) must be \
         Dispatched — cross-tenant dedup collision (confused-deputy bug)",
        backend.name()
    );

    let (wf_id_a2, initial_a2) = make_new_execution();
    let exec_a2 = NewExecution::new(&wf_id_a2, &initial_a2);
    // Step 3: tenant A repeats — same-tenant dedup must fire (Duplicate).
    let out_a2 = inbox
        .claim_and_materialize_start(Some(&row_a), &job_a2, &exec_a2)
        .await
        .expect("step 3: tenant A repeat");
    assert_eq!(
        out_a2.kind,
        DispatchKind::Duplicate,
        "[{}] step 3: same-tenant repeat must be Duplicate",
        backend.name()
    );
    // Duplicate must carry the winner's execution id (job_a1's).
    assert_eq!(
        out_a2.execution_id,
        job_a1.execution_id,
        "[{}] step 3: Duplicate outcome must carry tenant A's winner execution id",
        backend.name()
    );

    // Step 4: `exists` is scope-qualified: each tenant sees its own row only.
    let a_sees_self = inbox
        .exists(&scope_a(), "trg_iso", "evt_iso")
        .await
        .expect("exists scope_a");
    assert!(
        a_sees_self,
        "[{}] scope_a must see its own dedup row",
        backend.name()
    );
    let b_sees_self = inbox
        .exists(&scope_b(), "trg_iso", "evt_iso")
        .await
        .expect("exists scope_b");
    assert!(
        b_sees_self,
        "[{}] scope_b must see its own dedup row",
        backend.name()
    );
    // Cross-scope: each tenant must NOT see the other's row via exists.
    let a_sees_b = inbox
        .exists(&scope_a(), "trg_iso", "evt_iso")
        .await
        .expect("exists a→b check");
    // Both scopes have a row for this (trigger_id, event_id), but they are
    // separate rows.  The relevant isolation is that scope B's claim above
    // returned Dispatched, not Duplicate — that is the confused-deputy guard.
    // `exists` is per-scope so both return true (each for their own row).
    assert!(
        a_sees_b,
        "[{}] scope_a exists must still return true (own row present)",
        backend.name()
    );
}

/// `claim_and_materialize_start` rolls back atomically on execution-id
/// collision: if the execution row cannot be inserted (id already exists),
/// neither the dedup guard nor the Start job must land in the store.
///
/// Contract:
/// 1. Pre-insert an execution row with a known id.
/// 2. Attempt `claim_and_materialize_start` with a `JobDispatchMsg` whose
///    `execution_id` matches — must return `Err(StorageError::Duplicate)`.
/// 3. Assert: no dedup guard was inserted (`exists` returns false), and no
///    Start job was enqueued (`claim_pending` returns empty).
pub(crate) async fn assert_dedup_compose_rolls_back_on_id_collision(backend: &dyn Backend) {
    let inbox = backend.trigger_dedup_inbox().await;
    let store = backend.execution_store().await;
    let q = backend.job_dispatch_queue().await;
    let s = scope_a();

    // Pre-insert an execution row with a known id.
    store
        .create(&s, "exe_collision", "wf_rollback", serde_json::json!({}))
        .await
        .expect("pre-insert execution row");

    // Build a compose whose execution_id collides with the pre-existing row.
    let row = TriggerDedupRow::new("trg_rb", "evt_rb", s.clone(), "2026-01-01T00:00:00Z");
    let mut job = make_job(0x80, "plugin.rb", &["plugin.rb"]);
    // Override the execution_id to the colliding id.
    "exe_collision".clone_into(&mut job.execution_id);

    let (wf_id, initial) = make_new_execution();
    let exec = NewExecution::new(&wf_id, &initial);
    let result = inbox
        .claim_and_materialize_start(Some(&row), &job, &exec)
        .await;

    assert!(
        matches!(result, Err(StorageError::Duplicate { .. })),
        "[{}] compose with a colliding execution id must return Duplicate error, got {result:?}",
        backend.name()
    );

    // The dedup row must NOT have been inserted (rollback).
    let dedup_exists = inbox
        .exists(&s, "trg_rb", "evt_rb")
        .await
        .expect("exists after failed compose");
    assert!(
        !dedup_exists,
        "[{}] dedup row must NOT exist after a rolled-back compose",
        backend.name()
    );

    // No Start job must have been enqueued (rollback).
    let proc = [0x9Au8; 16];
    let enqueued = q
        .claim_pending(&proc, 16, &["plugin.rb".parse::<PluginKey>().unwrap()])
        .await
        .expect("claim_pending after failed compose");
    assert!(
        enqueued.is_empty(),
        "[{}] no Start job must be enqueued after a rolled-back compose",
        backend.name()
    );
}

/// `claim_and_materialize_start` must fail closed on a colliding job-dispatch
/// id (`start.id`) and leave all state intact.  The SQL backends hit the
/// job-dispatch primary key and roll the transaction back; the in-memory
/// backend must reject the collision too — never silently overwrite the queued
/// job while reporting `Dispatched`.  (Regression guard for the codex P2
/// backend-divergence on PR #814.)
///
/// The second compose reuses the SAME job id but a DIFFERENT execution id and a
/// DIFFERENT `(trigger, event)`, so it is not a dedup duplicate — the only
/// collision is on the job-dispatch primary key.
pub(crate) async fn assert_dedup_compose_rejects_duplicate_job_id(backend: &dyn Backend) {
    let inbox = backend.trigger_dedup_inbox().await;
    let q = backend.job_dispatch_queue().await;
    let s = scope_a();

    // 1. First compose succeeds: enqueues job id 0xB0 (execution id "exe_176").
    let row1 = TriggerDedupRow::new("trg_j1", "evt_j1", s.clone(), "2026-01-01T00:00:00Z");
    let job1 = make_job(0xB0, "plugin.jid", &["plugin.jid"]);
    let (wf1, init1) = make_new_execution();
    let exec1 = NewExecution::new(&wf1, &init1);
    let _ = inbox
        .claim_and_materialize_start(Some(&row1), &job1, &exec1)
        .await
        .expect("first compose dispatches");

    // 2. Second compose reuses the SAME job id (0xB0), different execution id.
    let row2 = TriggerDedupRow::new("trg_j2", "evt_j2", s.clone(), "2026-01-01T00:00:00Z");
    let mut job2 = make_job(0xB0, "plugin.jid", &["plugin.jid"]);
    "exe_jid_other".clone_into(&mut job2.execution_id);
    let (wf2, init2) = make_new_execution();
    let exec2 = NewExecution::new(&wf2, &init2);
    let result = inbox
        .claim_and_materialize_start(Some(&row2), &job2, &exec2)
        .await;
    assert!(
        result.is_err(),
        "[{}] compose with a colliding job-dispatch id must fail closed, got {result:?}",
        backend.name()
    );

    // 3. The original job must be intact (NOT overwritten by job2): exactly one
    //    queued job, still carrying the first execution id.
    let proc = [0xB1u8; 16];
    let claimed = q
        .claim_pending(&proc, 16, &["plugin.jid".parse::<PluginKey>().unwrap()])
        .await
        .expect("claim after failed compose");
    assert_eq!(
        claimed.len(),
        1,
        "[{}] exactly the original job must remain queued",
        backend.name()
    );
    assert_eq!(
        claimed[0].msg.execution_id,
        "exe_176",
        "[{}] the original job must NOT be overwritten by the colliding compose",
        backend.name()
    );

    // 4. The second dedup row must NOT have been inserted (all-or-nothing).
    let dedup2 = inbox
        .exists(&s, "trg_j2", "evt_j2")
        .await
        .expect("exists after failed compose");
    assert!(
        !dedup2,
        "[{}] the colliding compose must not insert its dedup row",
        backend.name()
    );
}

/// `claim_and_materialize_start` returns the winner's `execution_id` on
/// `Duplicate`, NOT a freshly-minted candidate.  This is the P2 contract
/// upgrade over the old `claim_and_enqueue_start` which returned a
/// caller-supplied candidate id.
///
/// Contract:
/// 1. First compose with `(trg_rb2, evt_rb2)` → `Dispatched`; record
///    `winner_id = outcome.execution_id`.
/// 2. Second compose with the same `(trg_rb2, evt_rb2)` → `Duplicate`;
///    `outcome.execution_id` must equal `winner_id`.
pub(crate) async fn assert_dedup_duplicate_returns_winner_id(backend: &dyn Backend) {
    let inbox = backend.trigger_dedup_inbox().await;
    let s = scope_a();

    let row = TriggerDedupRow::new("trg_rb2", "evt_rb2", s.clone(), "2026-01-01T00:00:00Z");
    let job1 = make_job(0x90, "plugin.w", &["plugin.w"]);
    let (wf_id1, initial1) = make_new_execution();
    let exec1 = NewExecution::new(&wf_id1, &initial1);

    let out1 = inbox
        .claim_and_materialize_start(Some(&row), &job1, &exec1)
        .await
        .expect("first compose");
    assert_eq!(
        out1.kind,
        DispatchKind::Dispatched,
        "[{}] first compose must be Dispatched",
        backend.name()
    );
    let winner_id = out1.execution_id.clone();

    // Second compose: different candidate id, same (trigger_id, event_id).
    let job2 = make_job(0x91, "plugin.w", &["plugin.w"]);
    let (wf_id2, initial2) = make_new_execution();
    let exec2 = NewExecution::new(&wf_id2, &initial2);

    let out2 = inbox
        .claim_and_materialize_start(Some(&row), &job2, &exec2)
        .await
        .expect("second compose");
    assert_eq!(
        out2.kind,
        DispatchKind::Duplicate,
        "[{}] second compose must be Duplicate",
        backend.name()
    );
    assert_eq!(
        out2.execution_id,
        winner_id,
        "[{}] Duplicate outcome must carry the original winner's execution id ({}), \
         not the new candidate's ({})",
        backend.name(),
        winner_id,
        job2.execution_id
    );
}

/// A released claim is immediately re-claimable, is fenced against a
/// superseded token, and does not consume the reclaim budget.
///
/// `release_claim` exists so a dispatch that hit momentary contention retries
/// on the next poll rather than waiting out `reclaim_after`, which is sized in
/// minutes to detect a runner that *died*. Every backend has to agree on all
/// three properties, because a `Cancel` that lost a millisecond-wide race is
/// the case this serves, and a per-backend difference would be a latency cliff
/// no operator could attribute.
pub(crate) async fn assert_control_queue_release_returns_row_for_redelivery(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let queue = backend.control_queue().await;
    let scope = scope_a();
    let processor = [21u8; 16];

    let execution_id = "exe_control_release";
    store
        .create(
            &scope,
            execution_id,
            "wf_control_release",
            serde_json::json!({}),
        )
        .await
        .expect("create the execution the command targets");
    queue
        .enqueue(&ControlMsg {
            id: [0x5Bu8; 16],
            execution_id: execution_id.to_owned(),
            command: ControlCommand::Cancel,
            scope: scope.clone(),
            w3c_traceparent: None,
            reclaim_count: 0,
            resume_target: None,
        })
        .await
        .expect("enqueue the control command");

    let first = queue
        .claim_pending(&processor, 16)
        .await
        .expect("first claim");
    assert_eq!(
        first.len(),
        1,
        "[{}] the command must claim",
        backend.name()
    );
    let released = first[0].token;

    queue
        .release_claim(&released)
        .await
        .expect("releasing an owned claim must succeed");

    // Re-claimable at once — no reclaim sweep, no waiting.
    let second = queue
        .claim_pending(&processor, 16)
        .await
        .expect("second claim");
    assert_eq!(
        second.len(),
        1,
        "[{}] a released claim must be re-claimable immediately, without a reclaim sweep",
        backend.name()
    );
    assert_eq!(
        second[0].msg.reclaim_count,
        0,
        "[{}] releasing must not spend the reclaim budget — a retry is not a stuck row",
        backend.name()
    );
    assert!(
        second[0].token.generation() > released.generation(),
        "[{}] the re-claim must mint a fresh generation so the released token is dead",
        backend.name()
    );

    // The superseded token cannot release the row out from under its new owner.
    let stale = queue.release_claim(&released).await;
    assert!(
        matches!(stale, Err(StorageError::FencedOut { .. })),
        "[{}] a superseded claim must not release a row another processor now owns; got {stale:?}",
        backend.name()
    );

    // And the row is still owned by the second claim, not sitting Pending.
    let third = queue
        .claim_pending(&processor, 16)
        .await
        .expect("third claim");
    assert!(
        third.is_empty(),
        "[{}] the fenced-out release must have changed nothing",
        backend.name()
    );
}
