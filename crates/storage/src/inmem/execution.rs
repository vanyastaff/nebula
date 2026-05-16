//! In-memory `ExecutionStore` + `IdempotencyGuard`.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use nebula_storage_port::dto::ExecutionRecord;
use nebula_storage_port::store::{ExecutionStore, IdempotencyGuard};
use nebula_storage_port::{FencingToken, Scope, StorageError, TransitionBatch, TransitionOutcome};
use parking_lot::Mutex;

/// One persisted execution row plus its lease bookkeeping.
#[derive(Debug, Clone)]
struct Row {
    scope: Scope,
    workflow_id: String,
    version: u64,
    status: String,
    state: serde_json::Value,
    /// Current lease holder, if any (alive only until `lease_expires_at`).
    lease_holder: Option<String>,
    lease_expires_at: Option<Instant>,
    /// Monotone fencing generation. Bumped every time the lease is
    /// (re)acquired by a different/expired holder, so a superseded holder's
    /// token no longer matches.
    fencing_generation: u64,
    journal: Vec<serde_json::Value>,
}

#[derive(Debug, Default)]
struct State {
    rows: HashMap<String, Row>,
    /// Appended outbox messages (control-queue rows) keyed by execution id.
    outbox: HashMap<String, Vec<nebula_storage_port::dto::ControlMsg>>,
}

/// In-memory execution aggregate. One mutex guards the whole store so a
/// `commit` applies its triple atomically.
#[derive(Debug, Default)]
pub struct InMemoryExecutionStore {
    inner: Mutex<State>,
}

impl InMemoryExecutionStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
        // it as a conflict, never an Apply, never a leak.
        if &row.scope != batch.scope() {
            return Ok(TransitionOutcome::VersionConflict {
                actual: row.version,
            });
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
        // CAS + fencing held — apply state, outbox, journal atomically.
        let new_version = row.version + 1;
        let new_state = batch.new_state().clone();
        let outbox: Vec<_> = batch.outbox().to_vec();
        let journal: Vec<_> = batch.journal().iter().map(|j| j.payload.clone()).collect();
        let row = st
            .rows
            .get_mut(&id)
            .unwrap_or_else(|| unreachable!("row presence checked under the same lock"));
        row.version = new_version;
        row.state = new_state;
        row.journal.extend(journal);
        if !outbox.is_empty() {
            st.outbox.entry(id.clone()).or_default().extend(outbox);
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
        if live && row.lease_holder.as_deref() != Some(holder) {
            // Another holder owns a live lease.
            return Ok(None);
        }
        // Acquire (or re-acquire by the same holder). A takeover from an
        // expired/absent holder bumps the fencing generation so the prior
        // holder's token is dead.
        let taking_over = row.lease_holder.as_deref() != Some(holder);
        if taking_over {
            row.fencing_generation += 1;
        }
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
#[derive(Debug, Default)]
pub struct InMemoryIdempotencyGuard {
    marked: Mutex<HashSet<String>>,
}

impl InMemoryIdempotencyGuard {
    /// Create an empty guard.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
