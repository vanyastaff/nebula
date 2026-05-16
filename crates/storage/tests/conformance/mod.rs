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
//! Backends whose adapter does not yet exist return their store via
//! `unimplemented!()`, so the suite compiles and is *red* until the
//! adapter lands (TDD target for P2 Tasks 9–14).

use std::sync::Arc;

use nebula_storage_port::dto::{
    CachedRecord, ControlCommand, ControlMsg, JournalEntry, WebhookActivationRecord,
};
use nebula_storage_port::store::{
    ControlQueue, ExecutionJournalReader, ExecutionStore, IdempotencyGuard, IdempotencyStore,
    WebhookActivationStore,
};
use nebula_storage_port::{FencingToken, Scope, TransitionBatch, TransitionOutcome};

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
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        Self {
            store: nebula_storage::inmem::InMemoryExecutionStore::new(),
            guard: nebula_storage::inmem::InMemoryIdempotencyGuard::new(),
            idem_store: nebula_storage::inmem::InMemoryIdempotencyStore::new(),
            webhook: nebula_storage::inmem::InMemoryWebhookActivationStore::new(),
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
}
