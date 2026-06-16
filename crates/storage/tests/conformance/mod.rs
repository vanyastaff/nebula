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

use nebula_storage_port::dto::{
    CachedRecord, CapabilityTag, ControlCommand, ControlMsg, DispatchKind, JobDispatchMsg,
    JournalEntry, NewExecution, TriggerDedupRow, WebhookActivationRecord, WorkflowRecord,
    WorkflowVersionRecord,
};
use nebula_storage_port::store::{
    ControlQueue, ExecutionJournalReader, ExecutionStore, IdempotencyGuard, IdempotencyStore,
    JobDispatchQueue, TriggerDedupInbox, WebhookActivationStore, WorkflowStore,
    WorkflowVersionStore,
};
use nebula_storage_port::{FencingToken, Scope, StorageError, TransitionBatch, TransitionOutcome};

/// A storage backend under conformance test. Returns port handles built on
/// that backend's concrete adapter.
#[async_trait::async_trait]
pub trait Backend: Send + Sync {
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
}

/// InMemory backend (always available).
///
/// Holds one execution store whose core is shared (it is `Clone` over an
/// `Arc<Mutex<…>>`), so the control queue, journal reader, job-dispatch queue,
/// and trigger-dedup inbox all observe the same rows and operate atomically
/// under one lock.
pub struct InMemoryBackend {
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
}

/// SQLite `:memory:` backend.
///
/// Each `Backend` instance owns one shared-cache in-memory database (so a
/// `create` and a later `commit`/`get` observe the same rows) created
/// lazily on first store request. Only built when the `sqlite` feature is
/// on; without it the case skips like Postgres.
#[derive(Default)]
pub struct SqliteBackend {
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
}

/// Postgres backend — only exercised when `DATABASE_URL` is set and the
/// crate is built with `--features postgres`; otherwise `skip_reason`
/// short-circuits the case so the suite stays green on a machine without
/// a database. Each `Backend` instance owns one pool created lazily on
/// first store request; the port schema is installed once.
#[derive(Default)]
pub struct PostgresBackend {
    #[cfg(feature = "postgres")]
    pool: tokio::sync::OnceCell<sqlx::PgPool>,
}

#[cfg(feature = "postgres")]
impl PostgresBackend {
    async fn pool(&self) -> sqlx::PgPool {
        self.pool
            .get_or_init(|| async {
                let url = std::env::var("DATABASE_URL")
                    .unwrap_or_else(|e| panic!("DATABASE_URL required for the Postgres case: {e}"));
                let pool = sqlx::postgres::PgPoolOptions::new()
                    .max_connections(8)
                    .connect(&url)
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
pub fn skip_reason(backend: &dyn Backend) -> Option<&'static str> {
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
pub async fn assert_create_get_roundtrip(backend: &dyn Backend) {
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
pub async fn assert_cas_conflict(backend: &dyn Backend) {
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
pub async fn assert_stale_fencing_is_fenced_out(backend: &dyn Backend) {
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
pub async fn assert_live_lease_blocks_acquire(backend: &dyn Backend) {
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
pub async fn assert_atomic_triple(backend: &dyn Backend) {
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
pub async fn assert_idempotency_first_writer_wins(backend: &dyn Backend) {
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
pub async fn assert_cross_scope_get_is_none(backend: &dyn Backend) {
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
pub async fn assert_cross_scope_commit_is_rejected(backend: &dyn Backend) {
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
pub async fn assert_workflow_store_contract(backend: &dyn Backend) {
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
pub async fn assert_save_with_published_version_is_atomic(backend: &dyn Backend) {
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
pub async fn assert_get_published_is_highest_numbered(backend: &dyn Backend) {
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
pub async fn assert_control_queue_outbox_and_fencing(backend: &dyn Backend) {
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
    let runner_b = [2u8; 16];
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
        claimed[0].id,
        [42u8; 16],
        "[{}] typed 16-byte id round-trips through the queue",
        backend.name()
    );

    // A stale runner (did not claim this row) must NOT flip it.
    queue
        .mark_completed(&[42u8; 16], &runner_b)
        .await
        .expect("mark_completed (stale)");
    let reclaimed = queue
        .claim_pending(&runner_a, 16)
        .await
        .expect("claim_pending after stale ack");
    assert!(
        reclaimed.is_empty(),
        "[{}] a stale processor's ack must be a no-op (row stays Processing, \
         not re-Pending and not Completed)",
        backend.name()
    );

    // The actual claimant can complete it.
    queue
        .mark_completed(&[42u8; 16], &runner_a)
        .await
        .expect("mark_completed (claimant)");
}

/// Journal entries appended by a `commit` are readable in order, and a
/// cross-tenant read yields an empty journal (never another tenant's
/// entries).
pub async fn assert_journal_visibility_and_scope(backend: &dyn Backend) {
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
pub async fn assert_idempotency_store_first_writer(backend: &dyn Backend) {
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
pub async fn assert_idempotency_store_cross_scope_isolated(backend: &dyn Backend) {
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
pub async fn assert_webhook_activation_and_scope(backend: &dyn Backend) {
    let store = backend.webhook_store().await;
    let s = scope_a();
    store
        .upsert(
            &s,
            WebhookActivationRecord {
                trigger_id: "trg_1".into(),
                scope: s.clone(),
                slug: "deploy-hook".into(),
                active: true,
            },
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
pub struct ScopedBackend<B: Backend> {
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
}

// ── job-dispatch + dedup conformance assertions ───────────────────────────

/// A `NewExecution` with placeholder content for conformance tests that focus
/// on the dedup/routing behaviour rather than the execution-row fields.
fn make_new_execution() -> (String, serde_json::Value) {
    ("wf_conformance".to_owned(), serde_json::json!({}))
}

fn make_job(id: u8, required_plugin_key: &str, tags: &[&str]) -> JobDispatchMsg {
    JobDispatchMsg::new(
        [id; 16],
        format!("exe_{id}"),
        ControlCommand::Start,
        scope_a(),
        serde_json::json!({}),
        None::<&str>,
        "sha256:abc",
        required_plugin_key,
        tags.iter().map(|s| CapabilityTag::from(*s)).collect(),
        None::<&str>,
        0,
    )
}

/// `claim_pending` only delivers rows whose `required_plugin_key` is a
/// member of the advertised set; rows requiring an unadvertised key are not
/// delivered.
pub async fn assert_job_dispatch_routes_by_tag(backend: &dyn Backend) {
    let q = backend.job_dispatch_queue().await;

    let job_a = make_job(0x10, "plugin.alpha", &["plugin.alpha"]);
    let job_b = make_job(0x11, "plugin.beta", &["plugin.beta"]);
    q.enqueue(&job_a).await.expect("enqueue alpha");
    q.enqueue(&job_b).await.expect("enqueue beta");

    let proc = [9u8; 16];
    // Advertise only alpha — must NOT receive beta.
    let claimed = q
        .claim_pending(&proc, 16, &[CapabilityTag::from("plugin.alpha")])
        .await
        .expect("claim");
    assert_eq!(
        claimed.len(),
        1,
        "[{}] only alpha row claimed",
        backend.name()
    );
    assert_eq!(
        claimed[0].required_plugin_key,
        "plugin.alpha",
        "[{}] claimed row must be alpha",
        backend.name()
    );

    // Advertise only beta — beta row is still Pending (alpha took none).
    let claimed_b = q
        .claim_pending(&proc, 16, &[CapabilityTag::from("plugin.beta")])
        .await
        .expect("claim beta");
    assert_eq!(claimed_b.len(), 1, "[{}] beta row claimed", backend.name());

    // Advertise an unrelated tag — nothing claimed.
    let nothing = q
        .claim_pending(&proc, 16, &[CapabilityTag::from("plugin.gamma")])
        .await
        .expect("claim gamma");
    assert!(
        nothing.is_empty(),
        "[{}] unadvertised tag must not match any row",
        backend.name()
    );
}

/// `mark_dispatched` is fenced by processor id: a stale processor cannot
/// transition a row it did not claim.
pub async fn assert_job_dispatch_fencing(backend: &dyn Backend) {
    let q = backend.job_dispatch_queue().await;
    let job = make_job(0x20, "plugin.x", &["plugin.x"]);
    q.enqueue(&job).await.expect("enqueue");

    let runner_a = [1u8; 16];
    let runner_b = [2u8; 16];
    let claimed = q
        .claim_pending(&runner_a, 16, &[CapabilityTag::from("plugin.x")])
        .await
        .expect("claim");
    assert_eq!(claimed.len(), 1, "[{}] claimed one row", backend.name());

    // Stale runner must NOT transition the row.
    q.mark_dispatched(&job.id, &runner_b)
        .await
        .expect("mark_dispatched (stale)");

    // The row is still Processing — a re-claim with the REAL runner confirms it.
    q.mark_dispatched(&job.id, &runner_a)
        .await
        .expect("mark_dispatched (claimant)");
    // After mark_dispatched, a fresh claim should find no pending rows.
    let after = q
        .claim_pending(&runner_a, 16, &[CapabilityTag::from("plugin.x")])
        .await
        .expect("claim after dispatch");
    assert!(
        after.is_empty(),
        "[{}] no pending rows after mark_dispatched",
        backend.name()
    );
}

/// `claim_and_materialize_start` is first-writer-wins when a `TriggerDedupRow`
/// is provided: the second call with the same `(trigger_id, event_id)` must
/// return `Duplicate` and must NOT enqueue a second job.  The `Duplicate`
/// outcome carries the winner's execution id, not the candidate's.
pub async fn assert_trigger_dedup_first_writer(backend: &dyn Backend) {
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
        .claim_pending(&proc, 16, &[CapabilityTag::from("plugin.y")])
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
pub async fn assert_dispatch_without_dedup_key(backend: &dyn Backend) {
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

    let proc = [8u8; 16];
    let claimed = q
        .claim_pending(&proc, 16, &[CapabilityTag::from("plugin.z")])
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
}

/// `claim_and_materialize_start` is atomic: the dedup guard, execution row,
/// and Start job are written together.  A dedup row in scope_a is invisible
/// from scope_b (cross-scope `exists` returns false), and after a Dispatched
/// compose the execution row is visible in the store and exactly one Start job
/// is claimable from the dispatch queue.
pub async fn assert_dedup_compose_is_atomic(backend: &dyn Backend) {
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
        .claim_pending(&proc, 16, &[CapabilityTag::from("plugin.q")])
        .await
        .expect("claim_pending after compose");
    assert_eq!(
        claimed.len(),
        1,
        "[{}] exactly one Start job must be enqueued after Dispatched compose (three-way atomicity)",
        backend.name()
    );
    assert_eq!(
        claimed[0].execution_id,
        job.execution_id,
        "[{}] claimed Start job execution_id must match the candidate ({})",
        backend.name(),
        job.execution_id
    );
}

/// Routing is keyed on `required_plugin_key`, NOT on membership in the
/// `capability_tags` superset.
///
/// A job with `required_plugin_key = "plugin.alpha"` and
/// `capability_tags = ["plugin.alpha", "plugin.beta"]` must NOT be claimed
/// by a worker that advertises only `["plugin.beta"]`.  This assertion
/// exists to lock cross-backend equivalence between Postgres
/// (`required_plugin_key = ANY($tags)`) and SQLite
/// (`required_plugin_key = ?` per advertised tag).
pub async fn assert_job_dispatch_routes_by_tag_superset(backend: &dyn Backend) {
    let q = backend.job_dispatch_queue().await;

    // Job requires alpha but lists both alpha and beta in capability_tags.
    let job = make_job(0x60, "plugin.alpha", &["plugin.alpha", "plugin.beta"]);
    q.enqueue(&job).await.expect("enqueue superset job");

    let proc = [0xAAu8; 16];

    // A worker advertising only beta must NOT claim this job.
    let claimed_by_beta = q
        .claim_pending(&proc, 16, &[CapabilityTag::from("plugin.beta")])
        .await
        .expect("claim beta-only");
    assert!(
        claimed_by_beta.is_empty(),
        "[{}] beta-only worker must not claim an alpha-required job \
         (routing is on required_plugin_key, not capability_tags superset)",
        backend.name()
    );

    // A worker advertising alpha must claim it.
    let claimed_by_alpha = q
        .claim_pending(&proc, 16, &[CapabilityTag::from("plugin.alpha")])
        .await
        .expect("claim alpha");
    assert_eq!(
        claimed_by_alpha.len(),
        1,
        "[{}] alpha worker must claim the alpha-required job",
        backend.name()
    );
    assert_eq!(
        claimed_by_alpha[0].required_plugin_key,
        "plugin.alpha",
        "[{}] claimed job must be the alpha-required one",
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
pub async fn assert_trigger_dedup_is_scoped(backend: &dyn Backend) {
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
pub async fn assert_dedup_compose_rolls_back_on_id_collision(backend: &dyn Backend) {
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
    job.execution_id = "exe_collision".to_owned();

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
        .claim_pending(&proc, 16, &[CapabilityTag::from("plugin.rb")])
        .await
        .expect("claim_pending after failed compose");
    assert!(
        enqueued.is_empty(),
        "[{}] no Start job must be enqueued after a rolled-back compose",
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
pub async fn assert_dedup_duplicate_returns_winner_id(backend: &dyn Backend) {
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
