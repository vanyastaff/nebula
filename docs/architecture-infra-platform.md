# Nebula Architecture — Infrastructure & Platform

## Ports Layer (trait interfaces)

Все traits определены в крейте `ports`. Core зависит ТОЛЬКО от `ports`.
Drivers реализуют `ports`. Bins собирают core + нужные drivers.

> Подробные определения traits — см. [architecture-overview.md](architecture-overview.md), секция Ports.

---

## Drivers (реализации ports)

### drivers/storage-sqlite
**Назначение:** SQLite backend для desktop и лёгкого self-host.

```rust
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl WorkflowRepo for SqliteStorage { /* ... */ }
impl ExecutionRepo for SqliteStorage {
    async fn transition(
        &self, id: &ExecutionId, from: ExecutionStatus, to: ExecutionStatus,
        entry: JournalEntry,
    ) -> Result<bool> {
        // SQLite: UPDATE ... WHERE status = $from
        // + INSERT INTO journal ...
        // В одной транзакции (native SQL transaction)
        Ok(true)
    }
}
impl CredentialRepo for SqliteStorage { /* ... */ }
```

### drivers/storage-postgres
**Назначение:** PostgreSQL для self-host и cloud.

```rust
pub struct PostgresStorage {
    pool: sqlx::Pool<Postgres>,
}

impl WorkflowRepo for PostgresStorage { /* ... */ }
impl ExecutionRepo for PostgresStorage {
    async fn transition(...) -> Result<bool> {
        // Postgres: native transaction + advisory locks для lease
    }
    async fn acquire_lease(&self, id: &ExecutionId, holder: &str, ttl: Duration) -> Result<bool> {
        // pg_advisory_lock для leader election / execution ownership
    }
}
```

### drivers/queue-memory
**Назначение:** In-memory bounded queue для desktop.

```rust
pub struct MemoryQueue {
    sender: tokio::sync::mpsc::Sender<Task>,
    receiver: Arc<Mutex<tokio::sync::mpsc::Receiver<Task>>>,
}

impl TaskQueue for MemoryQueue {
    async fn enqueue(&self, task: Task) -> Result<()> {
        self.sender.try_send(task).map_err(|_| QueueError::Full)?;
        Ok(())
    }
    async fn dequeue(&self, timeout: Duration) -> Result<Option<Task>> {
        tokio::time::timeout(timeout, self.receiver.lock().await.recv())
            .await
            .ok()
            .flatten()
            .pipe(Ok)
    }
}
```

### drivers/queue-redis
**Назначение:** Redis queue для self-host и cloud. Durable, с ack/nack.

```rust
pub struct RedisQueue {
    client: redis::Client,
    stream_key: String,
    group: String,
}

impl TaskQueue for RedisQueue {
    // Redis Streams: XADD / XREADGROUP / XACK
}
```

### drivers/blob-fs
**Назначение:** Filesystem blob storage для desktop и self-host.

```rust
pub struct FsBlobStore {
    root: PathBuf,
}
impl BlobStore for FsBlobStore { /* tokio::fs read/write */ }
```

### drivers/blob-s3
**Назначение:** S3/MinIO для self-host и cloud.

```rust
pub struct S3BlobStore {
    client: aws_sdk_s3::Client,
    bucket: String,
}
impl BlobStore for S3BlobStore { /* put_object / get_object */ }
```

### drivers/secrets-local
**Назначение:** Encrypted local keystore. AES-GCM + master key.

```rust
pub struct LocalSecretsStore {
    db_path: PathBuf,
    cipher: Aes256Gcm,
}
impl SecretsStore for LocalSecretsStore { /* encrypt/decrypt locally */ }
```

### drivers/sandbox-inprocess
**Назначение:** In-process sandbox с capability checks (без WASM).

```rust
pub struct InProcessSandbox {
    capability_checker: CapabilityChecker,
}

impl SandboxRunner for InProcessSandbox {
    async fn execute(&self, action: &dyn Action, ctx: SandboxedContext) -> Result<serde_json::Value> {
        // Проверяем capabilities, выполняем в том же процессе
        action.execute_dynamic(ctx).await
    }
}
```

### drivers/sandbox-wasm
**Назначение:** WASM isolation через wasmtime. Для community/hub actions.

```rust
pub struct WasmSandbox {
    engine: wasmtime::Engine,
    memory_limit: usize,
    fuel_limit: u64,
}

impl SandboxRunner for WasmSandbox {
    async fn execute(&self, action: &dyn Action, ctx: SandboxedContext) -> Result<serde_json::Value> {
        // Компилируем/кешируем WASM module
        // Устанавливаем memory + fuel limits
        // Вызываем через wasmtime с capability proxy
    }
}
```

---

## Bins (Distribution Units)

### bins/desktop
**Назначение:** Single-user приложение. Минимальные зависимости.
**Не компилирует:** postgres, redis, s3, wasmtime.

```rust
// desktop/src/main.rs — composition root
fn main() {
    let config = load_config("config.desktop.toml");

    // Собираем drivers
    let storage = SqliteStorage::new(&config.storage.path);
    let blobs = FsBlobStore::new(&config.blobs.root);
    let queue = MemoryQueue::new(config.engine.max_concurrent_executions);
    let sandbox = InProcessSandbox::new();
    let secrets = LocalSecretsStore::new(&config.secrets.path);

    // Собираем engine (core) с drivers через ports traits
    let engine = WorkflowEngine::new(
        Arc::new(storage) as Arc<dyn ExecutionRepo>,
        Arc::new(storage) as Arc<dyn WorkflowRepo>,
        Arc::new(queue) as Arc<dyn TaskQueue>,
        Arc::new(sandbox) as Arc<dyn SandboxRunner>,
        Arc::new(secrets) as Arc<dyn SecretsStore>,
        Arc::new(blobs) as Arc<dyn BlobStore>,
    );

    // Запускаем API на localhost
    start_api("127.0.0.1:8787", engine).await;
}
```

### bins/server
**Назначение:** Self-host, all-in-one. Backend'ы через cargo features.

```rust
// server/src/main.rs
fn main() {
    let config = load_config("config.selfhost.toml");

    // Runtime dispatch по config
    let storage: Arc<dyn ExecutionRepo> = match config.storage.backend {
        #[cfg(feature = "lite")]
        "sqlite" => Arc::new(SqliteStorage::new(&config.storage.path)),
        #[cfg(feature = "full")]
        "postgres" => Arc::new(PostgresStorage::new(&config.storage.url).await),
        _ => panic!("unsupported storage backend"),
    };

    let sandbox: Arc<dyn SandboxRunner> = match config.sandbox_driver.as_str() {
        "inprocess" => Arc::new(InProcessSandbox::new()),
        #[cfg(feature = "full")]
        "wasm" => Arc::new(WasmSandbox::new(config.sandbox.wasm_config)),
        _ => Arc::new(InProcessSandbox::new()),
    };

    let engine = WorkflowEngine::new(storage, queue, sandbox, secrets, blobs);
    start_api("0.0.0.0:8787", engine).await;
}
```

### bins/worker
**Назначение:** Cloud data plane. Получает задачи из queue, выполняет через sandbox.

```rust
// worker/src/main.rs
fn main() {
    let config = load_config("config.cloud.toml");

    let queue = RedisQueue::new(&config.queue.redis_url);
    let sandbox = WasmSandbox::new(config.sandbox.wasm_config);
    let blobs = S3BlobStore::new(&config.blobs);
    let secrets = LocalSecretsStore::new(&config.secrets.path);

    let runtime = ActionRuntime::new(
        Arc::new(sandbox) as Arc<dyn SandboxRunner>,
    );

    let worker = Worker::new(
        Arc::new(queue) as Arc<dyn TaskQueue>,
        runtime,
        config.engine.max_concurrent_executions,
    );

    worker.run().await; // poll queue → execute → ack/nack
}
```

### bins/control-plane
**Назначение:** Cloud control plane. API + scheduler + registry.

```rust
// control-plane/src/main.rs
fn main() {
    let config = load_config("config.cloud.toml");

    let storage = PostgresStorage::new(&config.storage.url).await;
    let queue = RedisQueue::new(&config.queue.redis_url);
    let registry = Registry::new(storage.clone());

    let engine = WorkflowEngine::new(
        Arc::new(storage) as Arc<dyn ExecutionRepo>,
        Arc::new(storage) as Arc<dyn WorkflowRepo>,
        Arc::new(queue) as Arc<dyn TaskQueue>,
        // Control plane не выполняет actions — только планирует
    );

    start_api("0.0.0.0:8787", engine, registry).await;
}
```

---

## Config Presets

### config.desktop.toml
```toml
[mode]
profile = "desktop"

[engine]
max_concurrent_executions = 4
default_timeout = "30s"

[storage]
backend = "sqlite"
path = "./nebula-data/nebula.sqlite"

[blobs]
backend = "filesystem"
root = "./nebula-data/blobs"

[queue]
backend = "memory"

[sandbox]
default_isolation = "capability_gated"
max_memory_mb = 256
max_cpu_time = "30s"

[sandbox.trusted_actions]
patterns = ["builtin.*", "internal.*"]
isolation = "none"

[sandbox.community_actions]
enabled = false

[api]
bind = "127.0.0.1:8787"
```

### config.selfhost.toml
```toml
[mode]
profile = "selfhost"

[engine]
max_concurrent_executions = 64
default_timeout = "60s"

[storage]
backend = "postgres"
url = "postgres://nebula:nebula@localhost:5432/nebula"
max_connections = 20

[blobs]
backend = "filesystem"
root = "/var/lib/nebula/blobs"

[queue]
backend = "redis"
redis_url = "redis://localhost:6379"

[sandbox]
default_isolation = "capability_gated"
max_memory_mb = 512

[sandbox.trusted_actions]
patterns = ["builtin.*", "internal.*"]
isolation = "none"

# Community — ВСЕГДА Isolated (hardcoded, не переопределяется)
[sandbox.community_actions]
enabled = true
patterns = ["community.*", "hub.*"]
isolation = "isolated"

[api]
bind = "0.0.0.0:8787"
```

### config.cloud.toml
```toml
[mode]
profile = "cloud"

[engine]
max_concurrent_executions = 500
default_timeout = "120s"

[storage]
backend = "postgres"
url = "postgres://nebula:nebula@pg:5432/nebula"
max_connections = 200

[blobs]
backend = "s3"
bucket = "nebula-prod-blobs"
region = "us-east-1"

[queue]
backend = "redis"
redis_url = "redis://redis:6379"

[sandbox]
# Cloud: default = isolated (WASM для всех)
default_isolation = "isolated"
max_memory_mb = 1024

[sandbox.trusted_actions]
patterns = ["builtin.*", "internal.*"]
isolation = "none"

# Community — ВСЕГДА Isolated (hardcoded, не переопределяется)
[sandbox.community_actions]
enabled = true
patterns = ["community.*", "hub.*"]
isolation = "isolated"
wasm_memory_limit_mb = 128
wasm_fuel_limit = 1_000_000

[tenant]
enabled = true
strategy = "shared"

[api]
bind = "0.0.0.0:8787"
```

---

## Cluster (Phase 2)

> MVP: single coordinator (control-plane) + workers через queue.
> HA: leader election через storage (postgres advisory locks / etcd).
> Raft — когда single-coordinator станет bottleneck.

### cluster
```rust
pub struct Coordinator {
    id: String,
    lease: Arc<dyn LeaderLease>,
    task_queue: Arc<dyn TaskQueue>,
    execution_repo: Arc<dyn ExecutionRepo>,
}

#[async_trait]
pub trait LeaderLease: Send + Sync {
    async fn try_acquire(&self, holder_id: &str, ttl: Duration) -> Result<bool>;
    async fn renew(&self, holder_id: &str) -> Result<bool>;
    async fn release(&self, holder_id: &str) -> Result<()>;
}

// Реализации:
// - PostgresAdvisoryLease (через drivers/storage-postgres)
// - EtcdLease (позже)

// Phase 2: Raft
#[cfg(feature = "raft-consensus")]
pub struct RaftCluster { /* ... */ }
```

---

## Tenant (Multi-tenancy)

### tenant
```rust
pub struct TenantManager {
    tenants: HashMap<TenantId, TenantInfo>,
    resource_allocator: ResourceAllocator,
}

pub struct TenantQuota {
    pub max_workflows: usize,
    pub max_executions_per_hour: usize,
    pub max_concurrent_executions: usize,
    pub max_storage_gb: usize,
}

pub enum PartitionStrategy {
    RowLevelSecurity,
    SchemaPerTenant,
    DatabasePerTenant,
}
```
