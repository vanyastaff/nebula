# Nebula Architecture - Business Logic & Cross-Cutting Concerns

## Business Logic Layer

### nebula-resource
**Назначение:** Управление жизненным циклом долгоживущих ресурсов с учетом scopes.

**Ключевые концепции:**
- Scoped resources (Global/Workflow/Execution/Action)
- Connection pooling
- Health monitoring
- Automatic cleanup

```rust
// Определение ресурса
#[derive(Resource)]
#[resource(
    id = "database",
    name = "Database Connection Pool",
    lifecycle = "global"  // Один экземпляр на все приложение
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

// Различные scopes ресурсов
#[derive(Resource)]
#[resource(lifecycle = "execution")]
pub struct LoggerResource;  // Новый logger для каждого execution

#[derive(Resource)]
#[resource(lifecycle = "workflow")]
pub struct MetricsCollectorResource;  // Один collector на workflow

// Использование в Action
impl ProcessAction for DatabaseQueryAction {
    async fn execute(&self, input: Input, ctx: &ActionContext) -> Result<Output> {
        // Получаем ресурс с правильным scope
        let db = ctx.get_resource::<DatabaseResource>().await?;  // Global
        let logger = ctx.get_resource::<LoggerResource>().await?;  // Per execution
        let metrics = ctx.get_resource::<MetricsCollectorResource>().await?;  // Per workflow
        
        logger.info("Executing query");
        let start = Instant::now();
        
        let result = db.query(&input.sql).await?;
        
        metrics.record_query_duration(start.elapsed());
        Ok(result)
    }
}
```

---

### nebula-registry
**Назначение:** Централизованный реестр Actions, Nodes, Workflows и Resources.

**Ключевые компоненты:**
- ActionRegistry - каталог actions
- NodeRegistry - группировка actions
- WorkflowRegistry - deployed workflows
- Discovery API

```rust
pub struct Registry {
    actions: Arc<RwLock<HashMap<ActionId, ActionMetadata>>>,
    nodes: Arc<RwLock<HashMap<NodeId, NodeDefinition>>>,
    workflows: Arc<RwLock<HashMap<WorkflowId, WorkflowDefinition>>>,
    search_index: Arc<SearchIndex>,
}

impl Registry {
    // Регистрация Action
    pub async fn register_action<A: Action>(&self) -> Result<()> {
        let metadata = A::metadata();
        self.actions.write().await.insert(metadata.id.clone(), metadata);
        self.search_index.index_action(&metadata).await?;
        Ok(())
    }
    
    // Поиск Actions по критериям
    pub async fn search_actions(&self, query: &SearchQuery) -> Vec<ActionMetadata> {
        self.search_index.search(query).await
    }
    
    // Discovery для UI
    pub async fn get_actions_by_category(&self, category: &str) -> Vec<ActionMetadata> {
        self.actions.read().await
            .values()
            .filter(|a| a.category == category)
            .cloned()
            .collect()
    }
    
    // Версионирование
    pub async fn get_compatible_actions(&self, version: &Version) -> Vec<ActionMetadata> {
        self.actions.read().await
            .values()
            .filter(|a| a.version.is_compatible_with(version))
            .cloned()
            .collect()
    }
}

// Auto-registration через макрос
#[register_action]
pub struct MyAction;

// Или програмно
registry.register_action::<EmailSendAction>().await?;
registry.register_node(slack_node).await?;
```

---

## Cross-Cutting Concerns Layer

### nebula-config
**Назначение:** Унифицированная система конфигурации с hot-reload.

**Ключевые возможности:**
- Множественные форматы (TOML/YAML/JSON)
- Environment variables override
- Hot-reload
- Schema validation

```rust
// Определение конфигурации
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct EngineConfig {
    pub max_concurrent_executions: usize,
    pub default_timeout: Duration,
    pub retry_policy: RetryConfig,
}

impl Configurable for EngineConfig {
    fn config_prefix() -> &'static str { "engine" }
    
    fn validate(&self) -> Result<()> {
        ensure!(self.max_concurrent_executions > 0, "Invalid concurrency");
        Ok(())
    }
}

// config.toml
/*
[engine]
max_concurrent_executions = 100
default_timeout = "5m"

[engine.retry_policy]
max_attempts = 3
strategy = "exponential"

[database]
url = "postgres://localhost/nebula"
max_connections = 50
*/

// Использование
let config = ConfigManager::load("config.toml").await?;
config.enable_hot_reload().await?;

let engine_config: EngineConfig = config.get()?;

// Подписка на изменения
config.on_reload(|new_config| async move {
    engine.update_config(new_config).await;
});
```

---

### nebula-log
**Назначение:** Структурированное логирование с контекстом.

**Ключевые возможности:**
- Structured logging
- Context propagation
- Multiple backends
- Async buffering

```rust
pub struct Logger {
    backend: Box<dyn LogBackend>,
    context: LogContext,
    filters: Vec<LogFilter>,
}

// Контекст автоматически добавляется
impl ExecutionContext {
    pub fn log_info(&self, msg: &str) {
        info!(
            execution_id = %self.execution_id,
            workflow_id = %self.workflow_id,
            node_id = ?self.current_node_id,
            "{}", msg
        );
    }
}

// Различные backends
let console = ConsoleBackend::new().with_colors();
let file = FileBackend::new("app.log").with_rotation(RotationPolicy::Daily);
let elastic = ElasticsearchBackend::new("http://localhost:9200");

// Использование в Action
ctx.log_info("Starting user lookup");
ctx.log_error("Database connection failed", &error);
```

---

### nebula-metrics
**Назначение:** Сбор и экспорт метрик.

**Ключевые компоненты:**
- System metrics
- Business metrics
- Custom metrics
- Export backends

```rust
pub struct MetricsManager {
    registry: Registry,
    collectors: Vec<Box<dyn MetricsCollector>>,
}

// Автоматический сбор метрик
#[derive(Metrics)]
pub struct WorkflowMetrics {
    #[metric(type = "counter")]
    pub executions_total: Counter,
    
    #[metric(type = "histogram", buckets = [0.1, 0.5, 1.0, 5.0, 10.0])]
    pub execution_duration: Histogram,
    
    #[metric(type = "gauge")]
    pub active_executions: Gauge,
}

// Использование
impl ActionContext {
    pub async fn measure<F, T>(&self, name: &str, f: F) -> T 
    where F: Future<Output = T> {
        let start = Instant::now();
        let result = f.await;
        self.metrics.record(name, start.elapsed());
        result
    }
}

// В Action
let user = ctx.measure("database.query", async {
    db.get_user(user_id).await
}).await?;
```

---

### nebula-error
**Назначение:** Унифицированная обработка ошибок с контекстом.

**Ключевые возможности:**
- Error hierarchy
- Context propagation
- Recovery strategies
- Localization support

```rust
#[derive(Error, Debug)]
pub enum NebulaError {
    #[error("Workflow error: {0}")]
    Workflow(#[from] WorkflowError),
    
    #[error("Action error: {0}")]
    Action(#[from] ActionError),
    
    #[error("Resource error: {0}")]
    Resource(#[from] ResourceError),
}

// Контекстные ошибки
pub trait ErrorContextExt {
    fn with_context(self, ctx: &ExecutionContext) -> ContextualError;
}

// Recovery strategies
pub enum ErrorRecoveryStrategy {
    Retry { policy: RetryPolicy },
    Fallback { handler: Box<dyn Fn() -> Result<Value>> },
    Compensate { action: ActionId },
    Skip,
}

// Использование
db.query(sql)
    .await
    .context("Failed to query database")
    .with_context(&ctx)?;
```

---

### nebula-resilience
**Назначение:** Паттерны устойчивости для надежной работы.

**Паттерны:**
- Circuit Breaker
- Retry with backoff
- Bulkhead isolation
- Rate limiting
- Timeout

```rust
// Circuit Breaker
let breaker = CircuitBreaker::new()
    .failure_threshold(5)
    .reset_timeout(Duration::from_secs(60));

let result = breaker.call(async {
    external_api.call().await
}).await?;

// Retry policy
let policy = RetryPolicy::exponential()
    .initial_delay(Duration::from_millis(100))
    .max_attempts(3)
    .max_delay(Duration::from_secs(10));

let result = with_retry!(policy, async {
    unreliable_operation().await
});

// Bulkhead для изоляции
let bulkhead = Bulkhead::new()
    .max_concurrent(10)
    .queue_size(50);

let result = bulkhead.execute(async {
    heavy_operation().await
}).await?;

// Композиция паттернов
let executor = ResilientExecutor::new()
    .with_circuit_breaker(breaker)
    .with_retry(policy)
    .with_bulkhead(bulkhead)
    .with_timeout(Duration::from_secs(30));

let result = executor.execute(async {
    complex_operation().await
}).await?;
```

---

### nebula-validator
**Назначение:** Универсальная система валидации.

**Ключевые возможности:**
- Composable validators
- Async validation
- Custom rules
- Derive macros

```rust
// Встроенные валидаторы
let email = EmailValidator::new();
let range = RangeValidator::new(18, 150);
let pattern = PatternValidator::new(r"^[A-Z][a-z]+$");

// Композиция
let validator = RequiredValidator
    .and(email)
    .and(LengthValidator::max(255));

// Derive валидация
#[derive(Validate)]
pub struct UserInput {
    #[validate(required, email)]
    pub email: String,
    
    #[validate(required, length(min = 8, max = 128))]
    pub password: String,
    
    #[validate(range(min = 18, max = 150))]
    pub age: Option<u8>,
    
    #[validate(custom = "validate_username")]
    pub username: String,
}

// Async валидация
async fn validate_unique_email(email: &str) -> Result<()> {
    let exists = db.email_exists(email).await?;
    ensure!(!exists, "Email already registered");
    Ok(())
}
```

---

### nebula-locale
**Назначение:** Локализация и интернационализация.

```rust
// Fluent формат локализации
/*
# en-US.ftl
welcome = Welcome { $user }!
error-validation = Field { $field } is invalid: { $reason }

# ru-RU.ftl  
welcome = Добро пожаловать, { $user }!
error-validation = Поле { $field } некорректно: { $reason }
*/

// Использование
let locale_manager = LocaleManager::new()
    .add_locale("en-US", "locales/en-US.ftl")
    .add_locale("ru-RU", "locales/ru-RU.ftl");

// Автоматический выбор локали
let msg = t!("welcome", user = "John");

// Локализация ошибок
impl ActionError {
    pub fn localized(&self, locale: &LocaleContext) -> String {
        match self {
            Self::ValidationFailed { field, reason } => {
                t!("error-validation", field = field, reason = reason)
            }
        }
    }
}
```

---

### nebula-system
**Назначение:** Мониторинг системных ресурсов.

```rust
pub struct SystemMonitor {
    system: Arc<RwLock<System>>,
    collectors: Vec<Box<dyn MetricsCollector>>,
}

// Системные метрики
let metrics = monitor.get_current_metrics().await;
println!("CPU: {:.1}%", metrics.cpu.usage_percent);
println!("Memory: {:.1}%", metrics.memory.usage_percent);

// Health checks
let health = HealthChecker::new()
    .add_check(DatabaseHealthCheck)
    .add_check(DiskSpaceHealthCheck { threshold: 90 })
    .add_check(MemoryHealthCheck { threshold: 80 });

let status = health.check_all().await;

// Resource pressure detection
if detector.detect_pressure(&metrics).contains(&ResourcePressure::MemoryCritical) {
    // Trigger cleanup or scale-out
}
```