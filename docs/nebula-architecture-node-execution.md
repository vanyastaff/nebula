# Nebula Architecture - Node & Execution Layers

## Node Layer

### nebula-node
**Назначение:** Группировка связанных Actions и Credentials в логические узлы для удобной организации и discovery.

**Ключевые концепции:**
- Node как пакет связанной функциональности
- Версионирование на уровне Node
- Метаданные для UI discovery

```rust
pub struct Node {
    pub id: NodeId,
    pub name: String,
    pub version: semver::Version,
    pub actions: Vec<ActionDefinition>,
    pub credentials: Vec<CredentialDefinition>,
    pub metadata: NodeMetadata,
}

// Пример: Node для работы со Slack
let slack_node = Node {
    id: NodeId::new("slack"),
    name: "Slack Integration",
    version: Version::new(2, 1, 0),
    actions: vec![
        ActionDefinition::new("slack.send_message"),
        ActionDefinition::new("slack.create_channel"),
        ActionDefinition::new("slack.upload_file"),
    ],
    credentials: vec![
        CredentialDefinition::new("slack_token", CredentialType::Bearer),
        CredentialDefinition::new("slack_webhook", CredentialType::Webhook),
    ],
};
```

---

### nebula-action
**Назначение:** Система Actions - атомарных единиц работы с гибким подходом к разработке.

**Подходы к разработке:**
1. **Simple approach** - для быстрых решений
2. **Derive macros** - для полноценной интеграции
3. **Trait approach** - для максимального контроля

```rust
// Подход 1: Простой код
pub struct SimpleEmailAction;

impl SimpleAction for SimpleEmailAction {
    type Input = EmailInput;
    type Output = EmailOutput;
    
    async fn execute_simple(&self, input: Self::Input, ctx: &ActionContext) -> Result<Self::Output> {
        let smtp = ctx.get_credential("smtp").await?;
        let client = EmailClient::new(&smtp);
        let message_id = client.send(&input.to, &input.subject, &input.body).await?;
        Ok(EmailOutput { message_id })
    }
}

// Подход 2: Derive макросы
#[derive(Action)]
#[action(
    id = "database.user_lookup",
    name = "User Database Lookup",
    description = "Look up user with caching"
)]
#[resources([DatabaseResource, CacheResource])]
#[credentials(["database"])]
pub struct UserLookupAction;

#[derive(Parameters)]
pub struct UserLookupInput {
    #[parameter(description = "User ID to lookup")]
    pub user_id: String,
    
    #[parameter(description = "Use cache", default = true)]
    pub use_cache: bool,
}

impl ProcessAction for UserLookupAction {
    type Input = UserLookupInput;
    type Output = User;
    
    async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
        let db = ctx.get_resource::<DatabaseResource>().await?;
        let cache = ctx.get_resource::<CacheResource>().await?;
        
        // Проверяем кеш
        if input.use_cache {
            if let Some(user) = cache.get(&input.user_id).await? {
                return Ok(ActionResult::Success(user));
            }
        }
        
        // Загружаем из БД
        let user = db.query_one("SELECT * FROM users WHERE id = $1", &[&input.user_id]).await?;
        cache.set(&input.user_id, &user).await?;
        
        Ok(ActionResult::Success(user))
    }
}
```

**Типы Actions:**
- `SimpleAction` - простые операции
- `ProcessAction` - обработка данных
- `StatefulAction` - с состоянием между вызовами
- `TriggerAction` - источники событий
- `SupplyAction` - поставщики ресурсов

---

### nebula-parameter
**Назначение:** Типобезопасная система параметров с валидацией и expression support.

**Ключевые возможности:**
- Декларативное определение параметров
- Автоматическая валидация
- Expression resolution
- UI metadata generation

```rust
// Программный подход
let parameters = ParameterCollection::new()
    .add_required("email", ParameterType::String {
        pattern: Some(r"^[^@]+@[^@]+$"),
    })
    .add_optional("age", ParameterType::Integer {
        min: Some(18),
        max: Some(150),
    })
    .add_conditional_parameter(ConditionalParameter {
        parameter_name: "send_notification",
        condition: "age >= 18",
        show_when: true,
    });

// Derive подход
#[derive(Parameters)]
pub struct UserRegistrationParams {
    #[parameter(description = "Email address", validation = "email")]
    pub email: String,
    
    #[parameter(description = "User age", min = 18, max = 150)]
    pub age: u8,
    
    #[parameter(
        description = "Send notification",
        show_when = "age >= 18"  // Условный параметр
    )]
    pub send_notification: bool,
    
    #[parameter(
        description = "Scheduled time",
        expression_type = "DateTime"  // Поддержка expressions
    )]
    pub scheduled_at: Option<String>,  // "$nodes.scheduler.result.time"
}

// Expression parameters
let params = hashmap! {
    "to" => ParameterValue::Expression("$nodes.user_lookup.result.email"),
    "subject" => ParameterValue::Template(
        "Order #{order_id} for {name}",
        vec!["$nodes.order.result.id", "$user.name"]
    ),
};
```

---

### nebula-credential
**Назначение:** Безопасное управление учетными данными с автоматической ротацией и шифрованием.

**Ключевые возможности:**
- Различные типы аутентификации
- Автоматическое обновление токенов
- Шифрование в памяти
- Audit trail

```rust
// Типы credentials
pub enum AuthData {
    ApiKey { key: SecretString },
    Bearer { token: SecretString },
    Basic { username: String, password: SecretString },
    OAuth2 { 
        access_token: SecretString,
        refresh_token: Option<SecretString>,
        expires_at: Option<SystemTime>,
    },
    Certificate { cert: Vec<u8>, private_key: SecretBytes },
}

// OAuth2 с автоматическим refresh
pub struct OAuth2Credential {
    access_token: SecretString,
    refresh_token: Option<SecretString>,
    expires_at: Option<SystemTime>,
    auto_refresh: bool,
}

impl Credential for OAuth2Credential {
    async fn get_auth_data(&self, context: &CredentialContext) -> Result<AuthData> {
        // Автоматически обновляем если истек
        if let Some(expires) = self.expires_at {
            if SystemTime::now() > expires && self.auto_refresh {
                self.refresh().await?;
            }
        }
        Ok(self.build_auth_data())
    }
}

// Использование в Action
let slack_client = context
    .get_authenticated_client::<SlackClient>("slack_token")
    .await?;
```

---

## Execution Layer

### nebula-engine
**Назначение:** Главный orchestrator выполнения workflows.

**Ключевые компоненты:**
- WorkflowEngine - основной движок
- Scheduler - планирование выполнения
- Executor - выполнение узлов
- State management

```rust
pub struct WorkflowEngine {
    scheduler: Arc<Scheduler>,
    executor: Arc<Executor>,
    state_store: Arc<StateStore>,
    resource_manager: Arc<ResourceManager>,
    event_bus: Arc<EventBus>,
}

impl WorkflowEngine {
    pub async fn execute_workflow(
        &self,
        workflow_id: WorkflowId,
        input: Value,
        options: ExecutionOptions,
    ) -> Result<ExecutionHandle> {
        // Создаем execution
        let execution_id = ExecutionId::new();
        let workflow = self.load_workflow(&workflow_id).await?;
        
        // Инициализируем контекст
        let context = ExecutionContext::new(
            execution_id.clone(),
            workflow_id.clone(),
            workflow,
        );
        
        // Планируем выполнение
        let handle = self.scheduler.schedule(
            context,
            input,
            options.priority,
        ).await?;
        
        // Отправляем событие
        self.event_bus.emit(ExecutionEvent::Started {
            execution_id,
            workflow_id,
        }).await?;
        
        Ok(handle)
    }
}
```

---

### nebula-runtime
**Назначение:** Runtime окружение для выполнения Actions и управления ресурсами.

**Ключевые компоненты:**
- ActionRuntime - выполнение actions
- ResourceRuntime - управление ресурсами
- Memory management
- Error handling

```rust
pub struct Runtime {
    action_runtime: ActionRuntime,
    resource_runtime: ResourceRuntime,
    memory_manager: Arc<MemoryManager>,
    metrics: Arc<RuntimeMetrics>,
}

pub struct ActionRuntime {
    action_registry: Arc<ActionRegistry>,
    executor: Arc<ActionExecutor>,
    sandbox: SecuritySandbox,
}

impl ActionRuntime {
    pub async fn execute_action(
        &self,
        action_id: &ActionId,
        context: ActionContext,
    ) -> Result<ActionResult> {
        // Получаем action
        let action = self.action_registry.get(action_id)?;
        
        // Проверяем безопасность
        self.sandbox.check_action(&action, &context)?;
        
        // Выполняем с метриками
        let start = Instant::now();
        let result = action.execute(context).await;
        
        self.metrics.record_execution(
            action_id,
            start.elapsed(),
            result.is_ok(),
        );
        
        result
    }
}
```

---

### nebula-worker
**Назначение:** Worker процессы для распределенного выполнения.

**Ключевые возможности:**
- Worker pools
- Task distribution
- Load balancing
- Health monitoring

```rust
pub struct Worker {
    id: WorkerId,
    capacity: WorkerCapacity,
    current_load: Arc<AtomicU32>,
    task_queue: Arc<TaskQueue>,
    runtime: Arc<Runtime>,
}

pub struct WorkerPool {
    workers: Vec<Worker>,
    scheduler: Arc<TaskScheduler>,
    balancer: Arc<LoadBalancer>,
}

impl WorkerPool {
    pub async fn submit_task(&self, task: Task) -> TaskHandle {
        // Выбираем worker по стратегии
        let worker = self.balancer.select_worker(&self.workers).await;
        
        // Добавляем в очередь worker'а
        worker.task_queue.push(task).await;
        
        TaskHandle::new(task.id, worker.id)
    }
    
    pub async fn scale(&self, delta: i32) {
        if delta > 0 {
            // Добавляем workers
            for _ in 0..delta {
                self.add_worker().await;
            }
        } else {
            // Удаляем workers с graceful shutdown
            for _ in 0..delta.abs() {
                self.remove_worker_gracefully().await;
            }
        }
    }
}

// Worker выполнение
impl Worker {
    async fn run(self) {
        loop {
            // Получаем задачу
            let task = self.task_queue.pop().await;
            
            // Проверяем capacity
            if self.current_load.load(Ordering::Relaxed) >= self.capacity.max_concurrent {
                self.task_queue.push_back(task).await;
                sleep(Duration::from_millis(100)).await;
                continue;
            }
            
            // Выполняем
            self.current_load.fetch_add(1, Ordering::Relaxed);
            
            tokio::spawn(async move {
                let result = self.runtime.execute_task(task).await;
                self.report_result(result).await;
                self.current_load.fetch_sub(1, Ordering::Relaxed);
            });
        }
    }
}
```

## Примеры интеграции слоев

### Полный flow выполнения

```rust
// 1. Определяем workflow
let workflow = WorkflowBuilder::new("order-processing")
    .add_node("validate", "validation.order")
    .add_node("payment", "payment.process")
    .add_node("notification", "notification.send")
    .connect("validate", "payment", "$nodes.validate.success")
    .connect("payment", "notification", "$nodes.payment.success")
    .build();

// 2. Регистрируем в engine
engine.register_workflow(workflow).await?;

// 3. Запускаем выполнение
let handle = engine.execute_workflow(
    "order-processing",
    json!({ "order_id": 12345, "amount": 99.99 }),
    ExecutionOptions::default(),
).await?;

// 4. Engine создает ExecutionContext
// 5. Scheduler планирует выполнение узлов
// 6. Worker'ы выполняют Actions
// 7. Results сохраняются в context
// 8. Events публикуются в eventbus

// Мониторинг выполнения
handle.on_node_complete(|node_id, result| {
    println!("Node {} completed: {:?}", node_id, result);
});

let final_result = handle.await?;
```