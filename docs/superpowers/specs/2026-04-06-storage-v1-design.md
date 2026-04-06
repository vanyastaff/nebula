# nebula-storage v1 — Design Spec

## Goal

Complete storage for production v1: implement PgExecutionRepo, add QueueBackend trait, define migration strategy, wire into engine checkpoint contract.

## Current State

Storage crate has: generic `Storage` trait, `MemoryStorage`, `PostgresStorage`, `PgWorkflowRepo` (fully working with CAS), `ExecutionRepo` trait (in-memory only), `WorkflowRepo` trait with versioning. SQLx migrations. 

**Missing:** PgExecutionRepo, QueueBackend, per-node output storage, idempotency table, execution list/filter queries.

---

## 1. PgExecutionRepo (v1 BLOCKER)

The critical missing piece — engine needs persistent execution state.

### Schema

```sql
-- Execution state (one row per workflow execution)
CREATE TABLE executions (
    id UUID PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES workflows(id),
    owner_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'created',
    version INT NOT NULL DEFAULT 1,
    budget JSONB NOT NULL DEFAULT '{}',
    node_states JSONB NOT NULL DEFAULT '{}',
    variables JSONB NOT NULL DEFAULT '{}',
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_executions_owner ON executions(owner_id);
CREATE INDEX idx_executions_workflow ON executions(workflow_id);
CREATE INDEX idx_executions_status ON executions(status);
CREATE INDEX idx_executions_created ON executions(created_at DESC);

-- Per-node outputs (one row per completed node)
CREATE TABLE node_outputs (
    execution_id UUID NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    node_id UUID NOT NULL,
    action_key TEXT NOT NULL,
    status TEXT NOT NULL,
    output BYTEA,  -- rmp-serde MessagePack (serialization spec Section 2.3)
    output_bytes INT NOT NULL DEFAULT 0,
    duration_ms INT NOT NULL DEFAULT 0,
    error_message TEXT,
    attempt INT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (execution_id, node_id, attempt)
);

-- Execution journal (append-only event log)
CREATE TABLE execution_journal (
    id BIGSERIAL PRIMARY KEY,
    execution_id UUID NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    entry_type TEXT NOT NULL,
    node_id UUID,
    data JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_journal_execution ON execution_journal(execution_id, created_at);

-- Idempotency keys (durable, per action B1)
CREATE TABLE idempotency_keys (
    key TEXT PRIMARY KEY,
    execution_id UUID NOT NULL,
    node_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- TTL cleanup: keys older than 7 days
CREATE INDEX idx_idempotency_created ON idempotency_keys(created_at);
```

### Trait Implementation

```rust
impl ExecutionRepo for PgExecutionRepo {
    async fn create(&self, state: &ExecutionState) -> Result<()>;
    async fn save_state(&self, state: &ExecutionState) -> Result<()>;  // CAS via version
    async fn load_state(&self, id: &ExecutionId) -> Result<ExecutionState>;
    async fn transition(&self, id: &ExecutionId, from: ExecutionStatus, to: ExecutionStatus) -> Result<()>;

    // Per-node outputs (RT12 — individually durable)
    async fn save_node_output(&self, exec_id: &ExecutionId, node_id: &NodeId, output: &NodeOutputRecord) -> Result<()>;
    async fn load_node_output(&self, exec_id: &ExecutionId, node_id: &NodeId) -> Result<Option<Value>>;
    async fn load_all_outputs(&self, exec_id: &ExecutionId) -> Result<HashMap<NodeId, Value>>;

    // Query
    async fn list(&self, filter: ExecutionFilter) -> Result<Vec<ExecutionSummary>>;
    async fn list_running(&self) -> Result<Vec<ExecutionId>>;
    async fn count(&self, filter: ExecutionFilter) -> Result<u64>;

    // Idempotency (B1)
    async fn check_idempotency(&self, key: &str) -> Result<bool>;
    async fn mark_idempotent(&self, key: &str, exec_id: &ExecutionId, node_id: &NodeId) -> Result<()>;

    // Journal
    async fn append_journal(&self, entry: &JournalEntry) -> Result<()>;
    async fn load_journal(&self, exec_id: &ExecutionId) -> Result<Vec<JournalEntry>>;

    // Lease (for distributed execution v2)
    async fn try_acquire_lease(&self, exec_id: &ExecutionId, worker_id: &str, ttl: Duration) -> Result<bool>;
    async fn renew_lease(&self, exec_id: &ExecutionId, worker_id: &str, ttl: Duration) -> Result<bool>;
}
```

### ExecutionFilter

```rust
pub struct ExecutionFilter {
    pub workflow_id: Option<WorkflowId>,
    pub owner_id: Option<OwnerId>,
    pub status: Option<ExecutionStatus>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub cursor: Option<String>,
    pub limit: u32,
}
```

---

## 2. QueueBackend Trait (RT11)

Abstraction for durable event/task queue. Postgres v1, Redis/NATS v2.

```rust
pub trait QueueBackend: Send + Sync {
    async fn enqueue(&self, queue: &str, task: QueuedTask) -> Result<(), StorageError>;
    async fn dequeue(&self, queue: &str, worker_id: &str) -> Result<Option<QueuedTask>, StorageError>;
    async fn ack(&self, task_id: &str) -> Result<(), StorageError>;
    async fn nack(&self, task_id: &str) -> Result<(), StorageError>;
    async fn queue_depth(&self, queue: &str) -> Result<u64, StorageError>;
}

pub struct QueuedTask {
    pub id: String,
    pub queue: String,
    pub payload: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub visible_after: DateTime<Utc>,
    pub attempts: u32,
    pub max_attempts: u32,
}
```

### Postgres Implementation

```sql
CREATE TABLE task_queue (
    id TEXT PRIMARY KEY,
    queue TEXT NOT NULL,
    payload BYTEA NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',  -- pending, processing, completed, failed
    worker_id TEXT,
    attempts INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL DEFAULT 3,
    visible_after TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_queue_dequeue ON task_queue(queue, status, visible_after)
    WHERE status = 'pending';
```

Dequeue: `SELECT ... FOR UPDATE SKIP LOCKED` pattern:
```sql
UPDATE task_queue
SET status = 'processing', worker_id = $1, attempts = attempts + 1, updated_at = NOW()
WHERE id = (
    SELECT id FROM task_queue
    WHERE queue = $2 AND status = 'pending' AND visible_after <= NOW()
    ORDER BY created_at
    FOR UPDATE SKIP LOCKED
    LIMIT 1
)
RETURNING *;
```

---

## 3. SQLite Storage (v1.1 — desktop)

For desktop app (Apple Shortcuts feedback, deployment modes spec):

```rust
pub struct SqliteStorage {
    pool: sqlx::SqlitePool,
}

impl WorkflowRepo for SqliteStorage { /* same trait, SQLite queries */ }
impl ExecutionRepo for SqliteStorage { /* same trait, SQLite queries */ }
impl QueueBackend for SqliteStorage { /* same trait, SQLite queries */ }
```

Same SQL schema adapted for SQLite syntax. Migrations embedded in binary.

---

## 4. Migration Strategy

### SQLx Migrations

```
crates/storage/migrations/
├── 20260406_001_create_workflows.sql        (exists)
├── 20260406_002_create_storage_kv.sql       (exists)
├── 20260406_003_create_executions.sql       (NEW)
├── 20260406_004_create_node_outputs.sql     (NEW)
├── 20260406_005_create_execution_journal.sql (NEW)
├── 20260406_006_create_idempotency_keys.sql (NEW)
├── 20260406_007_create_task_queue.sql       (NEW)
```

SQLx offline mode: `task db:prepare` regenerates `.sqlx/` for CI.

### Schema Versioning

`schema_version` table tracks applied migrations. SQLx handles this automatically. For MessagePack-stored data (node_outputs.output BYTEA), serde `#[serde(default)]` handles forward compatibility. Breaking schema changes = new migration + code handling both formats.

---

## 5. Row-Level Security (W1, Supabase feedback)

```sql
-- Enable RLS on all tenant-scoped tables
ALTER TABLE workflows ENABLE ROW LEVEL SECURITY;
ALTER TABLE executions ENABLE ROW LEVEL SECURITY;
ALTER TABLE node_outputs ENABLE ROW LEVEL SECURITY;

-- Policy: users see only their own data
CREATE POLICY tenant_isolation ON workflows
    USING (owner_id = current_setting('app.current_owner'));
CREATE POLICY tenant_isolation ON executions
    USING (owner_id = current_setting('app.current_owner'));

-- Set per-transaction (not per-connection, RT-10 fix)
SET LOCAL app.current_owner = $1;
```

Storage layer calls `SET LOCAL` before every query transaction.

---

## 6. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| ExecutionRepo | In-memory only | PgExecutionRepo with full schema |
| Node outputs | Not persisted | node_outputs table (RT12) |
| Idempotency | In-memory HashSet | Postgres table (B1) |
| Journal | Not persisted | execution_journal table |
| Queue | None | QueueBackend trait + PgQueue |
| SQLite | None | v1.1 for desktop |
| RLS | None | Per-transaction SET LOCAL |
| Leases | Not implemented | try_acquire_lease for v2 distributed |

---

## 7. Not In Scope

- Redis backend (v2)
- S3/object storage backend (v2)
- ClickHouse history writer (separate concern — telemetry spec)
- Read replica routing (v1.1)
- Connection pool sharing across crates (deployment concern)
- Data encryption at rest for execution state (PCI gap — credential spec C5 covers credentials, execution data TBD)
