# Nebula Architecture Documentation

## Обзор системы

Nebula - высокопроизводительный workflow engine на Rust, состоящий из 30 модульных крейтов, организованных в четкие архитектурные слои.

### Архитектурные принципы

1. **Типовая безопасность** - максимальное использование системы типов Rust
2. **Модульность** - четкое разделение ответственностей между компонентами
3. **Гибкость разработки** - поддержка простого кода и derive макросов
4. **Atomic Actions** - фокус на переиспользуемые блоки
5. **Smart Resource Management** - различные lifecycle scopes
6. **Expression-driven** - мощная система выражений для динамической логики
7. **Event-Driven** - loose coupling через eventbus
8. **Security Isolation** - sandbox с capability-based доступом для выполнения Actions

### Слои архитектуры

```
┌─────────────────────────────────────────────────────────┐
│                 Presentation Layer                      │
│       (nebula-ui, nebula-api, nebula-cli, nebula-hub)   │
├─────────────────────────────────────────────────────────┤
│                 Developer Tools Layer                   │
│       (nebula-sdk, nebula-derive, nebula-testing)       │
├─────────────────────────────────────────────────────────┤
│            Multi-Tenancy & Clustering Layer             │
│            (nebula-cluster, nebula-tenant)              │
├─────────────────────────────────────────────────────────┤
│                 Business Logic Layer                    │
│         (nebula-resource, nebula-registry)              │
├─────────────────────────────────────────────────────────┤
│                   Execution Layer                       │
│  (nebula-engine, nebula-runtime, nebula-worker,         │
│              nebula-sandbox)                             │
├─────────────────────────────────────────────────────────┤
│                     Node Layer                          │
│  (nebula-node, nebula-action, nebula-parameter,         │
│              nebula-credential)                         │
├─────────────────────────────────────────────────────────┤
│                     Core Layer                          │
│  (nebula-core, nebula-workflow, nebula-execution,       │
│   nebula-memory, nebula-expression,                     │
│   nebula-eventbus, nebula-idempotency)                  │
├─────────────────────────────────────────────────────────┤
│              Cross-Cutting Concerns Layer               │
│  (nebula-config, nebula-log, nebula-metrics,            │
│    nebula-resilience, nebula-system,                    │
│   nebula-validator, nebula-locale)                      │
├─────────────────────────────────────────────────────────┤
│                Infrastructure Layer                     │
│         (nebula-storage, nebula-binary)                 │
└─────────────────────────────────────────────────────────┘
```

---

## Core Layer

### nebula-core
**Назначение:** Базовые типы и трейты, используемые всеми крейтами системы. Предотвращает циклические зависимости.

**Ключевые компоненты:**
- Базовые идентификаторы (ExecutionId, WorkflowId, NodeId)
- Концепция Scope для resource management
- Общие трейты для loose coupling

```rust
// Основные типы
pub struct ExecutionId(Uuid);
pub struct WorkflowId(String);
pub struct NodeId(String);
pub struct UserId(String);
pub struct TenantId(String);

// Универсальный Scope
pub enum ScopeLevel {
    Global,
    Workflow(WorkflowId),
    Execution(ExecutionId),
    Action(ExecutionId, NodeId),
}

// Базовые трейты
pub trait Scoped {
    fn scope(&self) -> &ScopeLevel;
}

pub trait HasContext {
    fn execution_id(&self) -> Option<&ExecutionId>;
    fn workflow_id(&self) -> Option<&WorkflowId>;
    fn tenant_id(&self) -> Option<&TenantId>;
}

// Параметрическое значение — разделяет expressions от литеральных данных.
// Используется в WorkflowDefinition и ParameterCollection.
// После resolve на уровне Execution всё превращается в serde_json::Value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    /// Expression: "$nodes.user_lookup.result.email"
    Expression(Expression),
    /// Template: "Order #{order_id} for {name}"
    Template(TemplateString),
    /// Обычное JSON-значение
    Literal(serde_json::Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expression {
    pub raw: String,  // "$nodes.user_lookup.result.email"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateString {
    pub template: String,
    pub bindings: Vec<String>,
}
```

---

### nebula-workflow
**Назначение:** Декларативное определение workflow - описывает "что нужно делать".

**Ключевые компоненты:**
- WorkflowDefinition - структура workflow
- NodeDefinition - узлы workflow
- Connection - связи между узлами
- Validation - проверка корректности

```rust
// Пример определения workflow
let workflow = WorkflowDefinition {
    id: WorkflowId::new("user-registration"),
    name: "User Registration Process",
    nodes: vec![
        NodeDefinition {
            id: NodeId::new("validate"),
            action_id: ActionId::new("validation.user_data"),
            parameters: params!{
                "email_pattern" => ParamValue::Literal(json!("^[^@]+@[^@]+$")),
                "required_fields" => ParamValue::Literal(json!(["email", "password"]))
            },
        },
        NodeDefinition {
            id: NodeId::new("create_user"),
            action_id: ActionId::new("database.insert"),
            parameters: params!{
                "table" => ParamValue::Literal(json!("users")),
                // Expression — данные из предыдущего узла,
                // резолвится в serde_json::Value на этапе execution
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

### nebula-execution
**Назначение:** Runtime выполнение workflow - управляет "как выполняется".

**Ключевые компоненты:**
- ExecutionContext - контекст выполнения
- ExecutionState - состояние выполнения
- NodeOutput - результаты узлов
- Expression integration

```rust
// Контекст выполнения с интеграцией всех систем
pub struct ExecutionContext {
    pub execution_id: ExecutionId,
    pub workflow_id: WorkflowId,
    pub workflow_definition: Arc<WorkflowDefinition>,
    pub node_outputs: Arc<RwLock<HashMap<NodeId, NodeOutput>>>,
    pub resource_manager: Arc<ResourceManager>,
    pub credential_manager: Arc<CredentialManager>,
    pub expression_engine: Arc<ExpressionEngine>,
}

// NodeOutput хранит результат как serde_json::Value
pub struct NodeOutput {
    pub result: serde_json::Value,
    pub status: NodeStatus,
    pub duration: Duration,
}

// Использование
let context = ExecutionContext::new(workflow_id, execution_id);

// Вычисление expressions — результат serde_json::Value
let user_email: serde_json::Value = context
    .evaluate_expression("$nodes.create_user.result.email")
    .await?;

// Resolve ParamValue перед передачей в Action
let resolved: serde_json::Value = context
    .resolve_param_value(&param_value)
    .await?;

// Получение ресурсов с правильным scope
let database = context.get_resource::<DatabaseResource>().await?;
```

---

### nebula-expression
**Назначение:** Мощный язык выражений для динамической обработки данных.

**Ключевые возможности:**
- Доступ к результатам узлов: `$nodes.user_lookup.result.email`
- Условная логика: `if $user.premium then ... else ...`
- Pipeline операции: `$array | filter(...) | map(...) | sort(...)`
- String interpolation: `"Hello ${user.name}!"`
- Null safety: `$user?.address?.city ?? "Unknown"`

Все значения представлены как `serde_json::Value`. Expressions из `ParamValue::Expression` 
резолвятся на уровне Execution Layer до передачи в Action.

```rust
// Примеры expressions
let examples = vec![
    // Простой доступ
    "$nodes.input.result.user_email",
    
    // Условная логика
    "if $user.premium && $order.amount > 1000 then 'vip' else 'standard'",
    
    // Pipeline обработка
    r#"$nodes.fetch_users.result
       | filter(user => user.active == true)
       | map(user => user.email)
       | take(10)"#,
    
    // String template
    "${workflow.variables.base_url}/users/${nodes.create_user.result.id}",
];

// Использование — результат всегда serde_json::Value
let result: serde_json::Value = context.evaluate_expression(expression).await?;
```

---

### nebula-memory
**Назначение:** Управление памятью и кешированием с учетом scopes.

**Ключевые компоненты:**
- Scoped arenas (Global/Workflow/Execution/Action)
- Expression result caching
- Automatic cleanup
- Tiered cache

```rust
pub struct MemoryManager {
    global_arena: Arc<GlobalArena>,
    execution_arenas: Arc<DashMap<ExecutionId, ExecutionArena>>,
    workflow_arenas: Arc<DashMap<WorkflowId, WorkflowArena>>,
    cache: Arc<TieredMemoryCache>,
}

// Многоуровневый кеш
pub struct TieredMemoryCache {
    l1_hot: LruCache<CacheKey, Arc<CacheEntry>>,     // В памяти
    l2_warm: RwLock<BTreeMap<CacheKey, CacheEntry>>, // Теплый кеш
    l3_external: Option<Box<dyn ExternalCache>>,     // Redis
    expression_cache: ExpressionResultCache,          // Для expressions
}

// Использование — данные передаются как serde_json::Value
let data = context.allocate_scoped_memory(
    large_dataset,  // serde_json::Value
    ResourceLifecycle::Execution  // Очистится в конце execution
).await?;
```

---

### nebula-eventbus
**Назначение:** Pub/sub система для асинхронной коммуникации между компонентами.

**Ключевые компоненты:**
- Scoped subscriptions
- Event filtering
- Distributed events
- Automatic cleanup

```rust
// События workflow lifecycle
pub enum WorkflowEvent {
    WorkflowDeployed { workflow_id, version, deployed_by },
    WorkflowUpdated { workflow_id, old_version, new_version },
}

pub enum ExecutionEvent {
    ExecutionStarted { execution_id, workflow_id, input_data },
    ExecutionCompleted { execution_id, result, duration },
    ExecutionFailed { execution_id, error, retry_count },
}

// Подписка с scope
let subscription = event_bus.subscribe_scoped(
    |event: &ExecutionEvent| async move {
        println!("Execution event: {:?}", event);
    },
    SubscriptionScope::Workflow(workflow_id),
    Some(EventFilter::EventType("execution")),
);

// Публикация из контекста
context.emit_event(NodeEvent::NodeStarted {
    execution_id: context.execution_id.clone(),
    node_id: current_node,
    start_time: SystemTime::now(),
}).await?;
```

---

### nebula-idempotency
**Назначение:** Обеспечение идемпотентности операций для надежности.

**Ключевые компоненты:**
- Idempotency keys
- Result caching
- Deduplication
- Retry detection

```rust
pub struct IdempotencyManager {
    store: Arc<dyn IdempotencyStore>,
    ttl: Duration,
}

impl IdempotencyManager {
    pub async fn execute_once<F, T>(&self, key: &str, f: F) -> Result<T>
    where F: FnOnce() -> Future<Output = Result<T>> {
        // Проверяем, выполнялось ли уже
        if let Some(result) = self.store.get(key).await? {
            return Ok(result);
        }
        
        // Выполняем и сохраняем результат
        let result = f().await?;
        self.store.set(key, &result, self.ttl).await?;
        Ok(result)
    }
}

// Использование в Action
let result = context.idempotency_manager
    .execute_once(&request_id, || async {
        // Операция выполнится только один раз
        database.insert_user(user_data).await
    })
    .await?;
```