//! Threat-model conformance for the scoping decorators (spec §6.1).
//!
//! Each test wires **two** decorators — tenant A and tenant B — over the
//! *same* in-memory mock store, then drives the documented abuse case and
//! asserts the attacker tenant is denied. The mocks are intentionally
//! trivial `Arc<Mutex<HashMap>>` shims: the decorator is the unit under
//! test, not any real adapter. Cross-tenant denial must hold regardless of
//! the backend because the decorator substitutes the bound scope *before*
//! the backend ever sees the call.
//!
//! Abuse cases covered here (decorator-level):
//!
//! 1. Confused deputy / cross-tenant row access ⇒ `Ok(None)` (never the
//!    row, never an existence-leaking error), and a `commit` carrying the
//!    attacker's scope never Applies cross-tenant.
//! 2. Idempotency replay-oracle ⇒ tenant-namespaced keys; A cannot probe
//!    or poison B's dedup entry.
//! 3. Control-queue confused deputy ⇒ a Cancel enqueued by tenant A is
//!    stamped with A's scope, never B's.
//!
//! Abuse case 4 (credential scope-layer fail-closed + zeroize + pending
//! single-use cross-tenant replay) is covered by the credential
//! scope-layer re-home suite (Task 17), where the credential layer lives.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nebula_storage_port::dto::{CachedRecord, ControlCommand, ControlMsg, ExecutionRecord};
use nebula_storage_port::store::{ControlQueue, ExecutionStore, IdempotencyStore, ReclaimOutcome};
use nebula_storage_port::{FencingToken, Scope, StorageError, TransitionBatch, TransitionOutcome};
use nebula_tenancy::{ScopedControlQueue, ScopedExecutionStore, ScopedIdempotencyStore};

fn scope_a() -> Scope {
    Scope::new("ws_a", "org_a")
}

fn scope_b() -> Scope {
    Scope::new("ws_b", "org_b")
}

// ── Mock execution store ──────────────────────────────────────────────────
// Keyed by (workspace_id, org_id, id) so it enforces the same tenant
// predicate a real adapter's `WHERE workspace_id = ? AND org_id = ?`
// would. The decorator substitutes the bound scope before the key is
// formed, so an attacker's foreign scope simply produces a different key
// — a clean miss, never a leak.

type ExecKey = (String, String, String);

#[derive(Default)]
struct MockExecStore {
    rows: Mutex<HashMap<ExecKey, ExecutionRecord>>,
}

fn exec_key(scope: &Scope, id: &str) -> ExecKey {
    (
        scope.workspace_id.clone(),
        scope.org_id.clone(),
        id.to_string(),
    )
}

impl std::fmt::Debug for MockExecStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MockExecStore")
    }
}

#[async_trait::async_trait]
impl ExecutionStore for MockExecStore {
    async fn create(
        &self,
        scope: &Scope,
        id: &str,
        workflow_id: &str,
        initial_state: serde_json::Value,
    ) -> Result<(), StorageError> {
        let rec = ExecutionRecord {
            id: id.to_string(),
            workflow_id: workflow_id.to_string(),
            scope: scope.clone(),
            version: 0,
            status: "created".into(),
            state: initial_state,
            lease_holder: None,
            fencing: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        };
        self.rows
            .lock()
            .expect("mock lock")
            .insert(exec_key(scope, id), rec);
        Ok(())
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ExecutionRecord>, StorageError> {
        Ok(self
            .rows
            .lock()
            .expect("mock lock")
            .get(&exec_key(scope, id))
            .cloned())
    }

    async fn commit(&self, batch: TransitionBatch) -> Result<TransitionOutcome, StorageError> {
        let key = exec_key(batch.scope(), batch.execution_id());
        let mut rows = self.rows.lock().expect("mock lock");
        match rows.get_mut(&key) {
            Some(row) if row.version == batch.expected_version() => {
                row.version += 1;
                row.state = batch.new_state().clone();
                Ok(TransitionOutcome::Applied {
                    new_version: row.version,
                })
            },
            // No row for this (scope,id) — the CAS simply misses. A
            // cross-tenant commit lands here: NEVER Applied.
            _ => Ok(TransitionOutcome::VersionConflict { actual: 0 }),
        }
    }

    async fn acquire_lease(
        &self,
        _scope: &Scope,
        _id: &str,
        _holder: &str,
        _ttl: Duration,
    ) -> Result<Option<FencingToken>, StorageError> {
        Ok(Some(FencingToken::from_generation(0)))
    }

    async fn renew_lease(
        &self,
        _scope: &Scope,
        _id: &str,
        _token: FencingToken,
        _ttl: Duration,
    ) -> Result<bool, StorageError> {
        Ok(true)
    }

    async fn release_lease(
        &self,
        _scope: &Scope,
        _id: &str,
        _token: FencingToken,
    ) -> Result<bool, StorageError> {
        Ok(true)
    }

    async fn list_running(&self, _scope: &Scope) -> Result<Vec<String>, StorageError> {
        Ok(vec![])
    }

    async fn list_running_for_workflow(
        &self,
        _scope: &Scope,
        _workflow_id: &str,
    ) -> Result<Vec<String>, StorageError> {
        Ok(vec![])
    }

    async fn count(&self, _scope: &Scope, _workflow_id: Option<&str>) -> Result<u64, StorageError> {
        Ok(0)
    }
}

// ── Mock idempotency store ────────────────────────────────────────────────
// Keyed by `{ws}:{org}:{cache_key}` like the real backends: the store
// folds the scope in, and the decorator substitutes its bound scope before
// the call lands here, so two tenants' keyspaces are disjoint.

#[derive(Default)]
struct MockIdemStore {
    rows: Mutex<HashMap<String, CachedRecord>>,
}

impl std::fmt::Debug for MockIdemStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MockIdemStore")
    }
}

fn idem_key(scope: &Scope, cache_key: &str) -> String {
    format!("{}:{}:{}", scope.workspace_id, scope.org_id, cache_key)
}

#[async_trait::async_trait]
impl IdempotencyStore for MockIdemStore {
    async fn get(
        &self,
        scope: &Scope,
        cache_key: &str,
    ) -> Result<Option<CachedRecord>, StorageError> {
        Ok(self
            .rows
            .lock()
            .expect("mock lock")
            .get(&idem_key(scope, cache_key))
            .cloned())
    }

    async fn put(
        &self,
        scope: &Scope,
        cache_key: String,
        record: CachedRecord,
        _ttl: Duration,
    ) -> Result<(), StorageError> {
        // First-writer-wins, like the real stores.
        self.rows
            .lock()
            .expect("mock lock")
            .entry(idem_key(scope, &cache_key))
            .or_insert(record);
        Ok(())
    }

    async fn evict_expired(&self) -> Result<u64, StorageError> {
        Ok(0)
    }
}

// ── Mock control queue ────────────────────────────────────────────────────
// Records every enqueued message verbatim so the test can inspect the
// scope the decorator stamped.

#[derive(Default)]
struct MockControlQueue {
    enqueued: Mutex<Vec<ControlMsg>>,
}

impl std::fmt::Debug for MockControlQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MockControlQueue")
    }
}

#[async_trait::async_trait]
impl ControlQueue for MockControlQueue {
    async fn enqueue(&self, msg: &ControlMsg) -> Result<(), StorageError> {
        self.enqueued.lock().expect("mock lock").push(msg.clone());
        Ok(())
    }

    async fn claim_pending(
        &self,
        _processor: &[u8; 16],
        _batch_size: u32,
    ) -> Result<Vec<ControlMsg>, StorageError> {
        Ok(self.enqueued.lock().expect("mock lock").clone())
    }

    async fn mark_completed(
        &self,
        _id: &[u8; 16],
        _processor: &[u8; 16],
    ) -> Result<(), StorageError> {
        Ok(())
    }

    async fn mark_failed(
        &self,
        _id: &[u8; 16],
        _processor: &[u8; 16],
        _error: &str,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    async fn reclaim_stuck(
        &self,
        _reclaim_after: Duration,
        _max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError> {
        Ok(ReclaimOutcome::default())
    }

    async fn cleanup(&self, _retention: Duration) -> Result<u64, StorageError> {
        Ok(0)
    }
}

fn cached(body: &[u8]) -> CachedRecord {
    CachedRecord {
        status: 200,
        headers: b"h".to_vec(),
        body: body.to_vec(),
        fingerprint: b"fp".to_vec(),
        expires_at: "2999-01-01T00:00:00Z".into(),
    }
}

// ── Abuse case 1: confused deputy / cross-tenant row access ───────────────

#[tokio::test]
async fn cross_tenant_get_yields_none_never_the_row() {
    let mock: Arc<MockExecStore> = Arc::new(MockExecStore::default());
    let tenant_a = ScopedExecutionStore::new(mock.clone(), scope_a());
    let tenant_b = ScopedExecutionStore::new(mock.clone(), scope_b());

    // Tenant A creates an execution. The scope arg passed by the caller is
    // irrelevant — the decorator substitutes A's bound scope.
    tenant_a
        .create(&scope_a(), "exe_x", "wf_1", serde_json::json!({}))
        .await
        .expect("A creates");

    // Tenant B asks for the very same id, even forging A's scope in the
    // argument. The decorator substitutes B's bound scope, so the lookup
    // key is B's — a clean miss.
    let forged = tenant_b
        .get(&scope_a(), "exe_x")
        .await
        .expect("B get must not error");
    assert!(
        forged.is_none(),
        "cross-tenant get must yield None — never the row, never a leaking error"
    );

    // Sanity: A still sees its own row through the decorator.
    let own = tenant_a.get(&scope_b(), "exe_x").await.expect("A get");
    assert!(own.is_some(), "owner must still see its own row");
}

#[tokio::test]
async fn cross_tenant_commit_never_applies() {
    let mock: Arc<MockExecStore> = Arc::new(MockExecStore::default());
    let tenant_a = ScopedExecutionStore::new(mock.clone(), scope_a());
    let tenant_b = ScopedExecutionStore::new(mock.clone(), scope_b());

    tenant_a
        .create(&scope_a(), "exe_y", "wf_1", serde_json::json!({}))
        .await
        .expect("A creates");

    // Tenant B builds a batch *explicitly targeting A's scope* and A's
    // execution id — the confused-deputy attack. The decorator rebinds the
    // batch to B's scope before it reaches the store, so the CAS misses.
    let attack = TransitionBatch::builder()
        .scope(scope_a())
        .execution_id("exe_y")
        .expected_version(0)
        .fencing(FencingToken::from_generation(0))
        .new_state(serde_json::json!({"s": "hijacked"}))
        .build()
        .expect("batch");
    let outcome = tenant_b.commit(attack).await.expect("commit returns");
    assert!(
        !matches!(outcome, TransitionOutcome::Applied { .. }),
        "cross-tenant commit must NEVER Apply (got {outcome:?})"
    );

    // A's row is untouched: still version 0, original state.
    let row = tenant_a
        .get(&scope_a(), "exe_y")
        .await
        .expect("A get")
        .expect("A row present");
    assert_eq!(row.version, 0, "victim row must be unmodified");
    assert_eq!(row.state, serde_json::json!({}), "victim state intact");
}

// ── Abuse case 2: idempotency replay-oracle ───────────────────────────────

#[tokio::test]
async fn cross_tenant_idempotency_keys_are_isolated() {
    let mock: Arc<MockIdemStore> = Arc::new(MockIdemStore::default());
    let tenant_a = ScopedIdempotencyStore::new(mock.clone(), scope_a());
    let tenant_b = ScopedIdempotencyStore::new(mock.clone(), scope_b());

    // Both tenants use the *same raw key*, and each passes the *other
    // tenant's* scope as the per-call arg — proving the decorator ignores
    // it and substitutes its bound scope, so the stored keys still differ.
    tenant_a
        .put(
            &scope_b(),
            "POST /pay:idem-1".into(),
            cached(b"A-response"),
            Duration::from_mins(1),
        )
        .await
        .expect("A put");

    // B probes the same raw key: must be a clean miss (no replay oracle).
    let probe = tenant_b
        .get(&scope_a(), "POST /pay:idem-1")
        .await
        .expect("B get must not error");
    assert!(
        probe.is_none(),
        "tenant B must not observe tenant A's dedup entry"
    );

    // B poisons its own namespace with the same raw key; A's entry must
    // survive untouched (no cross-tenant poisoning).
    tenant_b
        .put(
            &scope_a(),
            "POST /pay:idem-1".into(),
            cached(b"B-poison"),
            Duration::from_mins(1),
        )
        .await
        .expect("B put");
    let a_entry = tenant_a
        .get(&scope_b(), "POST /pay:idem-1")
        .await
        .expect("A get")
        .expect("A entry present");
    assert_eq!(
        a_entry.body, b"A-response",
        "tenant A's response must be unpoisoned by tenant B"
    );
}

// ── Abuse case 3: control-queue confused deputy ───────────────────────────

#[tokio::test]
async fn cross_tenant_control_enqueue_is_stamped_with_bound_scope() {
    let mock: Arc<MockControlQueue> = Arc::new(MockControlQueue::default());
    let tenant_a = ScopedControlQueue::new(mock.clone(), scope_a());

    // Tenant A tries to enqueue a Cancel carrying tenant B's scope and B's
    // execution id — a confused-deputy attempt to cancel B's run.
    let attack = ControlMsg {
        id: [7u8; 16],
        execution_id: "exe_b_victim".into(),
        command: ControlCommand::Cancel,
        scope: scope_b(), // forged target tenant
        w3c_traceparent: None,
        reclaim_count: 0,
    };
    tenant_a.enqueue(&attack).await.expect("A enqueue");

    // The decorator overwrote the scope with A's bound scope before the
    // queue saw it — the Cancel can never be dispatched against B.
    let enqueued = mock.enqueued.lock().expect("mock lock");
    assert_eq!(enqueued.len(), 1);
    assert_eq!(
        enqueued[0].scope,
        scope_a(),
        "enqueued control message must carry the enqueuer's bound scope, never the forged target"
    );
    assert_ne!(
        enqueued[0].scope,
        scope_b(),
        "a low-privilege tenant must not enqueue a Cancel for another tenant"
    );
}
