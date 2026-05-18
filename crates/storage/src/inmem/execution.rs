//! In-memory `ExecutionStore` + `IdempotencyGuard` over a shared core.
//!
//! The execution store, control queue, and journal reader all wrap the
//! same [`SharedState`] so a `commit`'s outbox + journal rows are
//! observable through the queue / reader (the conformance suite asserts
//! this atomic-triple visibility).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

// Lease/queue expiry uses `tokio::time::Instant` (not `std::time::Instant`)
// so paused-time integration tests (`tokio::time::pause`/`advance`) drive
// lease TTL and reclaim staleness deterministically with zero wall-clock
// cost — the contract the prior in-memory adapter guaranteed.
use tokio::time::Instant;

use nebula_storage_port::dto::{ControlMsg, ExecutionRecord};
use nebula_storage_port::store::{ExecutionStore, IdempotencyGuard};
use nebula_storage_port::{FencingToken, Scope, StorageError, TransitionBatch, TransitionOutcome};
use parking_lot::Mutex;

/// One persisted execution row plus its lease bookkeeping.
#[derive(Debug, Clone)]
pub(super) struct Row {
    pub(super) scope: Scope,
    pub(super) workflow_id: String,
    pub(super) version: u64,
    pub(super) status: String,
    pub(super) state: serde_json::Value,
    /// Current lease holder, if any (alive only until `lease_expires_at`).
    pub(super) lease_holder: Option<String>,
    pub(super) lease_expires_at: Option<Instant>,
    /// Monotone fencing generation. Bumped every time the lease is
    /// (re)acquired by a different/expired holder, so a superseded
    /// holder's token no longer matches.
    pub(super) fencing_generation: u64,
    /// Append-only journal: `(seq, payload)` oldest first.
    pub(super) journal: Vec<(u64, serde_json::Value)>,
}

/// One queued control message plus its processing bookkeeping.
#[derive(Debug, Clone)]
pub(super) struct QueuedMsg {
    pub(super) msg: ControlMsg,
    pub(super) status: String,
    pub(super) processed_by: Option<[u8; 16]>,
    pub(super) processed_at: Option<Instant>,
    pub(super) reclaim_count: u32,
    pub(super) error_message: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct State {
    pub(super) rows: HashMap<String, Row>,
    /// Control-queue rows keyed by the message's 16-byte id.
    pub(super) queue: HashMap<[u8; 16], QueuedMsg>,
    /// Per-execution next journal sequence number.
    pub(super) next_seq: HashMap<String, u64>,
}

/// Shared mutable core. One mutex guards the whole store so a `commit`
/// applies its triple atomically and the queue/reader observe a
/// consistent snapshot.
pub(super) type SharedState = Arc<Mutex<State>>;

/// In-memory execution aggregate.
#[derive(Debug, Default, Clone)]
pub struct InMemoryExecutionStore {
    pub(super) inner: SharedState,
}

impl InMemoryExecutionStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow the shared core so sibling stores (control queue, journal
    /// reader) can be built over the same data.
    #[must_use]
    pub(super) fn shared(&self) -> SharedState {
        Arc::clone(&self.inner)
    }
}

/// Clamp the lease TTL to a sane range (mirrors the legacy repo: ≥1s,
/// ≤24h) so a zero / absurd TTL cannot make a lease instantly dead or
/// effectively eternal.
fn normalized_ttl(ttl: Duration) -> Duration {
    Duration::from_secs_f64(ttl.as_secs_f64().clamp(1.0, 86_400.0))
}

#[async_trait::async_trait]
impl ExecutionStore for InMemoryExecutionStore {
    async fn create(
        &self,
        scope: &Scope,
        id: &str,
        workflow_id: &str,
        initial_state: serde_json::Value,
    ) -> Result<(), StorageError> {
        let mut st = self.inner.lock();
        if st.rows.contains_key(id) {
            return Err(StorageError::Duplicate {
                entity: "execution",
                detail: format!("execution {id} already exists"),
            });
        }
        st.rows.insert(
            id.to_string(),
            Row {
                scope: scope.clone(),
                workflow_id: workflow_id.to_string(),
                version: 0,
                status: "Created".to_string(),
                state: initial_state,
                lease_holder: None,
                lease_expires_at: None,
                fencing_generation: 0,
                journal: Vec::new(),
            },
        );
        tracing::debug!(
            target: "nebula_storage::inmem",
            execution_id = id,
            workflow_id,
            "execution created"
        );
        Ok(())
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ExecutionRecord>, StorageError> {
        let st = self.inner.lock();
        match st.rows.get(id) {
            // Scope mismatch is an existence-preserving miss: never leak
            // another tenant's row.
            Some(row) if &row.scope == scope => Ok(Some(ExecutionRecord {
                id: id.to_string(),
                workflow_id: row.workflow_id.clone(),
                scope: row.scope.clone(),
                version: row.version,
                status: row.status.clone(),
                state: row.state.clone(),
                lease_holder: row.lease_holder.clone(),
                fencing: Some(row.fencing_generation),
                created_at: String::new(),
                updated_at: String::new(),
            })),
            _ => Ok(None),
        }
    }

    async fn commit(&self, batch: TransitionBatch) -> Result<TransitionOutcome, StorageError> {
        let mut st = self.inner.lock();
        let id = batch.execution_id().to_string();
        let Some(row) = st.rows.get(&id) else {
            // Unknown id (or invisible cross-tenant): treat as a CAS miss
            // — never Apply.
            return Ok(TransitionOutcome::VersionConflict { actual: 0 });
        };
        // Cross-scope commit: the row is invisible to this tenant. Surface
        // it exactly like the unknown-id path above (`actual: 0`), never
        // an Apply. Echoing the real `row.version` here would be a
        // cross-tenant version oracle — a caller in scope B could probe
        // scope A's row by observing the conflict's `actual` counter. A
        // cross-tenant row must be indistinguishable from a missing one.
        if &row.scope != batch.scope() {
            return Ok(TransitionOutcome::VersionConflict { actual: 0 });
        }
        // Fencing gate: a superseded/older generation is rejected even if
        // the version matches (closes the zombie-runner hole, spec §4.1).
        if batch.fencing().generation() != row.fencing_generation {
            tracing::warn!(
                target: "nebula_storage::inmem",
                execution_id = %id,
                caller_generation = batch.fencing().generation(),
                current_generation = row.fencing_generation,
                "commit fenced out: caller token superseded"
            );
            return Ok(TransitionOutcome::FencedOut);
        }
        if row.version != batch.expected_version() {
            return Ok(TransitionOutcome::VersionConflict {
                actual: row.version,
            });
        }
        // CAS + fencing held — apply state, outbox, journal atomically
        // under the single lock.
        let new_version = row.version + 1;
        let new_state = batch.new_state().clone();
        let outbox: Vec<ControlMsg> = batch.outbox().to_vec();
        let journal_payloads: Vec<serde_json::Value> =
            batch.journal().iter().map(|j| j.payload.clone()).collect();

        let mut seq = st.next_seq.get(&id).copied().unwrap_or(1);
        {
            // guard-justified: the row's presence was asserted earlier in
            // this same function under the *same* `st` lock guard (the CAS
            // + fencing check above borrows `row`); the lock is never
            // released between, so the entry cannot vanish here.
            let row = st
                .rows
                .get_mut(&id)
                .unwrap_or_else(|| unreachable!("row presence checked under the same lock"));
            row.version = new_version;
            row.state = new_state;
            for payload in journal_payloads {
                row.journal.push((seq, payload));
                seq += 1;
            }
        }
        st.next_seq.insert(id.clone(), seq);
        for msg in outbox {
            st.queue.insert(
                msg.id,
                QueuedMsg {
                    msg,
                    status: "Pending".to_string(),
                    processed_by: None,
                    processed_at: None,
                    reclaim_count: 0,
                    error_message: None,
                },
            );
        }
        tracing::debug!(
            target: "nebula_storage::inmem",
            execution_id = %id,
            new_version,
            "commit applied (state + outbox + journal)"
        );
        Ok(TransitionOutcome::Applied { new_version })
    }

    async fn acquire_lease(
        &self,
        scope: &Scope,
        id: &str,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<FencingToken>, StorageError> {
        let ttl = normalized_ttl(ttl);
        let mut st = self.inner.lock();
        let Some(row) = st.rows.get_mut(id) else {
            return Err(StorageError::not_found("execution", id));
        };
        if &row.scope != scope {
            return Err(StorageError::not_found("execution", id));
        }
        let now = Instant::now();
        let live = matches!(row.lease_expires_at, Some(exp) if exp >= now);
        if live {
            // A live lease blocks acquisition outright — including a
            // second acquire by the *same* holder. Renewal is the
            // dedicated `renew_lease` op (fencing-token gated); a second
            // `acquire_lease` while the lease is live is contention, not
            // a silent renew (zombie-runner closure — two concurrent
            // runners must see exactly one winner).
            return Ok(None);
        }
        // No live lease: acquire it. Every successful acquire bumps the
        // fencing generation, so every previously issued token is dead
        // — including one held by the *same* holder string (a
        // crashed-then-restarted runner reusing its `instance_id` is a
        // zombie w.r.t. its pre-crash token). Generation 0 therefore
        // universally means "no lease ever issued / stale"
        // (zombie-runner closure).
        row.fencing_generation += 1;
        row.lease_holder = Some(holder.to_string());
        row.lease_expires_at = Some(now.checked_add(ttl).unwrap_or(now));
        let token = FencingToken::from_generation(row.fencing_generation);
        tracing::debug!(
            target: "nebula_storage::inmem",
            execution_id = id,
            holder,
            generation = row.fencing_generation,
            "lease acquired"
        );
        Ok(Some(token))
    }

    async fn renew_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: FencingToken,
        ttl: Duration,
    ) -> Result<bool, StorageError> {
        let ttl = normalized_ttl(ttl);
        let mut st = self.inner.lock();
        let Some(row) = st.rows.get_mut(id) else {
            return Ok(false);
        };
        if &row.scope != scope || token.generation() != row.fencing_generation {
            return Ok(false);
        }
        let now = Instant::now();
        row.lease_expires_at = Some(now.checked_add(ttl).unwrap_or(now));
        Ok(true)
    }

    async fn release_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: FencingToken,
    ) -> Result<bool, StorageError> {
        let mut st = self.inner.lock();
        let Some(row) = st.rows.get_mut(id) else {
            return Ok(false);
        };
        if &row.scope != scope || token.generation() != row.fencing_generation {
            return Ok(false);
        }
        row.lease_holder = None;
        row.lease_expires_at = None;
        Ok(true)
    }

    async fn list_running(&self, scope: &Scope) -> Result<Vec<String>, StorageError> {
        let st = self.inner.lock();
        Ok(st
            .rows
            .iter()
            .filter(|(_, r)| &r.scope == scope)
            .map(|(id, _)| id.clone())
            .collect())
    }

    async fn list_running_for_workflow(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Vec<String>, StorageError> {
        let st = self.inner.lock();
        Ok(st
            .rows
            .iter()
            .filter(|(_, r)| &r.scope == scope && r.workflow_id == workflow_id)
            .map(|(id, _)| id.clone())
            .collect())
    }

    async fn count(&self, scope: &Scope, workflow_id: Option<&str>) -> Result<u64, StorageError> {
        let st = self.inner.lock();
        let n = st
            .rows
            .values()
            .filter(|r| &r.scope == scope && workflow_id.is_none_or(|w| r.workflow_id == w))
            .count();
        Ok(n as u64)
    }
}

/// In-memory idempotency guard. Keys are `{scope}:{execution_id}:{node_id}:
/// {attempt}` so a cross-tenant probe cannot collide with another tenant's
/// dedup entry (the decorator namespaces by scope; we fold scope into the
/// key directly here for the raw-adapter conformance case).
#[derive(Debug, Default, Clone)]
pub struct InMemoryIdempotencyGuard {
    // `Arc` so clones share the mark set — repeated guard handles for the
    // same store observe each other's marks (first-writer-wins is global).
    marked: Arc<Mutex<HashSet<String>>>,
}

impl InMemoryIdempotencyGuard {
    /// Create an empty guard.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Non-mutating introspection: report whether
    /// `{scope}:{execution_id}:{node_id}:{attempt}` is already marked,
    /// without marking it. Mirrors the key derivation of
    /// [`IdempotencyGuard::check_and_mark`] so callers can assert
    /// dedup state without perturbing it.
    #[must_use]
    pub fn is_marked(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
        attempt: u32,
    ) -> bool {
        let key = format!(
            "{}:{}:{execution_id}:{node_id}:{attempt}",
            scope.workspace_id, scope.org_id
        );
        self.marked.lock().contains(&key)
    }
}

#[async_trait::async_trait]
impl IdempotencyGuard for InMemoryIdempotencyGuard {
    async fn check_and_mark(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
        attempt: u32,
    ) -> Result<bool, StorageError> {
        let key = format!(
            "{}:{}:{execution_id}:{node_id}:{attempt}",
            scope.workspace_id, scope.org_id
        );
        let mut marked = self.marked.lock();
        Ok(marked.insert(key))
    }
}
