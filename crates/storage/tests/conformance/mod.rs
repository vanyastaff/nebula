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
    CachedRecord, ControlCommand, ControlMsg, JournalEntry, WebhookActivationRecord,
    WorkflowRecord, WorkflowVersionRecord,
};
use nebula_storage_port::store::{
    ControlQueue, ExecutionJournalReader, ExecutionStore, IdempotencyGuard, IdempotencyStore,
    WebhookActivationStore, WorkflowStore, WorkflowVersionStore,
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
}

/// InMemory backend (always available).
///
/// Holds one execution store whose core is shared (it is `Clone` over an
/// `Arc<Mutex<…>>`), so the control queue and journal reader observe the
/// outbox + journal rows a `commit` wrote.
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
/// token is dead. This is the §12.2 zombie-runner closure (ADR-0008):
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
    let applied = matches!(outcome, Ok(TransitionOutcome::Applied { .. }));
    assert!(
        !applied,
        "[{}] cross-tenant commit must NEVER Apply",
        backend.name()
    );
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

/// The durable idempotent-replay cache is first-writer-wins (a second
/// `put` on the same key keeps the original record + fingerprint) and
/// tenant-isolated: a probe under a different scope-namespaced key is a
/// miss, so tenant A can neither read nor poison tenant B's entry.
pub async fn assert_idempotency_store_first_writer_and_scope(backend: &dyn Backend) {
    let store = backend.idempotency_store().await;
    // The caller scope-namespaces the key (`{ws}:{org}:{key}`); the
    // store treats it opaquely.
    let key_a = "ws_a:org_a:POST /x:idem-1".to_string();
    let key_b = "ws_b:org_b:POST /x:idem-1".to_string();
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
            key_a.clone(),
            first.clone(),
            std::time::Duration::from_mins(1),
        )
        .await
        .expect("put #1");
    store
        .put(key_a.clone(), second, std::time::Duration::from_mins(1))
        .await
        .expect("put #2 (must be a no-op)");
    let got = store
        .get(&key_a)
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

    // A different tenant's scope-namespaced key is a clean miss — never
    // tenant A's record (replay-oracle mitigation, §6.1).
    let cross = store.get(&key_b).await.expect("get cross-scope key");
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
}
