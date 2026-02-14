# Nebula Architecture Overview

## System Overview

Nebula is a high-performance workflow engine written in Rust (similar to n8n/Make/Zapier), designed to run identically on desktop, self-hosted, and cloud environments. It is a single workspace containing multiple crates.

Key differentiator: **security, performance, and resource efficiency** are achieved not by limiting features, but through architectural decisions (capability-based sandbox, bounded concurrency, typed actions, execution plan built before run).

### Architectural Principles

1.  **Portable Core** — The core logic does not know where it is running (desktop/self-host/cloud).
2.  **Ports & Drivers** — Core depends only on trait interfaces (`ports`). Heavy implementations reside in `drivers/`.
3.  **Typed + Dynamic** — Typed Actions (first-party) + dynamic `serde_json::Value` (community/hub).
4.  **Capability-based Sandbox** — Actions have no default permissions; access is only granted via capabilities.
5.  **Execution Plan** — An execution plan with bounded concurrency is constructed *before* execution starts.
6.  **ExecutionRepo as Truth** — `ports::ExecutionRepo` (journal + CAS transitions) is the *single source of truth*. Events are merely projections.
7.  **Expression Safety** — Custom DSL with strict limits (no eval).
8.  **Resource Policies** — Scoped lifecycle with eviction, TTL, and pressure hooks.
9.  **Three Presets** — desktop/selfhost/cloud = different drivers + config, same core.

### Workspace Structure (High Level)

```
nebula/
  Cargo.toml                         # workspace root
  crates/
    # ──── CORE (portable, no heavy deps) ────
    core/                            # base types: ids, ParamValue, errors
    workflow/                        # graph, WorkflowDefinition, NodeDefinition
    execution/                       # ExecutionContext, Journal, Idempotency
    engine/                          # ExecutionPlanner, Scheduler, orchestration
    runtime/                         # ActionRuntime, execute node, budgets
    action/                          # Action trait, ActionResult, ActionError, capabilities
    expression/                      # safe DSL + eval limits
    parameter/                       # parameter schema, ParamValue resolve
    validator/                       # validators, combinators
    telemetry/                       # eventbus + metrics + logging + tracing
    memory/                          # arenas, scoped allocation, caching

    # ──── PORTS (only traits, zero heavy deps) ────
    ports/                           # trait StorageRepo, Queue, SandboxRunner,
                                     # BlobStore, SecretsStore + domain repos

    # ──── NODE/BUSINESS ────
    credential/                      # AuthData types, rotation, encryption
    resource/                        # resource lifecycle, scopes, policies
    registry/                        # action/workflow registry, versioning, schema

    # ──── CROSS-CUTTING ────
    config/                          # configuration, hot-reload, presets
    resilience/                      # circuit breaker, retry, bulkhead, rate limit
    system/                          # cross-platform utilities, pressure detection
    locale/                          # i18n
    derive/                          # proc macros (#[derive(Action)], etc.)

    # ──── DRIVERS (heavy deps — ONLY here) ────
    drivers/
      storage-sqlite/                # SQLite backend
      storage-postgres/              # PostgreSQL backend
      queue-memory/                  # in-memory bounded queue
      queue-redis/                   # Redis queue
      blob-fs/                       # filesystem blob storage
      blob-s3/                       # S3/MinIO blob storage
      secrets-local/                 # encrypted local keystore
      secrets-vault/                 # HashiCorp Vault (future)
      sandbox-inprocess/             # in-process capability checks
      sandbox-wasm/                  # WASM isolation (wasmtime)

    # ──── BINS (distribution units) ────
    bins/
      desktop/                       # single-user, minimal deps
      server/                        # self-host, all-in-one
      worker/                        # cloud data plane
      control-plane/                 # cloud API + scheduler + registry

    # ──── OPTIONAL / PHASE 2 ────
    cluster/                         # leader election, Raft (optional feature)
    tenant/                          # multi-tenancy isolation, quotas
```

### Dependency Direction (Strict Rule)

```
bins → drivers → ports ← core
              ↘       ↗
               drivers
```

**Core crates NEVER depend on drivers.**
Core speaks only to `ports` (traits). Drivers implement `ports`.
Bins "glue" core + required drivers via composition root.

### Architecture Layers

```
┌─────────────────────────────────────────────────────────┐
│                   Bins (Distribution)                    │
│     desktop, server, worker, control-plane              │
├─────────────────────────────────────────────────────────┤
│                   Drivers Layer                          │
│  storage-sqlite, storage-postgres, queue-memory,        │
│  queue-redis, blob-fs, blob-s3, secrets-local,          │
│  sandbox-inprocess, sandbox-wasm                        │
├─────────────────────────────────────────────────────────┤
│                   Ports Layer                            │
│  ports (traits: StorageRepo, Queue, SandboxRunner,      │
│         BlobStore, SecretsStore, domain repos)           │
├─────────────────────────────────────────────────────────┤
│               Business / Node Layer                      │
│        resource, registry, credential                    │
├─────────────────────────────────────────────────────────┤
│                  Execution Layer                         │
│     engine, runtime, sandbox-API (in action)             │
├─────────────────────────────────────────────────────────┤
│                    Core Layer                            │
│  core, workflow, execution, action, expression,          │
│  parameter, validator, memory, telemetry                 │
├─────────────────────────────────────────────────────────┤
│              Cross-Cutting Concerns                      │
│    config, resilience, system, locale, derive            │
└─────────────────────────────────────────────────────────┘
```

---

## Core Layer

### core
**Purpose:** Base types and traits used by all system crates.

```rust
// Basic identifiers
pub struct ExecutionId(Uuid);
pub struct WorkflowId(String);
pub struct NodeId(String);
pub struct UserId(String);
pub struct TenantId(String);
pub struct ActionId(String);

// Scopes for resource management
pub enum ScopeLevel {
    Global,
    Workflow(WorkflowId),
    Execution(ExecutionId),
    Action(ExecutionId, NodeId),
}

pub trait Scoped {
    fn scope(&self) -> &ScopeLevel;
}

// ParamValue — separates expressions from literal data.
// In WorkflowDefinition: ParamValue. After resolve: serde_json::Value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Expression(Expression),
    Template(TemplateString),
    Literal(serde_json::Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expression {
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateString {
    pub template: String,
    pub bindings: Vec<String>,
}
```

---

### workflow
**Purpose:** Declarative workflow definition — "what needs to be done".

```rust
let workflow = WorkflowDefinition {
    id: WorkflowId::new("user-registration"),
    name: "User Registration Process",
    nodes: vec![
        NodeDefinition {
            id: NodeId::new("validate"),
            action_id: ActionId::new("validation.user_data"),
            action_interface_version: InterfaceVersion { major: 1, minor: 0 },
            parameters: params!{
                "email_pattern" => ParamValue::Literal(json!("^[^@]+@[^@]+$")),
            },
        },
        NodeDefinition {
            id: NodeId::new("create_user"),
            action_id: ActionId::new("database.insert"),
            action_interface_version: InterfaceVersion { major: 2, minor: 1 },
            parameters: params!{
                "data" => ParamValue::Expression(Expression {
                    raw: "$nodes.validate.result.validated_data".into()
                })
            },
        }
    ],
    connections: vec![
        Connection {
            from_node: "validate",
            to_node: "create_user",
            condition: Some("$nodes.validate.success"),
        }
    ],
};
```

---

### execution
**Purpose:** Runtime workflow execution. Includes Journal and IdempotencyManager.

`ports::ExecutionRepo` — single source of truth (journal + CAS transitions).
Core does not have a separate "StateStore" — this is a driver implementation detail (Postgres/SQLite).
EventBus (telemetry) — only projections.

```rust
pub struct ExecutionContext {
    pub execution_id: ExecutionId,
    pub workflow_id: WorkflowId,
    pub workflow_definition: Arc<WorkflowDefinition>,
    pub node_outputs: Arc<RwLock<HashMap<NodeId, NodeOutput>>>,
    pub journal: Arc<ExecutionJournal>,
    pub idempotency: Arc<IdempotencyManager>,
    pub cancellation: CancellationToken,
    // Storage and sandbox — via ports traits, injected into runtime/engine
}

/// What is passed between nodes
pub enum NodeOutputData {
    /// Small data — inline JSON
    Inline(serde_json::Value),
    /// Large data → via BlobStore (spill when data limit exceeded)
    BlobRef { key: String, size: u64, mime: String },
}

pub struct NodeOutput {
    pub result: NodeOutputData,
    pub status: NodeStatus,
    pub duration: Duration,
}

/// Execution Journal — append-only log of decisions
pub struct NodeAttempt {
    pub node_id: NodeId,
    pub attempt_number: u32,
    pub idempotency_key: String,
    pub resolved_inputs: serde_json::Value,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub started_at: SystemTime,
    pub completed_at: Option<SystemTime>,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

pub enum JournalEntry {
    ExecutionStarted { input: serde_json::Value, plan: ExecutionPlan },
    NodeAttempt(NodeAttempt),
    ExecutionCompleted { output: serde_json::Value },
    ExecutionFailed { error: String, retry_count: u32 },
    CancellationRequested,
}
```

---

### expression
**Purpose:** Custom DSL for expressions. No eval, strict safety limits.

```rust
pub struct ExpressionLimits {
    pub max_ast_depth: usize,          // default: 32
    pub max_operations: usize,         // default: 10_000
    pub max_string_length: usize,      // default: 1MB
    pub max_array_length: usize,       // default: 100_000
    pub max_evaluation_time: Duration, // default: 5s
}

// Strict access model: expression sees only allowed scope.
// Secrets are NOT available via expressions — only via Capability.
let result: serde_json::Value = engine.evaluate(expr, &scope).await?;
```

---

### telemetry
**Purpose:** Unified observability: eventbus + metrics + logging + tracing.
Events — projections, NOT source of truth. ExecutionRepo = truth.

```rust
pub struct Telemetry {
    event_bus: EventBus,
    metrics: MetricsRegistry,
    logger: Logger,
}

// ExecutionRepo transition → emit event as projection
execution_repo.transition(&id, Running, Completed, journal_entry).await?;
telemetry.emit(ExecutionEvent::Completed { ... }).await?;
```

---

### memory
**Purpose:** Arenas, scoped allocation, tiered caching.

```rust
pub struct MemoryManager {
    global_arena: Arc<GlobalArena>,
    execution_arenas: Arc<DashMap<ExecutionId, ExecutionArena>>,
    cache: Arc<TieredMemoryCache>,
}
```

---

## Ports Layer

### ports
**Purpose:** Trait interfaces for everything that changes between environments.
Core crates depend ONLY on `ports`. Drivers implement `ports`.

```rust
// === Storage: Domain Repositories ===
// ExecutionRepo — The SINGLE source of truth for state.
// No separate "StateStore" in core. Drivers (Postgres/SQLite)
// implement CAS transitions + journal append as needed for their backend.

#[async_trait]
pub trait WorkflowRepo: Send + Sync {
    async fn get(&self, id: &WorkflowId) -> Result<Option<WorkflowDefinition>>;
    async fn save(&self, def: &WorkflowDefinition) -> Result<()>;
    async fn list(&self, filter: &WorkflowFilter) -> Result<Vec<WorkflowSummary>>;
}

#[async_trait]
pub trait ExecutionRepo: Send + Sync {
    async fn get_state(&self, id: &ExecutionId) -> Result<Option<ExecutionState>>;
    /// Atomic CAS transition + journal append.
    /// Driver decides how to do this: SQL transaction (Postgres),
    /// single-writer lock (SQLite), etc.
    async fn transition(
        &self, id: &ExecutionId,
        from: ExecutionStatus, to: ExecutionStatus,
        entry: JournalEntry,
    ) -> Result<bool>;
    async fn get_journal(&self, id: &ExecutionId) -> Result<Vec<JournalEntry>>;
    async fn acquire_lease(&self, id: &ExecutionId, holder: &str, ttl: Duration) -> Result<bool>;
    /// Find executions with expired lease (for reaper/recovery)
    async fn find_stale_leases(&self, older_than: Duration) -> Result<Vec<ExecutionId>>;
}

#[async_trait]
pub trait CredentialRepo: Send + Sync {
    async fn get(&self, id: &CredentialId, tenant: &TenantId) -> Result<Option<EncryptedCredential>>;
    async fn save(&self, cred: &EncryptedCredential) -> Result<()>;
}

// === Queue ===
// Semantics: at-least-once delivery.
// Consistency achieved via idempotency_key in NodeAttempt.
// Mandatory operation order in worker:
//   1. dequeue (task received, visibility timeout started)
//   2. acquire_lease in ExecutionRepo (if distributed)
//   3. journal append: attempt started
//   4. execute action
//   5. journal append: attempt completed/failed
//   6. ack (or nack for retry)
// If worker crashes between 3 and 6 — visibility timeout expires,
// task returns to queue, idempotency_key prevents duplicates.

#[async_trait]
pub trait TaskQueue: Send + Sync {
    async fn enqueue(&self, task: Task) -> Result<()>;
    /// Dequeue with visibility timeout. If not ack'd — task returns.
    async fn dequeue(&self, timeout: Duration) -> Result<Option<Task>>;
    async fn ack(&self, task_id: &str) -> Result<()>;
    async fn nack(&self, task_id: &str) -> Result<()>;
    /// Reaper: claim pending entries from dead consumers (Redis: XAUTOCLAIM)
    async fn claim_stale(&self, idle_time: Duration) -> Result<Vec<Task>>;
}

// === Blob Storage ===
#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn put(&self, key: &str, data: &[u8]) -> Result<()>;
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    async fn delete(&self, key: &str) -> Result<()>;
}

// === Secrets ===
#[async_trait]
pub trait SecretsStore: Send + Sync {
    async fn get_secret(&self, id: &str) -> Result<Option<SecretString>>;
    async fn set_secret(&self, id: &str, value: &SecretString) -> Result<()>;
    async fn delete_secret(&self, id: &str) -> Result<()>;
}

// === Sandbox ===
// The single trait for sandbox. No parallel interfaces.
// Capability checking — responsibility of implementation inside execute().
// Driver (inprocess/wasm) checks capabilities and enforces limits.
#[async_trait]
pub trait SandboxRunner: Send + Sync {
    /// Execute action in sandbox. Driver internally:
    /// 1. Checks capabilities (metadata vs granted)
    /// 2. Enforces limits (memory, CPU, wall time)
    /// 3. Executes action
    /// 4. Validates output
    async fn execute(
        &self,
        action: &dyn Action,
        context: SandboxedContext,
        metadata: &ActionMetadata,
    ) -> Result<serde_json::Value>;
}
```

---

## Presets (Desktop / Self-host / Cloud)

One core, different drivers + config:

| Aspect | Desktop | Self-host | Cloud |
|--------|---------|-----------|-------|
| Storage | storage-sqlite | storage-postgres (or sqlite) | storage-postgres |
| Blobs | blob-fs | blob-fs or blob-s3 | blob-s3 |
| Queue | queue-memory | queue-memory or queue-redis | queue-redis |
| Sandbox default | capability-gated | capability-gated (internal), isolated (community) | isolated (always) |
| Sandbox community | disabled or isolated | isolated (WASM) | isolated (WASM) |
| Secrets | secrets-local | secrets-local | secrets-vault/kms |
| Multi-tenancy | no | optional | mandatory |
| Cluster | no | no | coordinator + workers |
| Auth | no / local | local users / OAuth | multi-tenant |
| Concurrency | 4 | 64 | 500+ |

**Desktop** does not compile: postgres, redis, s3, wasmtime.
**Server** selects drivers via cargo features.
**Cloud** splits into control-plane + worker.
