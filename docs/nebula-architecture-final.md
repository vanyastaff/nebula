# Nebula Architecture - Infrastructure & Upper Layers

## Infrastructure Layer

### nebula-storage
**Назначение:** Абстракция над различными системами хранения данных.

**Поддерживаемые backends:**
- PostgreSQL/MySQL - реляционные данные
- MongoDB - документы
- Redis - кеш и сессии
- S3/MinIO - бинарные данные
- Local filesystem - разработка

```rust
// Универсальный trait для хранилищ
#[async_trait]
pub trait Storage: Send + Sync {
    type Key;
    type Value;
    type Error;
    
    async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, Self::Error>;
    async fn set(&self, key: &Self::Key, value: &Self::Value) -> Result<(), Self::Error>;
    async fn delete(&self, key: &Self::Key) -> Result<(), Self::Error>;
    async fn exists(&self, key: &Self::Key) -> Result<bool, Self::Error>;
}

// Специализированные хранилища
pub struct WorkflowStorage {
    backend: Box<dyn Storage<Key = WorkflowId, Value = WorkflowDefinition>>,
    cache: Arc<Cache>,
}

pub struct ExecutionStorage {
    backend: Box<dyn Storage<Key = ExecutionId, Value = ExecutionState>>,
    partitioner: ExecutionPartitioner,  // Для sharding по дате
}

pub struct BinaryStorage {
    backend: Box<dyn Storage<Key = String, Value = Vec<u8>>>,
    compression: CompressionStrategy,
}

// Транзакционность
pub struct TransactionalStorage {
    storage: Arc<dyn Storage>,
    tx_manager: TransactionManager,
}

impl TransactionalStorage {
    pub async fn transaction<F, T>(&self, f: F) -> Result<T>
    where F: FnOnce(&Transaction) -> Future<Output = Result<T>> {
        let tx = self.tx_manager.begin().await?;
        match f(&tx).await {
            Ok(result) => {
                tx.commit().await?;
                Ok(result)
            }
            Err(e) => {
                tx.rollback().await?;
                Err(e)
            }
        }
    }
}
```

---

### nebula-binary
**Назначение:** Эффективная бинарная сериализация для внутренних коммуникаций.

**Форматы:**
- MessagePack - основной формат
- Protobuf - для внешних API
- Bincode - для Rust-only коммуникаций
- JSON - для debug и совместимости

```rust
// Trait для сериализуемых типов
pub trait BinarySerializable: Sized {
    fn serialize_binary(&self) -> Result<Vec<u8>, SerializationError>;
    fn deserialize_binary(data: &[u8]) -> Result<Self, SerializationError>;
}

// Автоматическая имплементация через derive
#[derive(BinarySerializable)]
pub struct ExecutionMessage {
    pub execution_id: ExecutionId,
    pub workflow_id: WorkflowId,
    pub payload: Value,
}

// Оптимизированная передача больших данных
pub struct StreamingSerializer {
    chunk_size: usize,
    compression: Option<CompressionAlgorithm>,
}

impl StreamingSerializer {
    pub fn serialize_stream<T: Serialize>(&self, value: &T) -> impl Stream<Item = Result<Bytes>> {
        stream::unfold(serializer_state, |state| async move {
            // Сериализуем по чанкам
            let chunk = state.next_chunk().await?;
            Some((Ok(chunk), state))
        })
    }
}

// Zero-copy deserialization где возможно
pub struct ZeroCopyDeserializer<'a> {
    buffer: &'a [u8],
    schema: Schema,
}

impl<'a> ZeroCopyDeserializer<'a> {
    pub fn deserialize<T: Deserialize<'a>>(&self) -> Result<T> {
        // Десериализация без копирования для строк и массивов
        deserialize_from_borrowed(self.buffer)
    }
}
```

---

## Multi-Tenancy & Clustering Layer

### nebula-cluster
**Назначение:** Распределенное выполнение с координацией через Raft.

**Ключевые компоненты:**
- Consensus через Raft
- Work distribution
- Fault tolerance
- Auto-scaling

```rust
pub struct ClusterManager {
    node_id: NodeId,
    raft: Raft<ClusterStateMachine>,
    members: Arc<RwLock<HashMap<NodeId, NodeInfo>>>,
    coordinator: WorkflowCoordinator,
}

// Распределение нагрузки
pub enum SchedulingStrategy {
    LeastLoaded,      // Выбираем наименее загруженный узел
    RoundRobin,       // По кругу
    ConsistentHash,   // Для sticky sessions
    AffinityBased,    // Привязка к определенным узлам
}

impl ClusterManager {
    pub async fn execute_workflow(&self, workflow_id: WorkflowId, input: Value) -> Result<ExecutionId> {
        let target_node = self.coordinator.select_node(&workflow_id).await?;
        
        if target_node == self.node_id {
            self.execute_locally(workflow_id, input).await
        } else {
            self.execute_remotely(target_node, workflow_id, input).await
        }
    }
    
    // Обработка отказа узла
    pub async fn handle_node_failure(&self, failed_node: NodeId) {
        let affected_workflows = self.get_workflows_on_node(failed_node).await;
        
        for workflow_id in affected_workflows {
            let new_node = self.coordinator.reschedule(&workflow_id).await?;
            self.migrate_workflow(workflow_id, failed_node, new_node).await?;
        }
        
        self.gossip.broadcast_node_removal(failed_node).await;
    }
    
    // Auto-scaling
    pub async fn auto_scale(&self) {
        let metrics = self.collect_cluster_metrics().await;
        
        if metrics.avg_cpu > 80.0 || metrics.pending_tasks > 100 {
            self.scale_out(1).await?;
        } else if metrics.avg_cpu < 20.0 && self.members.len() > MIN_NODES {
            self.scale_in(1).await?;
        }
    }
}
```

---

### nebula-tenant
**Назначение:** Multi-tenancy с изоляцией на разных уровнях.

**Стратегии изоляции:**
- Shared - общие ресурсы с квотами
- Dedicated - выделенные ресурсы
- Isolated - полная изоляция

```rust
pub struct TenantManager {
    tenants: HashMap<TenantId, TenantInfo>,
    resource_allocator: ResourceAllocator,
    data_partitioner: DataPartitioner,
}

// Стратегии разделения данных
pub enum PartitionStrategy {
    SchemaPerTenant,      // Отдельная схема в БД
    TablePerTenant,       // Префиксы таблиц
    RowLevelSecurity,     // RLS политики
    DatabasePerTenant,    // Отдельная БД
}

// Resource quotas
pub struct TenantQuota {
    max_workflows: usize,
    max_executions_per_hour: usize,
    max_storage_gb: usize,
    max_concurrent_executions: usize,
    cpu_shares: f32,
    memory_limit_mb: usize,
}

// Enforcement
impl TenantContext {
    pub async fn check_quota(&self, resource: ResourceType) -> Result<()> {
        let usage = self.get_current_usage(resource).await?;
        let limit = self.quota.get_limit(resource);
        
        if usage >= limit {
            return Err(QuotaExceeded { resource, usage, limit });
        }
        Ok(())
    }
}

// Middleware для автоматической инъекции контекста
pub async fn tenant_middleware(req: Request, next: Next) -> Response {
    let tenant_id = extract_tenant_id(&req)?;
    let tenant_context = load_tenant_context(tenant_id).await?;
    
    req.extensions_mut().insert(tenant_context);
    next.call(req).await
}
```

---

## Developer Tools Layer

### nebula-sdk
**Назначение:** Публичное API для разработчиков.

**Модули:**
- `prelude` - часто используемые типы
- `action` - разработка Actions
- `workflow` - создание Workflows
- `testing` - утилиты тестирования

```rust
// nebula-sdk/src/prelude.rs
pub use nebula_action::{Action, SimpleAction, ProcessAction};
pub use nebula_workflow::{WorkflowBuilder, NodeBuilder};
pub use nebula_value::{Value, json};
pub use nebula_parameter::{Parameters, Parameter};

// Удобные макросы
#[macro_export]
macro_rules! workflow {
    ($name:expr => {
        $($node:ident: $action:expr $(=> $next:ident)?)*
    }) => {
        WorkflowBuilder::new($name)
            $(.add_node(stringify!($node), $action))*
            $($(.connect(stringify!($node), stringify!($next)))?)*
            .build()
    };
}

// Пример использования SDK
use nebula_sdk::prelude::*;

#[derive(Action)]
#[action(id = "my.custom_action")]
pub struct MyAction;

impl SimpleAction for MyAction {
    type Input = MyInput;
    type Output = MyOutput;
    
    async fn execute_simple(&self, input: Self::Input, ctx: &ActionContext) -> Result<Self::Output> {
        // Implementation
    }
}

let workflow = workflow!("my-workflow" => {
    input: "validation.input" => process
    process: "my.custom_action" => output
    output: "notification.send"
});
```

---

### nebula-derive
**Назначение:** Процедурные макросы для code generation.

**Макросы:**
- `#[derive(Action)]` - автогенерация Action boilerplate
- `#[derive(Parameters)]` - генерация параметров
- `#[derive(Workflow)]` - декларативные workflows
- `#[derive(Resource)]` - resource definitions

```rust
// Макрос Action генерирует
#[derive(Action)]
#[action(id = "test.action", name = "Test Action")]
pub struct TestAction;

// Превращается в:
impl Action for TestAction {
    fn metadata(&self) -> &ActionMetadata {
        static METADATA: Lazy<ActionMetadata> = Lazy::new(|| {
            ActionMetadata {
                id: ActionId::new("test.action"),
                name: "Test Action".to_string(),
                // ...
            }
        });
        &METADATA
    }
    // ... остальная имплементация
}
```

---

### nebula-testing
**Назначение:** Инструменты для тестирования workflows и actions.

```rust
pub struct WorkflowTestHarness {
    engine: MockEngine,
    resources: MockResourceManager,
}

impl WorkflowTestHarness {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn with_mock_action(mut self, id: &str, handler: impl Fn(Value) -> Value) -> Self {
        self.engine.register_mock(id, handler);
        self
    }
    
    pub async fn execute(&self, workflow: WorkflowDefinition, input: Value) -> TestResult {
        let execution = self.engine.execute(workflow, input).await?;
        
        TestResult {
            output: execution.output,
            events: execution.events,
            metrics: execution.metrics,
            node_outputs: execution.node_outputs,
        }
    }
}

// Тестирование Action
#[tokio::test]
async fn test_email_action() {
    let harness = ActionTestHarness::new()
        .with_credential("smtp", mock_smtp_credential())
        .with_resource::<MockEmailClient>();
    
    let result = harness.execute::<EmailAction>(EmailInput {
        to: "test@example.com",
        subject: "Test",
        body: "Hello",
    }).await;
    
    assert!(result.is_success());
    assert_eq!(result.output.message_id, "mock-123");
}

// Тестирование Workflow
#[tokio::test]
async fn test_registration_workflow() {
    let harness = WorkflowTestHarness::new()
        .with_mock_action("validation.user", |input| {
            json!({ "validated": true, "data": input })
        })
        .with_mock_action("database.insert", |input| {
            json!({ "id": 123, "created": true })
        });
    
    let result = harness.execute(
        registration_workflow(),
        json!({ "email": "user@example.com" })
    ).await;
    
    assert_eq!(result.node_outputs["create_user"]["id"], 123);
}
```

---

## Presentation Layer

### nebula-api
**Назначение:** REST/GraphQL API для управления workflows.

```rust
// REST endpoints
pub fn configure_routes(cfg: &mut ServiceConfig) {
    cfg
        // Workflows
        .route("/workflows", post(create_workflow))
        .route("/workflows/{id}", get(get_workflow))
        .route("/workflows/{id}/execute", post(execute_workflow))
        
        // Executions
        .route("/executions", get(list_executions))
        .route("/executions/{id}", get(get_execution))
        .route("/executions/{id}/cancel", post(cancel_execution))
        
        // Actions
        .route("/actions", get(list_actions))
        .route("/actions/search", get(search_actions));
}

// GraphQL schema
#[derive(GraphQLObject)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub nodes: Vec<Node>,
    pub executions: Vec<Execution>,
}

#[derive(GraphQLQuery)]
impl Query {
    async fn workflow(&self, id: String) -> Result<Workflow> {
        // Implementation
    }
    
    async fn search_actions(&self, query: String) -> Vec<Action> {
        // Implementation
    }
}
```

---

### nebula-ui
**Назначение:** Web interface для визуального создания и мониторинга workflows.

**Компоненты:**
- Workflow designer (drag-and-drop)
- Execution monitor
- Action catalog
- Metrics dashboard

---

### nebula-cli
**Назначение:** Command-line interface для управления Nebula.

```bash
# Workflow management
nebula workflow deploy my-workflow.yaml
nebula workflow execute user-registration --input data.json
nebula workflow list --filter "status=active"

# Execution monitoring  
nebula execution watch exec-123
nebula execution logs exec-123 --follow
nebula execution cancel exec-123

# Action development
nebula action create --template simple
nebula action test my-action --input test.json
nebula action publish my-action

# Cluster management
nebula cluster status
nebula cluster add-node node-4
nebula cluster rebalance
```

---

### nebula-hub
**Назначение:** Marketplace для sharing Actions и Workflows.

```rust
pub struct Hub {
    registry: PackageRegistry,
    storage: PackageStorage,
}

// Publishing
nebula hub publish my-actions-pack v1.0.0
nebula hub install slack-integration

// Package format
pub struct Package {
    pub name: String,
    pub version: Version,
    pub actions: Vec<ActionDefinition>,
    pub workflows: Vec<WorkflowTemplate>,
    pub dependencies: Vec<Dependency>,
}
```

---

## Полный пример использования

```rust
// 1. Создаем Action
#[derive(Action)]
#[action(id = "weather.fetch")]
pub struct WeatherAction;

impl SimpleAction for WeatherAction {
    type Input = WeatherInput;
    type Output = WeatherData;
    
    async fn execute_simple(&self, input: Self::Input, ctx: &ActionContext) -> Result<Self::Output> {
        let api_key = ctx.get_credential("weather_api").await?;
        let client = WeatherClient::new(api_key);
        Ok(client.get_weather(&input.city).await?)
    }
}

// 2. Создаем Workflow
let weather_workflow = WorkflowBuilder::new("weather-notification")
    .add_node("fetch", "weather.fetch")
    .add_node("check", "condition.check")
    .add_node("notify", "notification.send")
    .connect("fetch", "check")
    .connect_conditional("check", "notify", "$nodes.fetch.result.temp > 30")
    .build();

// 3. Деплоим и запускаем
let engine = WorkflowEngine::new(config);
engine.deploy_workflow(weather_workflow).await?;

let execution = engine.execute_workflow(
    "weather-notification",
    json!({ "city": "Moscow" }),
    ExecutionOptions::default(),
).await?;

// 4. Мониторим выполнение
execution.on_complete(|result| {
    println!("Workflow completed: {:?}", result);
});
```