# Взаимодействие слоев и крейтов в Nebula

## Принципы взаимодействия

### Правила зависимостей
1. **Однонаправленные зависимости** - слои могут зависеть только от слоев ниже
2. **Через интерфейсы** - взаимодействие через трейты из `nebula-core`
3. **Event-driven** - loose coupling через `nebula-eventbus`
4. **Shared types** - общие типы только в `nebula-core`

## Детальные примеры взаимодействия

### 1. nebula-value ↔ nebula-validator

**Направление:** `nebula-validator` зависит от `nebula-value`

```rust
// В nebula-value определены базовые типы
pub enum Value {
    String(StringValue),
    Number(Number),
    Object(ObjectValue),
    // ...
}

pub enum ValueType {
    String { min_length: Option<usize>, max_length: Option<usize> },
    Number { min: Option<f64>, max: Option<f64> },
    // ...
}

// nebula-validator использует эти типы для валидации
use nebula_value::{Value, ValueType};

impl Validator<Value> for TypeValidator {
    async fn validate(&self, value: &Value) -> ValidationResult {
        match (&self.expected_type, value) {
            (ValueType::String { min_length, max_length, .. }, Value::String(s)) => {
                let len = s.len();
                
                if let Some(min) = min_length {
                    if len < *min {
                        return ValidationResult::error(
                            format!("String too short: {} < {}", len, min)
                        );
                    }
                }
                
                if let Some(max) = max_length {
                    if len > *max {
                        return ValidationResult::error(
                            format!("String too long: {} > {}", len, max)
                        );
                    }
                }
                
                ValidationResult::valid()
            }
            (ValueType::Number { min, max }, Value::Number(n)) => {
                let val = n.as_f64();
                
                if let Some(min) = min {
                    if val < *min {
                        return ValidationResult::error(
                            format!("Number {} is less than minimum {}", val, min)
                        );
                    }
                }
                
                ValidationResult::valid()
            }
            (expected, actual) => {
                ValidationResult::error(
                    format!("Type mismatch: expected {:?}, got {:?}", expected, actual.type_name())
                )
            }
        }
    }
}

// Пример использования
let validator = TypeValidator::new(ValueType::String {
    min_length: Some(3),
    max_length: Some(50),
    pattern: Some(r"^[a-zA-Z]+$".to_string()),
});

let value = Value::String("Hello".into());
let result = validator.validate(&value).await;
assert!(result.is_valid());
```

### 2. nebula-validator ↔ nebula-parameter

**Направление:** `nebula-parameter` использует `nebula-validator` для проверки параметров

```rust
// nebula-parameter определяет параметры с валидацией
use nebula_validator::{Validator, EmailValidator, RangeValidator, CompositeValidator};
use nebula_value::Value;

pub struct Parameter {
    pub name: String,
    pub parameter_type: ParameterType,
    pub validators: Vec<Box<dyn Validator<Value>>>,
}

impl ParameterCollection {
    pub async fn validate_parameters(
        &self, 
        values: &HashMap<String, Value>
    ) -> Result<(), ParameterError> {
        for (name, param) in &self.parameters {
            let value = values.get(name)
                .ok_or_else(|| ParameterError::Missing(name.clone()))?;
            
            // Используем валидаторы из nebula-validator
            for validator in &param.validators {
                let result = validator.validate(value).await;
                
                if !result.is_valid() {
                    return Err(ParameterError::ValidationFailed {
                        parameter: name.clone(),
                        errors: result.errors,
                    });
                }
            }
        }
        Ok(())
    }
}

// Создание параметра с валидаторами
let email_param = Parameter {
    name: "user_email".to_string(),
    parameter_type: ParameterType::String,
    validators: vec![
        Box::new(RequiredValidator),
        Box::new(EmailValidator),
        Box::new(LengthValidator::max(255)),
    ],
};

// Использование в Action
#[derive(Parameters)]
pub struct UserRegistrationParams {
    #[parameter(validators = [EmailValidator, RequiredValidator])]
    pub email: String,
    
    #[parameter(validators = [RangeValidator::new(18, 150)])]
    pub age: u8,
}
```

### 3. nebula-expression ↔ nebula-execution ↔ nebula-value

**Цепочка:** Expression вычисляется в контексте Execution и возвращает Value

```rust
// nebula-execution предоставляет контекст
pub struct ExecutionContext {
    pub node_outputs: Arc<RwLock<HashMap<NodeId, NodeOutput>>>,
    pub variables: Arc<RwLock<HashMap<String, Value>>>,
    expression_engine: Arc<ExpressionEngine>,
}

impl ExecutionContext {
    pub async fn evaluate_expression(&self, expr: &str) -> Result<Value, ExpressionError> {
        // Создаем контекст для expression engine
        let eval_context = ExpressionContext {
            get_node_output: Box::new(|node_id| {
                self.node_outputs.read().await
                    .get(&node_id)
                    .map(|output| output.result.clone())
            }),
            get_variable: Box::new(|var_name| {
                self.variables.read().await
                    .get(&var_name)
                    .cloned()
            }),
            get_user_context: Box::new(|| {
                Value::Object(hashmap! {
                    "id" => Value::String(self.user_id.clone()),
                    "account" => Value::String(self.account_id.clone()),
                })
            }),
        };
        
        // Expression engine парсит и вычисляет
        self.expression_engine.evaluate(expr, eval_context).await
    }
}

// nebula-expression использует данные из контекста
impl ExpressionEngine {
    pub async fn evaluate(
        &self, 
        expr: &str, 
        context: ExpressionContext
    ) -> Result<Value, ExpressionError> {
        let ast = self.parse(expr)?;
        self.eval_ast(&ast, &context).await
    }
    
    async fn eval_ast(&self, ast: &Ast, ctx: &ExpressionContext) -> Result<Value> {
        match ast {
            Ast::NodeReference { node_id, field_path } => {
                // Получаем данные через контекст
                let node_output = (ctx.get_node_output)(node_id).await?;
                self.extract_field(&node_output, field_path)
            }
            Ast::Variable(name) => {
                (ctx.get_variable)(name).await
                    .ok_or_else(|| ExpressionError::VariableNotFound(name.clone()))
            }
            Ast::BinaryOp { left, op, right } => {
                let left_val = self.eval_ast(left, ctx).await?;
                let right_val = self.eval_ast(right, ctx).await?;
                self.apply_operator(op, &left_val, &right_val)
            }
            // ...
        }
    }
}

// Пример полного flow
let context = ExecutionContext::new(/* ... */);

// Сохраняем результат узла
context.node_outputs.write().await.insert(
    NodeId::new("fetch_user"),
    NodeOutput {
        result: Value::Object(hashmap! {
            "email" => Value::String("user@example.com"),
            "age" => Value::Number(25),
        }),
    }
);

// Вычисляем expression
let email = context.evaluate_expression("$nodes.fetch_user.result.email").await?;
assert_eq!(email, Value::String("user@example.com"));
```

### 4. nebula-action ↔ nebula-resource ↔ nebula-credential

**Цепочка:** Action запрашивает Resource, который может требовать Credential

```rust
// nebula-action определяет что нужно
#[derive(Action)]
#[resources([DatabaseResource])]
#[credentials(["database"])]
pub struct QueryUserAction;

impl ProcessAction for QueryUserAction {
    async fn execute(&self, input: Input, ctx: &ActionContext) -> Result<Output> {
        // Action запрашивает resource
        let db = ctx.get_resource::<DatabaseResource>().await?;
        
        // Resource внутри использует credential
        let users = db.query("SELECT * FROM users").await?;
        Ok(users)
    }
}

// nebula-resource создает ресурс с credential
pub struct DatabaseResource;

impl Resource for DatabaseResource {
    type Instance = DatabaseInstance;
    
    async fn create(&self, ctx: &ResourceContext) -> Result<Self::Instance> {
        // Resource запрашивает credential из контекста
        let cred = ctx.get_credential("database").await?;
        
        // Используем credential для создания подключения
        let connection = match cred {
            AuthData::Basic { username, password } => {
                let conn_string = format!(
                    "postgres://{}:{}@localhost/db",
                    username,
                    password.expose_secret()
                );
                PgConnection::connect(&conn_string).await?
            }
            _ => return Err(ResourceError::InvalidCredentialType),
        };
        
        Ok(DatabaseInstance { connection })
    }
}

// ActionContext связывает все вместе
impl ActionContext {
    pub async fn get_resource<R: Resource>(&self) -> Result<R::Instance> {
        // Определяем scope
        let scope = self.determine_resource_scope::<R>();
        
        // ResourceManager проверяет, нужен ли credential
        let required_creds = R::required_credentials();
        
        // Создаем ResourceContext с доступом к credentials
        let resource_ctx = ResourceContext {
            scope,
            credential_resolver: Box::new(move |cred_id| {
                self.execution_context.get_credential(cred_id)
            }),
        };
        
        // ResourceManager создает или возвращает существующий
        self.resource_manager.get_or_create::<R>(resource_ctx).await
    }
}
```

### 5. nebula-eventbus ↔ nebula-execution ↔ nebula-log

**Event flow:** Execution генерирует события, Log их записывает

```rust
// nebula-execution генерирует события
impl ExecutionContext {
    pub async fn start_node(&self, node_id: NodeId) -> Result<()> {
        // Emit event через eventbus
        self.event_bus.publish(NodeEvent::Started {
            execution_id: self.execution_id.clone(),
            workflow_id: self.workflow_id.clone(),
            node_id: node_id.clone(),
            timestamp: SystemTime::now(),
        }).await?;
        
        // Также логируем
        self.logger.info(&format!("Starting node {}", node_id));
        
        Ok(())
    }
}

// nebula-log подписывается на события
pub struct EventLogger {
    logger: Logger,
}

impl EventLogger {
    pub fn subscribe_to_events(event_bus: &EventBus) {
        // Подписываемся на все execution события
        event_bus.subscribe(|event: ExecutionEvent| async move {
            match event {
                ExecutionEvent::Started { execution_id, workflow_id, .. } => {
                    log::info!(
                        target: "execution",
                        execution_id = %execution_id,
                        workflow_id = %workflow_id,
                        "Execution started"
                    );
                }
                ExecutionEvent::Failed { execution_id, error, .. } => {
                    log::error!(
                        target: "execution", 
                        execution_id = %execution_id,
                        error = %error,
                        "Execution failed"
                    );
                }
                // ...
            }
        });
        
        // Подписываемся на node события
        event_bus.subscribe(|event: NodeEvent| async move {
            match event {
                NodeEvent::Started { node_id, .. } => {
                    log::debug!("Node {} started", node_id);
                }
                NodeEvent::Completed { node_id, duration, .. } => {
                    log::info!("Node {} completed in {:?}", node_id, duration);
                }
                // ...
            }
        });
    }
}

// nebula-metrics тоже слушает события
pub struct MetricsCollector;

impl MetricsCollector {
    pub fn subscribe_to_events(event_bus: &EventBus) {
        event_bus.subscribe(|event: NodeEvent| async move {
            match event {
                NodeEvent::Completed { duration, .. } => {
                    metrics::histogram!("node_duration_seconds", duration.as_secs_f64());
                    metrics::increment_counter!("nodes_completed_total");
                }
                NodeEvent::Failed { .. } => {
                    metrics::increment_counter!("nodes_failed_total");
                }
                // ...
            }
        });
    }
}
```

### 6. nebula-config → Все крейты

**Паттерн:** Config инжектируется во все крейты через DI

```rust
// nebula-config определяет конфигурации
#[derive(Config)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub timeout: Duration,
}

#[derive(Config)]
pub struct CacheConfig {
    pub backend: CacheBackend,
    pub ttl: Duration,
    pub max_size: usize,
}

// Каждый крейт получает свою конфигурацию
impl DatabaseResource {
    pub fn new(config: DatabaseConfig) -> Self {
        Self { config }
    }
    
    async fn create_instance(&self) -> DatabaseInstance {
        let pool = PgPool::builder()
            .max_connections(self.config.max_connections)
            .connect_timeout(self.config.timeout)
            .build(&self.config.url)
            .await?;
            
        DatabaseInstance { pool }
    }
}

impl CacheResource {
    pub fn new(config: CacheConfig) -> Self {
        let backend = match config.backend {
            CacheBackend::Redis => RedisCache::new(),
            CacheBackend::Memory => MemoryCache::new(config.max_size),
        };
        
        Self { backend, config }
    }
}

// Центральная инициализация
pub struct Application {
    config_manager: ConfigManager,
    resource_manager: ResourceManager,
}

impl Application {
    pub async fn initialize() -> Self {
        // Загружаем конфигурацию
        let config_manager = ConfigManager::from_file("config.toml").await?;
        
        // Создаем resource manager с конфигурацией
        let resource_manager = ResourceManager::new();
        
        // Регистрируем ресурсы с их конфигурациями
        let db_config: DatabaseConfig = config_manager.get()?;
        resource_manager.register(DatabaseResource::new(db_config));
        
        let cache_config: CacheConfig = config_manager.get()?;
        resource_manager.register(CacheResource::new(cache_config));
        
        Self { config_manager, resource_manager }
    }
}
```

### 7. nebula-resilience обертывает другие крейты

**Паттерн:** Resilience patterns оборачивают вызовы других крейтов

```rust
// nebula-action использует resilience
impl ProcessAction for ExternalApiAction {
    async fn execute(&self, input: Input, ctx: &ActionContext) -> Result<Output> {
        // Получаем resilience executor из контекста
        let resilience = ctx.get_resilience_executor();
        
        // Оборачиваем внешний вызов в resilience patterns
        let result = resilience
            .with_circuit_breaker()
            .with_retry(RetryPolicy::exponential())
            .with_timeout(Duration::from_secs(30))
            .execute(async {
                // Реальный вызов API
                let client = ctx.get_resource::<HttpClient>().await?;
                client.post(&input.url, &input.body).await
            })
            .await?;
        
        Ok(result)
    }
}

// nebula-resource использует resilience для health checks
impl ResourceInstance for DatabaseInstance {
    async fn health_check(&self) -> HealthStatus {
        let resilience = ResilientExecutor::new()
            .with_timeout(Duration::from_secs(5))
            .with_retry(RetryPolicy::fixed(3, Duration::from_millis(100)));
        
        match resilience.execute(async {
            self.pool.acquire().await?.ping().await
        }).await {
            Ok(_) => HealthStatus::Healthy,
            Err(e) => HealthStatus::Unhealthy { reason: e.to_string() },
        }
    }
}
```

### 8. nebula-memory управляет памятью для других крейтов

**Паттерн:** Memory manager предоставляет scoped allocation

```rust
// nebula-execution использует memory для кеширования
impl ExecutionContext {
    pub async fn cache_node_result(&self, node_id: NodeId, result: Value) -> Result<()> {
        // Allocate в execution-scoped memory
        let cached = self.memory_manager
            .allocate_scoped(result, MemoryScope::Execution(self.execution_id))
            .await?;
        
        self.node_cache.insert(node_id, cached);
        Ok(())
    }
}

// nebula-expression кеширует compiled expressions
impl ExpressionEngine {
    pub async fn compile_and_cache(&self, expr: &str) -> Result<CompiledExpression> {
        // Проверяем кеш
        if let Some(compiled) = self.memory_manager.get_cached(expr).await {
            return Ok(compiled);
        }
        
        // Компилируем
        let compiled = self.compile(expr)?;
        
        // Кешируем в global scope (т.к. expressions переиспользуются)
        self.memory_manager
            .cache(expr, compiled.clone(), MemoryScope::Global)
            .await?;
        
        Ok(compiled)
    }
}
```

### 9. Cross-cutting concerns через middleware pattern

```rust
// nebula-tenant, nebula-log, nebula-metrics работают через middleware
pub struct ExecutionPipeline {
    middlewares: Vec<Box<dyn ExecutionMiddleware>>,
}

#[async_trait]
pub trait ExecutionMiddleware: Send + Sync {
    async fn before_execution(&self, ctx: &mut ExecutionContext) -> Result<()>;
    async fn after_execution(&self, ctx: &ExecutionContext, result: &ExecutionResult) -> Result<()>;
}

// Tenant middleware
pub struct TenantMiddleware;

impl ExecutionMiddleware for TenantMiddleware {
    async fn before_execution(&self, ctx: &mut ExecutionContext) -> Result<()> {
        // Извлекаем tenant context
        let tenant_id = ctx.extract_tenant_id()?;
        let tenant = TenantManager::get(tenant_id).await?;
        
        // Проверяем квоты
        tenant.check_quota(ResourceType::Execution).await?;
        
        // Инжектируем в контекст
        ctx.set_tenant_context(tenant);
        Ok(())
    }
}

// Logging middleware
pub struct LoggingMiddleware;

impl ExecutionMiddleware for LoggingMiddleware {
    async fn before_execution(&self, ctx: &mut ExecutionContext) -> Result<()> {
        log::info!("Starting execution {}", ctx.execution_id);
        Ok(())
    }
    
    async fn after_execution(&self, ctx: &ExecutionContext, result: &ExecutionResult) -> Result<()> {
        match result {
            Ok(output) => log::info!("Execution {} completed", ctx.execution_id),
            Err(e) => log::error!("Execution {} failed: {}", ctx.execution_id, e),
        }
        Ok(())
    }
}

// Metrics middleware
pub struct MetricsMiddleware;

impl ExecutionMiddleware for MetricsMiddleware {
    async fn before_execution(&self, ctx: &mut ExecutionContext) -> Result<()> {
        ctx.set_metric_start_time(Instant::now());
        metrics::increment_gauge!("executions_active", 1.0);
        Ok(())
    }
    
    async fn after_execution(&self, ctx: &ExecutionContext, result: &ExecutionResult) -> Result<()> {
        let duration = ctx.get_metric_start_time().elapsed();
        metrics::histogram!("execution_duration", duration);
        metrics::decrement_gauge!("executions_active", 1.0);
        
        if result.is_err() {
            metrics::increment_counter!("executions_failed");
        }
        Ok(())
    }
}
```

## Диаграмма взаимодействия слоев

```
┌─────────────────────────────────────────────────┐
│              Presentation Layer                  │
│                                                  │
│  API/CLI/UI вызывают Engine через SDK           │
└────────────────────┬────────────────────────────┘
                     │ использует
┌────────────────────▼────────────────────────────┐
│              Developer Tools                     │
│                                                  │
│  SDK реэкспортирует публичное API крейтов       │
└────────────────────┬────────────────────────────┘
                     │ зависит от
┌────────────────────▼────────────────────────────┐
│           Multi-tenancy & Clustering             │
│                                                  │
│  Оборачивает Engine для распределенной работы   │
└────────────────────┬────────────────────────────┘
                     │ использует
┌────────────────────▼────────────────────────────┐
│              Business Logic                      │
│                                                  │
│  Registry предоставляет Actions для Engine      │
│  Resources управляются через ResourceManager     │
└────────────────────┬────────────────────────────┘
                     │ координирует
┌────────────────────▼────────────────────────────┐
│              Execution Layer                     │
│                                                  │
│  Engine orchestrates Workers                     │
│  Runtime executes Actions                        │
└────────────────────┬────────────────────────────┘
                     │ выполняет
┌────────────────────▼────────────────────────────┐
│                Node Layer                        │
│                                                  │
│  Actions используют Parameters и Credentials     │
└────────────────────┬────────────────────────────┘
                     │ базируется на
┌────────────────────▼────────────────────────────┐
│                Core Layer                        │
│                                                  │
│  Workflow definitions, Values, Expressions       │
│  EventBus для loose coupling                     │
└────────────────────┬────────────────────────────┘
                     │ использует
┌────────────────────▼────────────────────────────┐
│          Cross-Cutting Concerns                  │
│                                                  │
│  Config, Log, Metrics, Errors, Resilience       │
│  Validator, Locale, System monitoring            │
└────────────────────┬────────────────────────────┘
                     │ хранит в
┌────────────────────▼────────────────────────────┐
│            Infrastructure Layer                  │
│                                                  │
│  Storage abstractions, Binary serialization      │
└─────────────────────────────────────────────────┘
```

## Ключевые паттерны взаимодействия

1. **Dependency Injection** - конфигурация и ресурсы инжектируются сверху вниз
2. **Event-driven** - слои общаются через события для loose coupling
3. **Middleware chain** - cross-cutting concerns через цепочку middleware
4. **Type safety** - `nebula-value` обеспечивает type safety через все слои
5. **Context propagation** - `ExecutionContext` несет информацию через все вызовы
6. **Resource scoping** - автоматическое управление жизненным циклом на разных уровнях