# Nebula Architecture Overview

## Обзор системы

Nebula — высокопроизводительный workflow engine на Rust (аналог n8n/Make/Zapier), 
работающий одинаково на desktop, self-host и cloud. Один workspace, множество крейтов.

Ключевое отличие: безопасность, производительность и экономия ресурсов — не за счёт фич,
а за счёт архитектурных решений (capability-based sandbox, bounded concurrency, 
typed actions, execution plan до запуска).

### Архитектурные принципы

1. **Portable Core** — ядро не знает где запущено (desktop/self-host/cloud)
2. **Ports & Drivers** — core зависит только от trait-интерфейсов (`ports`), тяжёлые реализации — в `drivers/`
3. **Typed + Dynamic** — типизированные Actions (first-party) + dynamic serde_json::Value (community/hub)
4. **Capability-based Sandbox** — Action не имеет ничего по умолчанию, доступ только через capabilities
5. **Execution Plan** — план с bounded concurrency строится ДО запуска
6. **ExecutionRepo as Truth** — `ports::ExecutionRepo` (journal + CAS transitions) = единственный источник истины, events = проекции
7. **Expression Safety** — собственный DSL с жёсткими limits (no eval)
8. **Resource Policies** — scoped lifecycle с eviction, TTL, pressure hooks
9. **Three Presets** — desktop/selfhost/cloud = разные drivers + config, одно ядро

### Workspace структура (верхний уровень)

```
nebula/
  Cargo.toml                         # workspace root
  crates/
    # ──── CORE (portable, без тяжёлых deps) ────
    core/                            # базовые типы: ids, ParamValue, errors
    workflow/                        # граф, WorkflowDefinition, NodeDefinition
    execution/                       # ExecutionContext, Journal, Idempotency
    engine/                          # ExecutionPlanner, Scheduler, orchestration
    runtime/                         # ActionRuntime, execute node, budgets
    action/                          # Action trait, ActionResult, ActionError, capabilities
    expression/                      # safe DSL + eval limits
    parameter/                       # parameter schema, ParamValue resolve
    validator/                       # validators, combinators
    telemetry/                       # eventbus + metrics + logging + tracing
    memory/                          # arenas, scoped allocation, caching

    # ──── PORTS (только traits, ноль тяжёлых deps) ────
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

    # ──── DRIVERS (тяжёлые deps — ТОЛЬКО тут) ────
    drivers/
      storage-sqlite/                # SQLite backend
      storage-postgres/              # PostgreSQL backend
      queue-memory/                  # in-memory bounded queue
      queue-redis/                   # Redis queue
      blob-fs/                       # filesystem blob storage
      blob-s3/                       # S3/MinIO blob storage
      secrets-local/                 # encrypted local keystore
      secrets-vault/                 # HashiCorp Vault (позже)
      sandbox-inprocess/             # in-process capability checks
      sandbox-wasm/                  # WASM isolation (wasmtime)

    # ──── BINS (distribution units) ────
    bins/
      desktop/                       # single-user, минимальные deps
      server/                        # self-host, all-in-one
      worker/                        # cloud data plane
      control-plane/                 # cloud API + scheduler + registry

    # ──── OPTIONAL / PHASE 2 ────
    cluster/                         # leader election, Raft (optional feature)
    tenant/                          # multi-tenancy isolation, quotas
```

### Dependency Direction (жёсткое правило)

```
bins → drivers → ports ← core
              ↘       ↗
               drivers
```

**Core крейты НИКОГДА не зависят от drivers.**
Core говорит только с `ports` (traits). Drivers реализуют `ports`.
Bins "склеивают" core + нужные drivers через composition root.

### Слои архитектуры

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
│     engine, runtime, sandbox-API (в action)              │
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
**Назначение:** Базовые типы и трейты, используемые всеми крейтами системы.

```rust
// Основные идентификаторы
pub struct ExecutionId(Uuid);
pub struct WorkflowId(String);
pub struct NodeId(String);
pub struct UserId(String);
pub struct TenantId(String);
pub struct ActionId(String);

// Scopes для resource management
pub enum ScopeLevel {
    Global,
    Workflow(WorkflowId),
    Execution(ExecutionId),
    Action(ExecutionId, NodeId),
}

pub trait Scoped {
    fn scope(&self) -> &ScopeLevel;
}

// ParamValue — разделяет expressions от литеральных данных.
// В WorkflowDefinition: ParamValue. После resolve: serde_json::Value.
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
**Назначение:** Декларативное определение workflow — "что нужно делать".

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
**Назначение:** Runtime выполнение workflow. Включает Journal и IdempotencyManager.

`ports::ExecutionRepo` — единственный источник истины (journal + CAS transitions).
В core нет отдельного "StateStore" — это implementation detail драйвера (Postgres/SQLite).
EventBus (telemetry) — только проекции.

```rust
pub struct ExecutionContext {
    pub execution_id: ExecutionId,
    pub workflow_id: WorkflowId,
    pub workflow_definition: Arc<WorkflowDefinition>,
    pub node_outputs: Arc<RwLock<HashMap<NodeId, NodeOutput>>>,
    pub journal: Arc<ExecutionJournal>,
    pub idempotency: Arc<IdempotencyManager>,
    pub cancellation: CancellationToken,
    // Storage и sandbox — через ports traits, инжектируются в runtime/engine
}

/// Что передаётся между нодами
pub enum NodeOutputData {
    /// Маленькие данные — inline JSON
    Inline(serde_json::Value),
    /// Большие данные → через BlobStore (spill при превышении data limit)
    BlobRef { key: String, size: u64, mime: String },
}

pub struct NodeOutput {
    pub result: NodeOutputData,
    pub status: NodeStatus,
    pub duration: Duration,
}

/// Execution Journal — append-only лог решений
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
**Назначение:** Собственный DSL для выражений. Без eval, с жёсткими safety limits.

```rust
pub struct ExpressionLimits {
    pub max_ast_depth: usize,          // default: 32
    pub max_operations: usize,         // default: 10_000
    pub max_string_length: usize,      // default: 1MB
    pub max_array_length: usize,       // default: 100_000
    pub max_evaluation_time: Duration, // default: 5s
}

// Strict access model: expression видит только разрешённый scope.
// Secrets НЕ доступны через expressions — только по Capability.
let result: serde_json::Value = engine.evaluate(expr, &scope).await?;
```

---

### telemetry
**Назначение:** Объединённая наблюдаемость: eventbus + metrics + logging + tracing.
Events — проекции, НЕ источник истины. ExecutionRepo = truth.

```rust
pub struct Telemetry {
    event_bus: EventBus,
    metrics: MetricsRegistry,
    logger: Logger,
}

// ExecutionRepo transition → emit event как проекция
execution_repo.transition(&id, Running, Completed, journal_entry).await?;
telemetry.emit(ExecutionEvent::Completed { ... }).await?;
```

---

### memory
**Назначение:** Arenas, scoped allocation, tiered caching.

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
**Назначение:** Trait-интерфейсы для всего, что меняется между окружениями.
Core крейты зависят ТОЛЬКО от `ports`. Drivers реализуют `ports`.

```rust
// === Storage: доменные репозитории ===
// ExecutionRepo — ЕДИНСТВЕННЫЙ источник истины о состоянии.
// Нет отдельного "StateStore" в core. Драйверы (Postgres/SQLite)
// реализуют CAS transitions + journal append как нужно для их backend.

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
    /// Драйвер решает как это сделать: SQL transaction (Postgres),
    /// single-writer lock (SQLite), etc.
    async fn transition(
        &self, id: &ExecutionId,
        from: ExecutionStatus, to: ExecutionStatus,
        entry: JournalEntry,
    ) -> Result<bool>;
    async fn get_journal(&self, id: &ExecutionId) -> Result<Vec<JournalEntry>>;
    async fn acquire_lease(&self, id: &ExecutionId, holder: &str, ttl: Duration) -> Result<bool>;
    /// Найти executions с истёкшим lease (для reaper/recovery)
    async fn find_stale_leases(&self, older_than: Duration) -> Result<Vec<ExecutionId>>;
}

#[async_trait]
pub trait CredentialRepo: Send + Sync {
    async fn get(&self, id: &CredentialId, tenant: &TenantId) -> Result<Option<EncryptedCredential>>;
    async fn save(&self, cred: &EncryptedCredential) -> Result<()>;
}

// === Queue ===
// Семантика: at-least-once delivery.
// Consistency достигается через idempotency_key в NodeAttempt.
// Обязательный порядок операций в worker:
//   1. dequeue (task получен, visibility timeout запущен)
//   2. acquire_lease в ExecutionRepo (если distributed)
//   3. journal append: attempt started
//   4. execute action
//   5. journal append: attempt completed/failed
//   6. ack (или nack для retry)
// Если worker упал между 3 и 6 — visibility timeout истечёт,
// задача вернётся в очередь, idempotency_key предотвратит дубли.

#[async_trait]
pub trait TaskQueue: Send + Sync {
    async fn enqueue(&self, task: Task) -> Result<()>;
    /// Dequeue с visibility timeout. Если не ack — задача вернётся.
    async fn dequeue(&self, timeout: Duration) -> Result<Option<Task>>;
    async fn ack(&self, task_id: &str) -> Result<()>;
    async fn nack(&self, task_id: &str) -> Result<()>;
    /// Reaper: claim pending entries у мёртвых consumers (Redis: XAUTOCLAIM)
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
// Единственный trait для sandbox. Никаких параллельных интерфейсов.
// Capability checking — ответственность реализации внутри execute().
// Драйвер (inprocess/wasm) сам проверяет capabilities и enforce-ит limits.
#[async_trait]
pub trait SandboxRunner: Send + Sync {
    /// Выполнить action в sandbox. Драйвер внутри:
    /// 1. Проверяет capabilities (metadata vs granted)
    /// 2. Enforce-ит limits (memory, CPU, wall time)
    /// 3. Выполняет action
    /// 4. Валидирует output
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

Одно ядро, разные drivers + config:

| Аспект | Desktop | Self-host | Cloud |
|--------|---------|-----------|-------|
| Storage | storage-sqlite | storage-postgres (или sqlite) | storage-postgres |
| Blobs | blob-fs | blob-fs или blob-s3 | blob-s3 |
| Queue | queue-memory | queue-memory или queue-redis | queue-redis |
| Sandbox default | capability-gated | capability-gated (internal), isolated (community) | isolated (всегда) |
| Sandbox community | выключено или isolated | isolated (WASM) | isolated (WASM) |
| Secrets | secrets-local | secrets-local | secrets-vault/kms |
| Multi-tenancy | нет | optional | обязательно |
| Cluster | нет | нет | coordinator + workers |
| Auth | нет / local | local users / OAuth | multi-tenant |
| Concurrency | 4 | 64 | 500+ |

**Desktop** не компилирует: postgres, redis, s3, wasmtime.
**Server** выбирает drivers через cargo features.
**Cloud** разделяется на control-plane + worker.
