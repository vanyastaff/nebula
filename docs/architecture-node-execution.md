# Nebula Architecture — Node Execution

## Action Result & Error Model

**Принцип:** `Ok(ActionResult)` = action выполнился, вот flow control инструкция.
`Err(ActionError)` = action НЕ выполнился, вот почему.

Retry — всегда реакция на ошибку → `Err(ActionError::Retryable)`.
Action не решает retry policy — это ответственность engine/config/budget.

### ActionResult

```rust
/// Flow control инструкция от Action к Engine.
/// Action говорит ЧТО получилось, Engine решает КАК продолжить.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ActionResult<T> {
    // ════════════════════════════════════════
    // MVP — Core Flow Control
    // ════════════════════════════════════════

    /// Успешное завершение. Engine → следующие nodes по dependency graph.
    Success { output: T },

    /// Пропустить этот node. Engine → skip, идём к следующим.
    /// Use case: условие не выполнено, данных нет, фильтр отсёк.
    Skip {
        reason: String,
        output: Option<T>,
    },

    /// StatefulAction: ещё не закончил, нужен ещё вызов.
    /// Engine → сохраняет state, re-enqueue в TaskQueue (с опциональной задержкой).
    Continue {
        output: T,
        progress: Option<f64>,       // 0.0..1.0
        delay: Option<Duration>,     // rate limiting, backoff
    },

    /// StatefulAction: итерация завершена.
    /// Engine → финализирует state, идёт к следующим nodes.
    Break {
        output: T,
        reason: BreakReason,
    },

    /// Выбрать ветку. Покрывает if/else (BranchKey="true"/"false")
    /// и switch (BranchKey = произвольный ключ).
    /// Engine → активирует connections с matching branch key.
    Branch {
        selected: BranchKey,
        output: T,
        alternatives: HashMap<BranchKey, T>,
        metadata: Option<serde_json::Value>,
    },

    /// Направить данные в конкретный output port.
    /// Engine → route по port key.
    Route {
        port: PortKey,
        data: T,
    },

    /// Данные в несколько output портов одновременно.
    /// Engine → fan-out по всем портам.
    MultiOutput {
        outputs: HashMap<PortKey, T>,
        main_output: Option<T>,
    },

    /// Ожидание внешнего события, таймера, или человека.
    /// Engine → pause execution, сохранить state, ждать callback.
    Wait {
        condition: WaitCondition,
        timeout: Option<Duration>,
        partial_output: Option<T>,
    },

    // ════════════════════════════════════════
    // Phase 2
    // ════════════════════════════════════════

    /// Async: запустил долгую внешнюю операцию, нужно poll.
    /// Engine → сохранить operation_id, poll с интервалом.
    AsyncOperation {
        operation_id: String,
        estimated_duration: Duration,
        poll_interval: Duration,
        initial_status: T,
    },

    /// Streaming: один item из потока.
    /// Engine → передать downstream, ждать следующий item.
    StreamItem {
        output: T,
        stream_metadata: StreamMetadata,
        side_outputs: Option<HashMap<String, T>>,
    },

    /// Human-in-the-loop: нужен ввод/approval от человека.
    /// Engine → pause, отправить notification, ждать response.
    InteractionRequired {
        interaction_request: InteractionRequest,
        state_output: T,
        response_timeout: Duration,
    },

    /// Saga: подготовлена транзакция, ждём commit/rollback от coordinator.
    /// Engine → Saga coordinator решает commit/compensate.
    TransactionPrepared {
        transaction_id: String,
        rollback_data: T,
        vote: TransactionVote,
        expires_at: DateTime<Utc>,
    },
}

pub type BranchKey = String;
pub type PortKey = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BreakReason {
    Completed,
    MaxIterations,
    ConditionMet,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WaitCondition {
    /// Ждать внешний HTTP callback
    Webhook { callback_id: String },
    /// Ждать до указанного времени
    Until { datetime: DateTime<Utc> },
    /// Ждать указанную длительность
    Duration { duration: Duration },
    /// Ждать approval от пользователя
    Approval { approver: String, message: String },
    /// Ждать completion другого execution
    Execution { execution_id: ExecutionId },
}
```

### ActionError

```rust
/// Ошибка выполнения Action. Retry — всегда здесь, не в ActionResult.
/// Engine смотрит на retry policy (config, metadata, budget) и решает:
/// ретраить или нет, с каким backoff, сколько раз.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionError {
    /// Можно ретраить. Action говорит "не получилось, но можно попробовать".
    /// Engine решает retry policy: backoff, max attempts, budget.
    Retryable {
        error: String,
        /// Подсказка от action — engine может проигнорировать
        backoff_hint: Option<Duration>,
        /// Частичный результат (3 из 5 записей обработаны)
        partial_output: Option<serde_json::Value>,
    },

    /// Нельзя ретраить. Fail fast.
    /// Невалидные credentials, schema mismatch, business logic rejection.
    Fatal {
        error: String,
        details: Option<serde_json::Value>,
    },

    /// Ошибка валидации (до выполнения).
    /// Невалидные параметры, schema не совпадает.
    Validation(String),

    /// Sandbox violation — action запросил capability которой у него нет.
    SandboxViolation {
        capability: String,
        action_id: String,
    },

    /// Execution отменена (CancellationToken сработал).
    Cancelled,

    /// Превышен лимит данных (output слишком большой).
    DataLimitExceeded {
        limit_bytes: u64,
        actual_bytes: u64,
    },
}

impl ActionError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable { .. })
    }

    pub fn retryable(msg: impl Into<String>) -> Self {
        Self::Retryable { error: msg.into(), backoff_hint: None, partial_output: None }
    }

    pub fn fatal(msg: impl Into<String>) -> Self {
        Self::Fatal { error: msg.into(), details: None }
    }
}
```

### Как Engine обрабатывает ActionResult

```rust
// engine/src/result_handler.rs
impl ResultHandler {
    pub async fn handle(
        &self,
        node_id: &NodeId,
        result: ActionResult<serde_json::Value>,
        execution: &mut ExecutionState,
    ) -> Result<Vec<NodeId>> {  // возвращает список next nodes для запуска
        match result {
            ActionResult::Success { output } => {
                execution.set_node_output(node_id, output);
                Ok(execution.get_dependents(node_id))
            }
            ActionResult::Skip { reason, output } => {
                execution.set_node_skipped(node_id, &reason, output);
                Ok(execution.get_dependents(node_id))
            }
            ActionResult::Continue { output, progress, delay } => {
                execution.update_node_state(node_id, output);
                // Re-enqueue в TaskQueue с delay
                self.task_queue.enqueue(Task {
                    node_id: node_id.clone(),
                    delay,
                    ..existing_task
                }).await?;
                Ok(vec![]) // Не запускаем dependents — ещё не закончили
            }
            ActionResult::Break { output, reason } => {
                execution.set_node_output(node_id, output);
                Ok(execution.get_dependents(node_id))
            }
            ActionResult::Branch { selected, output, alternatives, .. } => {
                execution.set_node_output(node_id, output);
                // Активируем только connections с matching branch key
                Ok(execution.get_branch_dependents(node_id, &selected))
            }
            ActionResult::Route { port, data } => {
                execution.set_port_output(node_id, &port, data);
                Ok(execution.get_port_dependents(node_id, &port))
            }
            ActionResult::MultiOutput { outputs, main_output } => {
                for (port, data) in outputs {
                    execution.set_port_output(node_id, &port, data);
                }
                Ok(execution.get_all_port_dependents(node_id))
            }
            ActionResult::Wait { condition, timeout, partial_output } => {
                execution.set_node_waiting(node_id, condition, timeout);
                Ok(vec![]) // Не запускаем dependents — ждём
            }
            // Phase 2 variants...
            _ => todo!("Phase 2"),
        }
    }
}
```

---

## Action Layer

### action
**Назначение:** Определение и выполнение атомарных действий в workflow.

**Два режима исполнения:**
- **Typed mode** — first-party: `Input: Deserialize + Validate`, `Output: Serialize`. Compile-time.
- **Dynamic mode** — community/hub/WASM: `serde_json::Value` + JSON Schema validation в runtime.

**MVP Action Types (3 штуки):**
- `ProcessAction` — stateless, one-shot (covers 80% use cases)
- `StatefulAction` — iterative, paginated, checkpoint
- `TriggerAction` — event sources (webhook, cron, poll)

Phase 2: InteractiveAction (human-in-the-loop), TransactionalAction (Saga).

### ActionContext

```rust
/// Контекст исполнения, доступный каждому Action.
/// При CapabilityGated/Isolated — обёрнут в SandboxedContext (capability checks).
pub struct ActionContext {
    pub execution_id: ExecutionId,
    pub node_id: NodeId,
    pub workflow_id: WorkflowId,
    /// CancellationToken — action ОБЯЗАН проверять в длинных операциях.
    pub cancellation: CancellationToken,
    /// Доступ к credentials (через ports::SecretsStore)
    credentials: Arc<dyn SecretsStore>,
    /// Доступ к resources (через resource crate)
    resources: Arc<ResourceManager>,
    /// Telemetry
    telemetry: Arc<Telemetry>,
}

impl ActionContext {
    /// Получить credential. В SandboxedContext — с capability check.
    pub async fn get_credential(&self, id: &str) -> Result<AuthData, ActionError> {
        self.credentials.get_secret(id).await
            .map_err(|e| ActionError::fatal(format!("Credential '{}': {}", id, e)))?
            .ok_or_else(|| ActionError::fatal(format!("Credential '{}' not found", id)))
    }

    /// Получить resource. В SandboxedContext — с capability check.
    pub async fn get_resource<R: Resource>(&self) -> Result<R::Instance, ActionError> {
        self.resources.get::<R>().await
            .map_err(|e| ActionError::fatal(e.to_string()))
    }

    /// Проверить cancellation. Вызывать в циклах/batch операциях.
    pub fn check_cancelled(&self) -> Result<(), ActionError> {
        if self.cancellation.is_cancelled() {
            Err(ActionError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn emit_metric(&self, name: &str, value: f64) {
        self.telemetry.emit_metric(name, value);
    }
}
```

### ProcessAction (stateless)

```rust
#[async_trait]
pub trait ProcessAction: Action {
    type Input: Send + Sync + 'static;
    type Output: Send + Sync + 'static;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError>;

    /// Optional: валидация до выполнения
    async fn validate_input(&self, input: &Self::Input) -> Result<(), ActionError> {
        Ok(())
    }
}

// === Пример: typed mode (first-party) ===

#[derive(Action)]
#[action(id = "database.user_lookup", mode = "typed")]
#[sandbox(isolation = "capability_gated", capabilities = [
    Resource("database"), Resource("cache"), Credential("database"),
])]
pub struct UserLookupAction;

impl ProcessAction for UserLookupAction {
    type Input = UserLookupInput;
    type Output = User;

    async fn execute(&self, input: Self::Input, ctx: &ActionContext)
        -> Result<ActionResult<Self::Output>, ActionError>
    {
        let db = ctx.get_resource::<DatabaseResource>().await?;
        let cache = ctx.get_resource::<CacheResource>().await?;

        if input.use_cache {
            if let Some(user) = cache.get(&input.user_id).await
                .map_err(|e| ActionError::retryable(e.to_string()))?
            {
                return Ok(ActionResult::Success { output: user });
            }
        }

        let user = db.query_one("SELECT * FROM users WHERE id = $1", &[&input.user_id])
            .await
            .map_err(|e| match e {
                DbError::ConnectionLost => ActionError::retryable("DB connection lost"),
                DbError::NotFound => ActionError::fatal("User not found"),
                other => ActionError::fatal(other.to_string()),
            })?;

        cache.set(&input.user_id, &user).await.ok(); // best-effort cache
        Ok(ActionResult::Success { output: user })
    }
}

// === Пример: Branch (if/switch) ===

impl ProcessAction for OrderRouterAction {
    type Input = Order;
    type Output = Order;

    async fn execute(&self, input: Self::Input, ctx: &ActionContext)
        -> Result<ActionResult<Self::Output>, ActionError>
    {
        let branch = match input.total {
            x if x > 10_000.0 => "high_value",
            x if x > 1_000.0  => "medium_value",
            _                  => "standard",
        };

        Ok(ActionResult::Branch {
            selected: branch.into(),
            output: input.clone(),
            alternatives: HashMap::new(),
            metadata: Some(json!({ "total": input.total, "threshold": [10000, 1000] })),
        })
    }
}
```

### Dynamic mode (community/hub/WASM)

```rust
/// Community/hub actions. Input/output = serde_json::Value.
/// Runtime автоматически:
/// 1. Валидирует input по metadata.input_schema (JSON Schema)
/// 2. Оборачивает в SandboxedContext (isolation = Isolated, WASM)
/// 3. Валидирует output по metadata.output_schema
/// 4. Enforce-ит data limits (max_node_output_bytes)
#[async_trait]
pub trait DynamicActionHandler: Send + Sync {
    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &SandboxedContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError>;
}
```

### StatefulAction (iterative, paginated)

```rust
#[async_trait]
pub trait StatefulAction: Action {
    type State: Serialize + DeserializeOwned + Send + Sync + 'static;
    type Input: Send + Sync + 'static;
    type Output: Send + Sync + 'static;

    async fn execute_with_state(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError>;

    async fn initialize_state(
        &self,
        input: &Self::Input,
        ctx: &ActionContext,
    ) -> Result<Self::State, ActionError>;

    fn state_version(&self) -> u32 { 1 }

    async fn migrate_state(
        &self,
        old_state: serde_json::Value,
        old_version: u32,
    ) -> Result<Self::State, ActionError> {
        serde_json::from_value(old_state)
            .map_err(|e| ActionError::fatal(format!("State migration failed: {}", e)))
    }
}

// === Пример: paginated API с cancellation ===

impl StatefulAction for PaginatedFetchAction {
    type State = FetchState;
    type Input = FetchInput;
    type Output = FetchOutput;

    async fn execute_with_state(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Проверяем cancellation ПЕРЕД каждой итерацией
        ctx.check_cancelled()?;

        let page = self.fetch_page(&input.url, state.cursor.as_deref())
            .await
            .map_err(|e| ActionError::retryable(e.to_string()))?;

        state.total_items += page.items.len();
        state.cursor = page.next_cursor;

        if state.cursor.is_none() || state.total_items >= input.max_items {
            Ok(ActionResult::Break {
                output: FetchOutput { total: state.total_items },
                reason: BreakReason::Completed,
            })
        } else {
            Ok(ActionResult::Continue {
                output: FetchOutput { total: state.total_items },
                progress: input.max_items.map(|max|
                    state.total_items as f64 / max as f64
                ),
                delay: Some(Duration::from_millis(500)), // rate limit
            })
        }
    }

    async fn initialize_state(&self, _input: &Self::Input, _ctx: &ActionContext)
        -> Result<Self::State, ActionError>
    {
        Ok(FetchState { cursor: None, total_items: 0 })
    }
}
```

### TriggerAction (event sources)

```rust
/// TriggerAction — источник событий для запуска workflows.
/// Engine управляет lifecycle (регистрация webhook, cron, polling loop).
/// Action только предоставляет handler.
pub enum TriggerKind {
    /// Внешний HTTP вызов → запуск workflow
    Webhook {
        path: String,
        method: HttpMethod,
        auth: WebhookAuth,
    },
    /// Периодический запуск
    Schedule {
        cron: String,
        timezone: String,
    },
    /// Поллинг внешнего источника
    Poll {
        interval: Duration,
        /// Ключ дедупликации — предотвращает повторный запуск по тем же данным
        dedup_key: Option<String>,
    },
    /// Подписка на внешний event stream
    Subscribe {
        config: serde_json::Value,
    },
}

#[async_trait]
pub trait TriggerAction: Action {
    type Config: Send + Sync + 'static;
    type Event: Send + Sync + 'static;

    /// Какой тип триггера
    fn kind(&self, config: &Self::Config) -> TriggerKind;

    /// Для Poll: проверить и вернуть новые данные
    async fn poll(
        &self,
        config: &Self::Config,
        last_state: Option<serde_json::Value>,
        ctx: &ActionContext,
    ) -> Result<Vec<TriggerEvent<Self::Event>>, ActionError> {
        Ok(vec![])
    }

    /// Для Webhook: обработать входящий запрос
    async fn handle_webhook(
        &self,
        config: &Self::Config,
        request: WebhookRequest,
        ctx: &ActionContext,
    ) -> Result<TriggerEvent<Self::Event>, ActionError> {
        Err(ActionError::fatal("Webhook not supported"))
    }
}

pub struct TriggerEvent<T> {
    pub data: T,
    pub dedup_key: Option<String>,
    pub timestamp: DateTime<Utc>,
}

// Engine управляет lifecycle:
// - Webhook → регистрирует HTTP route в API server
// - Schedule → cron scheduler (tokio-cron или аналог)
// - Poll → polling loop с дедупликацией по dedup_key
// - Subscribe → connection management + reconnect
```

---

## Data Passing Policy

**Проблема:** `NodeOutput.result: serde_json::Value` может быть 500MB JSON.
Одна нода возвращает гигантский массив → OOM.

```rust
pub struct DataPassingPolicy {
    /// Максимальный размер output одной ноды (default: 10MB)
    pub max_node_output_bytes: u64,
    /// Максимальный суммарный размер данных в execution (default: 100MB)
    pub max_total_execution_bytes: u64,
    /// Что делать при превышении
    pub large_data_strategy: LargeDataStrategy,
}

pub enum LargeDataStrategy {
    /// Ошибка (ActionError::DataLimitExceeded)
    Reject,
    /// Spill в blob storage, передать reference
    SpillToBlob,
}

/// Что передаётся между нодами
pub enum NodeOutputData {
    /// Маленькие данные — inline JSON
    Inline(serde_json::Value),
    /// Большие данные → через BlobStore, передаётся reference
    BlobRef {
        key: String,
        size: u64,
        mime: String,
    },
}

// Runtime проверяет ПОСЛЕ выполнения Action:
// 1. Сериализует output
// 2. Проверяет размер vs max_node_output_bytes
// 3. Если превышен + strategy=SpillToBlob → пишет в BlobStore, передаёт BlobRef
// 4. Если превышен + strategy=Reject → ActionError::DataLimitExceeded
```

---

## Execution Layer

### engine
**Назначение:** Главный orchestrator. Строит ExecutionPlan ДО запуска.

```rust
pub struct WorkflowEngine {
    planner: Arc<ExecutionPlanner>,
    scheduler: Arc<Scheduler>,
    result_handler: Arc<ResultHandler>,
    execution_repo: Arc<dyn ExecutionRepo>,
    workflow_repo: Arc<dyn WorkflowRepo>,
    task_queue: Arc<dyn TaskQueue>,
    telemetry: Arc<Telemetry>,
    /// Active cancellation tokens per execution
    active_tokens: Arc<DashMap<ExecutionId, CancellationToken>>,
}

pub struct ExecutionPlan {
    pub execution_id: ExecutionId,
    pub dependency_graph: DependencyGraph,
    pub parallel_groups: Vec<Vec<NodeId>>,
    pub required_capabilities: Vec<Capability>,
    pub required_resources: Vec<ResourceId>,
    pub budget: ExecutionBudget,
}

pub struct ExecutionBudget {
    pub max_concurrent_nodes: usize,
    pub max_total_retries: u32,
    pub max_wall_time: Duration,
    pub max_payload_bytes: u64,
    pub tenant_quota: Option<TenantQuota>,
    pub data_policy: DataPassingPolicy,
}

impl WorkflowEngine {
    pub async fn execute_workflow(
        &self,
        workflow_id: WorkflowId,
        input: serde_json::Value,
        options: ExecutionOptions,
    ) -> Result<ExecutionHandle> {
        let execution_id = ExecutionId::new();
        let workflow = self.workflow_repo.get(&workflow_id).await?
            .ok_or(EngineError::WorkflowNotFound)?;

        // 1. Строим план ДО запуска
        let plan = self.planner.build_plan(&workflow, &input, &options).await?;

        // 2. Pre-check: capabilities, ресурсы, квоты
        self.planner.validate_plan(&plan).await?;

        // 3. ExecutionRepo: CAS transition + journal (source of truth)
        self.execution_repo.transition(
            &execution_id,
            ExecutionStatus::Created,
            ExecutionStatus::Running,
            JournalEntry::ExecutionStarted { input: input.clone(), plan: plan.clone() },
        ).await?;

        // 4. Создаём CancellationToken
        let cancel_token = CancellationToken::new();
        self.active_tokens.insert(execution_id.clone(), cancel_token.clone());

        // 5. Spawn watchdog (max_wall_time)
        self.spawn_watchdog(execution_id.clone(), plan.budget.max_wall_time);

        // 6. Ставим в очередь
        self.task_queue.enqueue(Task {
            execution_id: execution_id.clone(),
            workflow_id,
            input,
            budget: plan.budget,
            cancellation: cancel_token,
        }).await?;

        // 7. Event как ПРОЕКЦИЯ
        self.telemetry.emit(ExecutionEvent::Started {
            execution_id: execution_id.clone(),
        }).await?;

        Ok(ExecutionHandle::new(execution_id))
    }
}
```

### Cancellation

```rust
/// Engine создаёт CancellationToken для каждого execution.
/// При cancel request — token отменяется, все running actions получают сигнал.
impl WorkflowEngine {
    pub async fn cancel_execution(&self, id: &ExecutionId) -> Result<()> {
        // 1. CAS transition → Cancelling
        self.execution_repo.transition(
            id,
            ExecutionStatus::Running,
            ExecutionStatus::Cancelling,
            JournalEntry::CancellationRequested,
        ).await?;

        // 2. Cancel token → все running actions получат ActionError::Cancelled
        if let Some(token) = self.active_tokens.get(id) {
            token.cancel();
        }

        // 3. Worker при Cancelled → nack task, release lease (graceful drain)
        Ok(())
    }
}
```

### Execution-level timeout watchdog

```rust
impl WorkflowEngine {
    fn spawn_watchdog(&self, execution_id: ExecutionId, max_wall_time: Duration) {
        let repo = self.execution_repo.clone();
        let tokens = self.active_tokens.clone();
        let telemetry = self.telemetry.clone();

        tokio::spawn(async move {
            tokio::time::sleep(max_wall_time).await;

            if let Ok(Some(state)) = repo.get_state(&execution_id).await {
                if state.status == ExecutionStatus::Running {
                    // Принудительно cancel
                    if let Some(token) = tokens.get(&execution_id) {
                        token.cancel();
                    }
                    repo.transition(
                        &execution_id,
                        ExecutionStatus::Running,
                        ExecutionStatus::TimedOut,
                        JournalEntry::ExecutionFailed {
                            error: format!("Wall time exceeded: {:?}", max_wall_time),
                            retry_count: 0,
                        },
                    ).await.ok();
                    telemetry.emit(ExecutionEvent::TimedOut {
                        execution_id: execution_id.clone(),
                    }).await.ok();
                }
            }
        });
    }
}
```

---

### Sandbox (capability-based, через ports::SandboxRunner)

**Один trait `SandboxRunner` в `ports`. Реализации — в drivers.**

```rust
pub enum IsolationLevel {
    /// Доверенный код — builtin/first-party. Только через ctx.* API.
    /// НЕ sandbox. Допустим только для подписанных builtin-действий.
    None,
    /// Capability-gated: вызовы проксируются через capability checks,
    /// но код исполняется IN-PROCESS. НЕ защищает от UB, эксплойтов,
    /// чтения памяти, side channels. Это "policy-gated host API", не sandbox.
    CapabilityGated,
    /// Полная изоляция — WASM/process sandbox.
    /// ОБЯЗАТЕЛЬНО для community/hub/marketplace. Не переопределяется конфигом.
    Isolated,
}

pub enum Capability {
    Network { allowed_hosts: Vec<String> },
    FileSystem { paths: Vec<PathBuf>, read_only: bool },
    Resource(ResourceId),
    Credential(CredentialId),
    MaxMemory(usize),
    MaxCpuTime(Duration),
    Environment { keys: Vec<String> },
}

/// SandboxedContext — обёртка ActionContext с capability checks.
pub struct SandboxedContext {
    inner: ActionContext,
    granted_capabilities: Vec<Capability>,
}

impl SandboxedContext {
    pub async fn get_resource<R: Resource>(&self) -> Result<R::Instance, ActionError> {
        self.check_capability(&Capability::Resource(R::resource_id()))?;
        self.inner.get_resource::<R>().await
    }

    pub async fn get_credential(&self, id: &str) -> Result<AuthData, ActionError> {
        self.check_capability(&Capability::Credential(CredentialId::new(id)))?;
        self.inner.get_credential(id).await
    }

    fn check_capability(&self, cap: &Capability) -> Result<(), ActionError> {
        if self.granted_capabilities.contains(cap) {
            Ok(())
        } else {
            Err(ActionError::SandboxViolation {
                capability: format!("{:?}", cap),
                action_id: self.inner.node_id.to_string(),
            })
        }
    }

    /// Делегирует к inner
    pub fn check_cancelled(&self) -> Result<(), ActionError> {
        self.inner.check_cancelled()
    }
}

// Sandbox policy:
// [sandbox.community_actions]
// patterns = ["community.*", "hub.*", "marketplace.*"]
// isolation = "isolated"   ← HARDCODED, не переопределяется конфигом
```

**Реализации (drivers):**
- `drivers/sandbox-inprocess` — capability checks in-process (None/CapabilityGated)
- `drivers/sandbox-wasm` — wasmtime (Isolated — полная изоляция)

---

### runtime
**Назначение:** Выполнение Actions через SandboxRunner (из `ports`).

```rust
pub struct ActionRuntime {
    action_registry: Arc<ActionRegistry>,
    sandbox: Arc<dyn SandboxRunner>,
    data_policy: DataPassingPolicy,
}

impl ActionRuntime {
    pub async fn execute_action(
        &self,
        action_id: &ActionId,
        context: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        let action = self.action_registry.get(action_id)
            .map_err(|e| ActionError::fatal(e.to_string()))?;
        let metadata = action.metadata();
        let isolation = self.resolve_isolation_level(metadata);

        // 1. Execute through sandbox
        let result = match isolation {
            IsolationLevel::None => {
                action.execute_dynamic(context).await?
            }
            IsolationLevel::CapabilityGated | IsolationLevel::Isolated => {
                let sandboxed = SandboxedContext::new(context, metadata.capabilities.clone());
                self.sandbox.execute(action.as_ref(), sandboxed, metadata).await?
            }
        };

        // 2. Enforce data limits on output
        self.enforce_data_limits(&result)?;

        Ok(result)
    }

    fn enforce_data_limits(
        &self,
        result: &ActionResult<serde_json::Value>,
    ) -> Result<(), ActionError> {
        let output = match result {
            ActionResult::Success { output } => output,
            ActionResult::Break { output, .. } => output,
            ActionResult::Branch { output, .. } => output,
            _ => return Ok(()),
        };

        let size = serde_json::to_vec(output)
            .map(|v| v.len() as u64)
            .unwrap_or(0);

        if size > self.data_policy.max_node_output_bytes {
            match self.data_policy.large_data_strategy {
                LargeDataStrategy::Reject => {
                    Err(ActionError::DataLimitExceeded {
                        limit_bytes: self.data_policy.max_node_output_bytes,
                        actual_bytes: size,
                    })
                }
                LargeDataStrategy::SpillToBlob => {
                    // TODO: spill to BlobStore, replace with BlobRef
                    Ok(())
                }
            }
        } else {
            Ok(())
        }
    }
}
```

---

### Worker (bounded concurrency)

```rust
pub struct Worker {
    id: String,
    accepting: AtomicBool,
    concurrency_semaphore: Arc<Semaphore>,
    task_queue: Arc<dyn TaskQueue>,
    runtime: Arc<ActionRuntime>,
    running_tasks: Arc<TaskTracker>,
}

impl Worker {
    async fn run(&self) {
        loop {
            if !self.accepting.load(Ordering::Relaxed) {
                break;
            }

            let task = match self.task_queue.dequeue(Duration::from_secs(5)).await {
                Ok(Some(task)) => task,
                _ => continue,
            };

            let permit = self.concurrency_semaphore
                .clone().acquire_owned().await
                .expect("semaphore closed");

            let runtime = self.runtime.clone();
            let queue = self.task_queue.clone();

            self.running_tasks.spawn(async move {
                let _permit = permit;
                let result = runtime.execute_action(&task.action_id, task.context).await;
                match result {
                    Ok(_) => queue.ack(&task.id).await.ok(),
                    Err(ref e) if e.is_retryable() => queue.nack(&task.id).await.ok(),
                    Err(_) => queue.ack(&task.id).await.ok(), // Fatal → не ретраим
                };
            });
        }
    }

    /// Graceful shutdown
    pub async fn shutdown(&self, timeout: Duration) {
        // 1. Перестаём dequeue
        self.accepting.store(false, Ordering::SeqCst);
        // 2. Ждём завершения running tasks (до timeout)
        tokio::time::timeout(timeout, self.running_tasks.wait()).await.ok();
        // 3. Незавершённые → nack (вернутся в queue)
        self.nack_remaining().await;
        // 4. Release lease'ы
        self.release_leases().await;
    }
}
```

---

## Полный flow выполнения

```rust
// 1. Определяем workflow с Branch routing
let workflow = WorkflowBuilder::new("order-processing")
    .add_node("validate", "validation.order")
    .add_node("router", "order.router")
    .add_node("payment_high", "payment.enterprise")
    .add_node("payment_std", "payment.standard")
    .add_node("notification", "notification.send")
    .connect("validate", "router", None)
    .connect_branch("router", "payment_high", "high_value")
    .connect_branch("router", "payment_std", "standard")
    .connect("payment_high", "notification", None)
    .connect("payment_std", "notification", None)
    .build();

// 2. Запускаем
let handle = engine.execute_workflow(
    "order-processing",
    json!({ "order_id": 12345, "amount": 99.99 }),
    ExecutionOptions::default(),
).await?;

// Внутри:
// 1.  ExecutionPlanner строит план (граф, groups, budget)
// 2.  Pre-check: capabilities, ресурсы, квоты
// 3.  ExecutionRepo: CAS Created→Running + journal (source of truth)
// 4.  CancellationToken создан, watchdog запущен
// 5.  Task → TaskQueue
// 6.  Worker dequeue → Semaphore → acquire_lease
// 7.  Runtime → SandboxRunner → Action.execute()
// 8.  Action returns Ok(ActionResult::Branch { selected: "standard" })
//     → ResultHandler активирует connection "router" → "payment_std"
// 9.  Или Action returns Err(ActionError::Retryable { .. })
//     → Engine проверяет retry policy/budget → nack → re-enqueue
// 10. Journal append (NodeAttempt с idempotency_key)
// 11. Data limit check (max_node_output_bytes)
// 12. ExecutionRepo: CAS → final status
// 13. Queue ack
// 14. Telemetry emit (проекция)

// Cancellation:
engine.cancel_execution(&handle.execution_id()).await?;
// → CancellationToken → running actions → ActionError::Cancelled
// → Worker nack → graceful drain

let final_result = handle.await?;
```
