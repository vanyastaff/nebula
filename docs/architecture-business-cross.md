# Nebula Architecture — Business Logic & Cross-Cutting Concerns

## Business Logic Layer

### resource
**Назначение:** Управление жизненным циклом ресурсов с scopes и policies.

**Правила scope:**
- **Global** — только: пулы соединений (DB, Redis), кеш-клиенты, HTTP клиенты.
  Никогда: stateful объекты, per-user данные.
- **Workflow** — per-workflow агрегаторы (метрики, счётчики).
- **Execution** — per-execution логгеры, временные хранилища.
- **Action** — одноразовые, живут только во время execute().

```rust
#[derive(Resource)]
#[resource(
    id = "database",
    name = "Database Connection Pool",
    lifecycle = "global",
    max_instances = 1,
    idle_timeout = "5m",
    health_check_interval = "30s",
)]
pub struct DatabaseResource;

pub struct DatabaseInstance {
    pool: sqlx::Pool<Postgres>,
    metrics: DatabaseMetrics,
}

impl ResourceInstance for DatabaseInstance {
    async fn health_check(&self) -> HealthStatus {
        match self.pool.acquire().await {
            Ok(_) => HealthStatus::Healthy,
            Err(e) => HealthStatus::Unhealthy { reason: e.to_string() },
        }
    }

    async fn cleanup(&mut self) -> Result<()> {
        self.pool.close().await;
        Ok(())
    }
}

/// Resource policies — обязательны
pub struct ResourcePolicy {
    pub max_instances_per_scope: usize,
    pub idle_timeout: Option<Duration>,
    pub ttl: Option<Duration>,
    pub health_check_interval: Duration,
    /// Привязка к pressure detection из system:
    pub pressure_action: PressureAction,
}

pub enum PressureAction {
    Ignore,
    EvictIdle,
    EvictAll,
    Restart,
}

// Использование в Action
impl ProcessAction for DatabaseQueryAction {
    async fn execute(&self, input: Input, ctx: &ActionContext) -> Result<Output> {
        let db = ctx.get_resource::<DatabaseResource>().await?;    // Global
        let logger = ctx.get_resource::<LoggerResource>().await?;  // Per execution
        let result = db.query(&input.sql).await?;
        Ok(result)
    }
}
```

---

### registry
**Назначение:** Реестр Actions/Workflows с interface versioning и schema hash.

```rust
pub struct ActionMetadata {
    pub id: ActionId,
    pub name: String,
    pub category: String,
    /// Package version (semver)
    pub version: semver::Version,
    /// Interface version — меняется ТОЛЬКО при изменении schema
    pub interface_version: InterfaceVersion,
    /// SHA-256(input_schema + output_schema) — быстрая проверка совместимости
    pub schema_hash: String,
    pub input_schema: serde_json::Value,     // JSON Schema
    pub output_schema: serde_json::Value,
    pub capabilities: Vec<Capability>,
    pub isolation_level: Option<IsolationLevel>,
    pub execution_mode: ExecutionMode,       // Typed | Dynamic
    pub migrations: Vec<SchemaMigration>,
}

pub struct InterfaceVersion {
    pub major: u32,  // Breaking changes
    pub minor: u32,  // Backward-compatible
}

pub enum SchemaMigration {
    RenameParam { from: String, to: String },
    AddDefault { param: String, default: serde_json::Value },
    DeprecateParam { param: String, replacement: Option<String> },
}

pub enum ExecutionMode {
    Typed,    // Input: Deserialize + Validate, Output: Serialize
    Dynamic,  // serde_json::Value + JSON Schema
}

impl Registry {
    pub async fn register_action<A: Action>(&self) -> Result<()> {
        let metadata = A::metadata();

        // Проверяем совместимость при обновлении
        if let Some(existing) = self.actions.read().await.get(&metadata.id) {
            self.check_compatibility(existing, &metadata)?;
        }

        self.actions.write().await.insert(metadata.id.clone(), metadata);
        Ok(())
    }
}

// В WorkflowDefinition — привязка к interface_version
pub struct NodeDefinition {
    pub id: NodeId,
    pub action_id: ActionId,
    pub action_interface_version: InterfaceVersion,
    pub parameters: HashMap<String, ParamValue>,
}
```

---

## Cross-Cutting Concerns Layer

### config
**Назначение:** Конфигурация с presets (desktop/selfhost/cloud).

**Жёсткое разделение:**
- **StaticConfig** — определяет drivers и инфраструктуру. Только при старте процесса.
- **DynamicConfig** — лимиты, таймауты, квоты. Можно менять через hot-reload.

```rust
/// Только при старте. Смена → рестарт процесса.
#[derive(Debug, Clone, Deserialize)]
pub struct StaticConfig {
    pub mode: ModeConfig,
    pub storage: StorageBackendConfig,  // sqlite | postgres
    pub queue: QueueBackendConfig,      // memory | redis
    pub sandbox: SandboxDriverConfig,   // inprocess | wasm
    pub blobs: BlobBackendConfig,       // fs | s3
    pub api: ApiConfig,                 // bind address
}

/// Можно менять на лету.
#[derive(Debug, Clone, Deserialize)]
pub struct DynamicConfig {
    pub engine: EngineConfig,           // max_concurrent, default_timeout
    pub rate_limits: RateLimitConfig,
    pub sandbox_limits: SandboxLimitsConfig, // memory, cpu time
    pub tenant_quotas: HashMap<TenantId, TenantQuota>,
}

pub enum Profile {
    Desktop,
    SelfHost,
    Cloud,
}

// Использование
let static_config = ConfigManager::load_static("config.toml")?;
let dynamic_config = ConfigManager::load_dynamic("config.toml")?;

// Hot-reload — только DynamicConfig
config_manager.enable_hot_reload(|new_dynamic| async move {
    engine.apply_dynamic_config(&new_dynamic);
    // НЕ меняем drivers, storage backend, queue backend на лету
});
```

---

### telemetry
**Назначение:** eventbus + logging + metrics + tracing. Единый крейт.

Events — проекции, НЕ источник истины. `ports::ExecutionRepo` = truth.

```rust
pub struct Telemetry {
    event_bus: EventBus,
    metrics: MetricsRegistry,
}

// EventBus
pub enum ExecutionEvent {
    Started { execution_id: ExecutionId },
    Completed { execution_id: ExecutionId, duration: Duration },
    Failed { execution_id: ExecutionId, error: String },
}

// Metrics
#[derive(Metrics)]
pub struct WorkflowMetrics {
    #[metric(type = "counter")]
    pub executions_total: Counter,
    #[metric(type = "histogram")]
    pub execution_duration: Histogram,
    #[metric(type = "gauge")]
    pub active_executions: Gauge,
}

// Logging с контекстом
impl ExecutionContext {
    pub fn log_info(&self, msg: &str) {
        info!(
            execution_id = %self.execution_id,
            workflow_id = %self.workflow_id,
            "{}", msg
        );
    }
}
```

---

### resilience
**Назначение:** Паттерны устойчивости.

```rust
// Circuit Breaker
let breaker = CircuitBreaker::new(CircuitConfig {
    failure_threshold: 5,
    recovery_timeout: Duration::from_secs(30),
    half_open_requests: 3,
});

let result = breaker.execute(|| async {
    external_service.call().await
}).await?;

// Retry with backoff
let result = retry(
    ExponentialBackoff::new(Duration::from_millis(100), 3),
    || async { flaky_operation().await },
).await?;

// Rate limiter
let limiter = RateLimiter::new(100, Duration::from_secs(60)); // 100 req/min
limiter.acquire().await?;

// Bulkhead
let bulkhead = Bulkhead::new(10); // max 10 concurrent
let _permit = bulkhead.acquire().await?;
```

---

### system
**Назначение:** Кроссплатформенные утилиты, мониторинг ресурсов, pressure detection.

```rust
pub struct SystemMonitor {
    sysinfo: System,
    pressure_hooks: Vec<Box<dyn PressureHandler>>,
}

pub enum PressureLevel {
    Normal,
    Warning,   // >70% memory
    Critical,  // >90% memory
}

// Привязка к resource policies: при Critical → evict idle ресурсы
impl SystemMonitor {
    pub async fn check_pressure(&self) -> PressureLevel {
        let mem_usage = self.sysinfo.used_memory() as f64 / self.sysinfo.total_memory() as f64;
        match mem_usage {
            x if x > 0.9 => PressureLevel::Critical,
            x if x > 0.7 => PressureLevel::Warning,
            _ => PressureLevel::Normal,
        }
    }
}
```

---

### locale
**Назначение:** i18n для ошибок и UI.

```rust
let msg = locale.format("error.workflow_not_found", &args! {
    "workflow_id" => workflow_id,
})?;
```

---

### derive
**Назначение:** Proc macros: `#[derive(Action)]`, `#[derive(Parameters)]`, `#[derive(Resource)]`, `#[derive(Metrics)]`.

Всегда optional dependency — можно использовать trait approach вручную.
