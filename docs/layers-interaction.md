# Nebula Architecture — Layers Interaction

## Принципы взаимодействия

1. **Ports & Drivers** — core говорит только с trait-интерфейсами (`ports`), drivers реализуют
2. **ExecutionRepo = Truth** — `ports::ExecutionRepo` (journal + CAS) = единственный источник истины, events = проекции
3. **Dependency Injection** — bins собирают core + drivers через composition root
4. **Typed + Dynamic** — typed Actions (first-party) + dynamic serde_json::Value (community)
5. **Bounded Concurrency** — Semaphore + bounded channels, никаких unbounded spawn
6. **Execution Plan** — план с budget строится ДО запуска
7. **Capability-based Sandbox** — community actions всегда Full isolation

---

## 1. core ↔ ports (фундаментальная граница)

**Правило:** Core крейты НИКОГДА не зависят от drivers. Только от `ports` (traits).

```rust
// core/engine зависит от ports::TaskQueue (trait), не от queue-redis (impl)
pub struct WorkflowEngine {
    execution_repo: Arc<dyn ExecutionRepo>,    // ports trait
    workflow_repo: Arc<dyn WorkflowRepo>,      // ports trait
    task_queue: Arc<dyn TaskQueue>,            // ports trait
    telemetry: Arc<Telemetry>,
}

// runtime зависит от ports::SandboxRunner (trait), не от sandbox-wasm (impl)
pub struct ActionRuntime {
    sandbox: Arc<dyn SandboxRunner>,           // ports trait
    action_registry: Arc<ActionRegistry>,
}

// Конкретные реализации подставляются в bins (composition root)
```

**Почему это важно:**
- Desktop билд не компилирует postgres/redis/s3/wasmtime
- Один и тот же engine работает с SQLite (desktop) и Postgres (cloud)
- Тестирование core — без внешних зависимостей (in-memory mocks)

---

## 2. bins → drivers → ports (composition root)

**Паттерн:** Каждый bin-крейт = composition root. Читает config, создаёт drivers, инжектирует в core.

```rust
// bins/desktop/src/main.rs
#[tokio::main]
async fn main() -> Result<()> {
    let config: NebulaConfig = ConfigManager::load("config.desktop.toml").await?;

    // 1. Создаём drivers (конкретные реализации ports traits)
    let storage = SqliteStorage::new(&config.storage.path).await?;
    let blobs = FsBlobStore::new(&config.blobs.root)?;
    let queue = MemoryQueue::new(config.engine.max_concurrent_executions);
    let sandbox = InProcessSandbox::new();
    let secrets = LocalSecretsStore::new(&config.secrets.path)?;

    // 2. Собираем core (engine/runtime) с drivers через ports traits
    let engine = WorkflowEngine::builder()
        .execution_repo(Arc::new(storage.clone()) as Arc<dyn ExecutionRepo>)
        .workflow_repo(Arc::new(storage.clone()) as Arc<dyn WorkflowRepo>)
        .task_queue(Arc::new(queue) as Arc<dyn TaskQueue>)
        .build()?;

    let runtime = ActionRuntime::builder()
        .sandbox(Arc::new(sandbox) as Arc<dyn SandboxRunner>)
        .build()?;

    // 3. Запускаем worker + API
    let worker = Worker::new(engine.task_queue(), runtime, config.engine.max_concurrent_executions);

    tokio::select! {
        _ = worker.run() => {},
        _ = start_api(&config.api.bind, engine) => {},
    }

    Ok(())
}

// bins/server/src/main.rs — то же самое, но с postgres/redis drivers
// bins/worker/src/main.rs — только runtime + queue (без engine/API)
// bins/control-plane/src/main.rs — только engine + API (без runtime)
```

---

## 3. engine ↔ execution ↔ ports::ExecutionRepo

**Flow:** Engine строит план → записывает в ExecutionRepo → ставит в TaskQueue.

```rust
impl WorkflowEngine {
    pub async fn execute_workflow(&self, workflow_id: WorkflowId, input: serde_json::Value) -> Result<ExecutionHandle> {
        let workflow = self.workflow_repo.get(&workflow_id).await?
            .ok_or(EngineError::WorkflowNotFound)?;

        // 1. Строим ExecutionPlan (dependency graph, parallel groups, budget)
        let plan = self.planner.build_plan(&workflow, &input).await?;

        // 2. ExecutionRepo — source of truth (atomic CAS + journal append)
        let execution_id = ExecutionId::new();
        self.execution_repo.transition(
            &execution_id,
            ExecutionStatus::Created,
            ExecutionStatus::Running,
            JournalEntry::ExecutionStarted {
                input: input.clone(),
                plan: plan.clone(),
            },
        ).await?;

        // 3. Ставим в очередь (через ports::TaskQueue)
        self.task_queue.enqueue(Task {
            execution_id: execution_id.clone(),
            workflow_id,
            input,
            budget: plan.budget,
        }).await?;

        // 4. Telemetry — ПРОЕКЦИЯ, не source of truth
        self.telemetry.emit(ExecutionEvent::Started {
            execution_id: execution_id.clone(),
        }).await?;

        Ok(ExecutionHandle::new(execution_id))
    }
}
```

**Важно:** при восстановлении после падения — состояние из ExecutionRepo, НЕ из событий.

---

## 4. worker ↔ ports::TaskQueue ↔ runtime ↔ ports::SandboxRunner

**Семантика доставки: at-least-once.**
Консистентность достигается через `idempotency_key` в `NodeAttempt`.

**Обязательный порядок операций (worker loop):**
1. `dequeue` — task получен, visibility timeout запущен
2. `acquire_lease` в ExecutionRepo (если distributed)
3. journal append: attempt started
4. execute action (через SandboxRunner)
5. journal append: attempt completed/failed
6. `ack` (успех) или `nack` (retry)

**Failure scenarios:**
- Worker упал между 3 и 6 → visibility timeout истечёт → task вернётся в очередь → 
  `idempotency_key` предотвратит дубли выполнения
- Worker упал до 3 → task вернётся, как будто ничего не было
- Queue потерял ack → повторная доставка → idempotency check

```rust
impl Worker {
    async fn run(&self) {
        // Reaper: периодически claim stale entries у мёртвых consumers
        let reaper_handle = self.spawn_reaper();

        loop {
            // 1. Dequeue с visibility timeout
            let task = match self.task_queue.dequeue(Duration::from_secs(5)).await {
                Ok(Some(task)) => task,
                _ => continue,
            };

            // 2. Bounded concurrency — Semaphore (НИКОГДА unbounded spawn)
            let permit = self.concurrency_semaphore
                .clone().acquire_owned().await
                .expect("semaphore closed");

            let runtime = self.runtime.clone();
            let queue = self.task_queue.clone();
            let execution_repo = self.execution_repo.clone();

            tokio::spawn(async move {
                let _permit = permit; // Drop = release

                // 3. Acquire lease (distributed mode)
                // В single-process (desktop) — no-op
                if !execution_repo.acquire_lease(
                    &task.execution_id, &self.id, Duration::from_secs(60)
                ).await.unwrap_or(false) {
                    queue.nack(&task.id).await.ok();
                    return;
                }

                // 4. Journal: attempt started
                execution_repo.transition(
                    &task.execution_id,
                    ExecutionStatus::Running, ExecutionStatus::Running,
                    JournalEntry::NodeAttempt(NodeAttempt {
                        node_id: task.node_id.clone(),
                        attempt_number: task.attempt,
                        idempotency_key: task.idempotency_key.clone(),
                        resolved_inputs: task.resolved_inputs.clone(),
                        output: None, error: None,
                        started_at: SystemTime::now(),
                        completed_at: None,
                        bytes_in: 0, bytes_out: 0,
                    }),
                ).await.ok();

                // 5. Execute action
                let result = runtime.execute_action(&task.action_id, task.context).await;

                // 6. Journal: attempt completed/failed
                let next_status = if result.is_ok() {
                    ExecutionStatus::Completed
                } else {
                    ExecutionStatus::Failed
                };
                execution_repo.transition(
                    &task.execution_id,
                    ExecutionStatus::Running, next_status,
                    JournalEntry::NodeAttempt(NodeAttempt {
                        node_id: task.node_id.clone(),
                        attempt_number: task.attempt,
                        idempotency_key: task.idempotency_key.clone(),
                        resolved_inputs: task.resolved_inputs.clone(),
                        output: result.as_ref().ok().cloned(),
                        error: result.as_ref().err().map(|e| e.to_string()),
                        started_at: task.started_at,
                        completed_at: Some(SystemTime::now()),
                        bytes_in: 0, bytes_out: 0,
                    }),
                ).await.ok();

                // 7. Ack/nack в queue (ПОСЛЕ journal)
                // Fatal → ack (не ретраим). Retryable → nack (вернётся в queue).
                match &result {
                    Ok(_) => queue.ack(&task.id).await.ok(),
                    Err(e) if e.is_retryable() => queue.nack(&task.id).await.ok(),
                    Err(_) => queue.ack(&task.id).await.ok(), // Fatal — не ретраим
                };
            });
        }
    }

    /// Reaper: периодически claim stale entries у мёртвых consumers
    fn spawn_reaper(&self) -> JoinHandle<()> {
        let queue = self.task_queue.clone();
        let execution_repo = self.execution_repo.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                // Claim tasks stuck с мёртвыми consumers
                if let Ok(stale) = queue.claim_stale(Duration::from_secs(60)).await {
                    for task in stale {
                        // Re-enqueue или handle
                        queue.nack(&task.id).await.ok();
                    }
                }
                // Также: check stale leases в ExecutionRepo
                if let Ok(stale_execs) = execution_repo.find_stale_leases(Duration::from_secs(120)).await {
                    for exec_id in stale_execs {
                        // Mark as failed / re-schedulable
                        tracing::warn!(%exec_id, "Found stale lease, marking for recovery");
                    }
                }
            }
        })
    }
}
```

---

## 5. runtime ↔ sandbox ↔ action (capability enforcement + data limits)

**Flow:** Runtime определяет IsolationLevel → создаёт SandboxedContext → вызывает SandboxRunner → enforce data limits.

**Return type:** `Result<ActionResult<serde_json::Value>, ActionError>`.
- `Ok(ActionResult::*)` = flow control (Success, Branch, Continue, Wait, ...)
- `Err(ActionError::Retryable)` → engine решает retry policy → nack
- `Err(ActionError::Fatal)` → fail fast → ack (не ретраим)
- `Err(ActionError::Cancelled)` → execution cancelled → nack

```rust
impl ActionRuntime {
    pub async fn execute_action(
        &self,
        action_id: &ActionId,
        context: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        let action = self.action_registry.get(action_id)
            .map_err(|e| ActionError::fatal(e.to_string()))?;
        let metadata = action.metadata();

        // 1. Resolve isolation level
        let isolation = self.resolve_isolation_level(metadata);

        let result = match isolation {
            IsolationLevel::None => {
                // Trusted builtin: напрямую
                action.execute(context).await?
            }
            IsolationLevel::CapabilityGated | IsolationLevel::Isolated => {
                let sandboxed = SandboxedContext::new(
                    context,
                    metadata.capabilities.clone(),
                );
                // SandboxRunner enforce-ит capabilities + isolation
                self.sandbox.execute(action.as_ref(), sandboxed, metadata).await?
            }
        };

        // 2. Enforce data limits на output
        self.enforce_data_limits(&result)?;

        Ok(result)
    }
}

// SandboxedContext проксирует вызовы через capability checks
impl SandboxedContext {
    pub async fn get_resource<R: Resource>(&self) -> Result<R::Instance, ActionError> {
        self.check_capability(&Capability::Resource(R::resource_id()))?;
        self.inner.get_resource::<R>().await
    }

    pub async fn get_credential(&self, id: &str) -> Result<AuthData, ActionError> {
        self.check_capability(&Capability::Credential(CredentialId::new(id)))?;
        self.inner.get_credential(id).await
    }

    /// CancellationToken — action проверяет в длинных операциях
    pub fn check_cancelled(&self) -> Result<(), ActionError> {
        self.inner.check_cancelled()
    }
}

// Пример violation:
// Action "community.risky_plugin" запрашивает get_resource::<DatabaseResource>()
// но у него нет Capability::Resource("database")
// → ActionError::SandboxViolation { capability: "database", action_id: "community.risky_plugin" }
```

---

## 6. telemetry ↔ execution (events как проекции)

**Паттерн:** ExecutionRepo transition → emit event.
Events — "best effort", могут потеряться. ExecutionRepo — восстанавливаемый.

```rust
// execution: после state transition → emit через telemetry
impl ExecutionContext {
    pub async fn complete_node(&self, node_id: &NodeId, output: serde_json::Value) -> Result<()> {
        // 1. ExecutionRepo — source of truth (atomic CAS + journal)
        self.execution_repo.transition(
            &self.execution_id,
            ExecutionStatus::Running,
            ExecutionStatus::Running, // ещё не финальный
            JournalEntry::NodeAttempt(/* ... */),
        ).await?;

        // 2. Telemetry — проекция
        self.telemetry.emit(NodeEvent::Completed {
            execution_id: self.execution_id.clone(),
            node_id: node_id.clone(),
            duration: elapsed,
        }).await?;

        Ok(())
    }
}

// telemetry: подписчики получают проекции
impl Telemetry {
    pub fn setup_event_logging(&self) {
        self.event_bus.subscribe(|event: ExecutionEvent| async move {
            match event {
                ExecutionEvent::Completed { execution_id, duration, .. } => {
                    info!(execution_id = %execution_id, "Completed in {:?}", duration);
                }
                ExecutionEvent::Failed { execution_id, error, .. } => {
                    error!(execution_id = %execution_id, error = %error, "Failed");
                }
                _ => {}
            }
        });
    }

    pub fn setup_metrics(&self) {
        self.event_bus.subscribe(|event: NodeEvent| async move {
            match event {
                NodeEvent::Completed { duration, .. } => {
                    metrics::histogram!("node_duration_seconds", duration.as_secs_f64());
                }
                NodeEvent::Failed { .. } => {
                    metrics::increment_counter!("nodes_failed_total");
                }
                _ => {}
            }
        });
    }
}

// ВАЖНО: при recovery — из ExecutionRepo (journal), НЕ из событий
```

---

## 7. config → preset → drivers selection

**Паттерн:** Config определяет поведение, cargo features определяют возможности.

**Важно: StaticConfig vs DynamicConfig.**
Не всё можно менять на лету. Hot-reload допустим только для DynamicConfig.

```rust
/// Только при старте процесса. Смена требует рестарт.
pub struct StaticConfig {
    pub storage_backend: String,      // "sqlite" | "postgres"
    pub queue_backend: String,        // "memory" | "redis"
    pub sandbox_driver: String,       // "inprocess" | "wasm"
    pub blob_backend: String,         // "fs" | "s3"
    pub api_bind: String,
}

/// Можно менять на лету через hot-reload.
pub struct DynamicConfig {
    pub max_concurrent_executions: usize,
    pub default_timeout: Duration,
    pub rate_limits: RateLimitConfig,
    pub tenant_quotas: HashMap<TenantId, TenantQuota>,
    pub sandbox_limits: SandboxLimitsConfig,  // memory, cpu time
}

// Engine/runtime принимает только DynamicConfig на hot-reload:
impl WorkflowEngine {
    pub fn apply_dynamic_config(&self, config: &DynamicConfig) {
        self.scheduler.update_concurrency(config.max_concurrent_executions);
        self.rate_limiter.update(config.rate_limits);
        // НЕ меняем drivers на лету — это StaticConfig
    }
}
```

```rust
// В bin-крейте: StaticConfig → выбор drivers при старте
pub fn build_app(config: &StaticConfig) -> Result<App> {
    let storage: Arc<dyn ExecutionRepo> = match config.storage_backend.as_str() {
        "sqlite" => {
            #[cfg(feature = "lite")]
            { Arc::new(SqliteStorage::new(&config.storage_path).await?) }
            #[cfg(not(feature = "lite"))]
            { return Err(anyhow!("sqlite support not compiled")) }
        }
        "postgres" => {
            #[cfg(feature = "full")]
            { Arc::new(PostgresStorage::new(&config.storage_url).await?) }
            #[cfg(not(feature = "full"))]
            { return Err(anyhow!("postgres support not compiled")) }
        }
        other => return Err(anyhow!("unknown storage backend: {}", other)),
    };

    let sandbox: Arc<dyn SandboxRunner> = match config.sandbox_driver.as_str() {
        "inprocess" => Arc::new(InProcessSandbox::new()),
        "wasm" => {
            #[cfg(feature = "full")]
            { Arc::new(WasmSandbox::new(config.wasm_config())) }
            #[cfg(not(feature = "full"))]
            { return Err(anyhow!("wasm sandbox not compiled")) }
        }
        _ => Arc::new(InProcessSandbox::new()),
    };

    Ok(App { engine, runtime, worker, ... })
}
```

---

## 8. resource ↔ system (pressure hooks)

**Паттерн:** SystemMonitor детектирует pressure → ResourceManager реагирует по policy.

```rust
impl ResourceManager {
    pub fn connect_pressure_hooks(&self, system_monitor: &SystemMonitor) {
        let manager = self.clone();
        system_monitor.on_pressure(move |level| {
            match level {
                PressureLevel::Warning => {
                    // Evict idle ресурсы у которых policy = EvictIdle
                    manager.evict_by_policy(PressureAction::EvictIdle);
                }
                PressureLevel::Critical => {
                    // Evict всё кроме active
                    manager.evict_by_policy(PressureAction::EvictAll);
                }
                _ => {}
            }
        });
    }
}
```

---

## 9. registry ↔ action (interface versioning)

**Паттерн:** Registry хранит ActionMetadata с interface_version и schema_hash.
WorkflowDefinition привязывается к interface_version, не к package version.

```rust
// При обновлении Action
impl Registry {
    fn check_compatibility(&self, existing: &ActionMetadata, new: &ActionMetadata) -> Result<()> {
        // Если schema_hash совпадает — контракт не изменился, ok
        if existing.schema_hash == new.schema_hash {
            return Ok(());
        }

        // Schema изменилась — interface_version.major должен быть bumped
        if new.interface_version.major <= existing.interface_version.major {
            return Err(RegistryError::IncompatibleUpdate {
                reason: "Schema changed but interface_version.major not bumped".into(),
            });
        }

        // Проверяем что есть migration rules
        if new.migrations.is_empty() {
            warn!("Schema changed without migration rules");
        }

        Ok(())
    }
}

// При запуске workflow — engine проверяет что interface_version совместима
// и при необходимости применяет SchemaMigration (rename param, add default, etc.)
```

---

## Итоговая диаграмма потоков

```
Пользователь
    │
    ▼
┌───────────────┐     config.toml          ┌───────────────┐
│   API (axum)  │◄─────────────────────────│    config      │
└───────┬───────┘                          └───────────────┘
        │
        ▼
┌───────────────┐  builds plan    ┌────────────────┐
│    engine     │────────────────►│ ExecutionPlan   │
│  (scheduler)  │                 │ (budget, graph) │
└───────┬───────┘                 └────────────────┘
        │
        │ transition (CAS)         emit (projection)
        ▼                          ▼
┌───────────────┐          ┌────────────────┐
│ ExecutionRepo │          │   telemetry    │
│ (ports trait) │          │ (eventbus +    │
│ ═══════════   │          │  metrics/log)  │
│ SQLite impl   │          └────────────────┘
│ Postgres impl │
└───────┬───────┘
        │ enqueue
        ▼
┌───────────────┐
│  TaskQueue    │
│ (ports trait) │
│ ═══════════   │
│ Memory impl   │
│ Redis impl    │
└───────┬───────┘
        │ dequeue
        ▼
┌───────────────┐  Semaphore    ┌────────────────┐
│    worker     │──────────────►│    runtime     │
│ (bounded)     │               │                │
└───────────────┘               └───────┬────────┘
                                        │
                                        ▼
                                ┌────────────────┐
                                │ SandboxRunner  │
                                │ (ports trait)  │
                                │ ═══════════    │
                                │ InProcess impl │
                                │ WASM impl      │
                                └───────┬────────┘
                                        │
                                        ▼
                                ┌────────────────┐
                                │    action      │
                                │ (capability    │
                                │  checked ctx)  │
                                └────────────────┘
```

## Ключевые паттерны (сводка)

1. **Core → ports (traits) → drivers (impls)** — жёсткое направление зависимостей
2. **Bins = composition roots** — склеивают core + drivers по config/features
3. **ExecutionRepo = truth** — единственный источник истины (journal + CAS), events = проекции
4. **Execution Plan → bounded budget** — никаких unbounded spawn
5. **Sandbox: community = Isolated always** — CapabilityGated для first-party, Isolated (WASM) для community
6. **Queue: at-least-once + idempotency** — формализованный порядок journal/ack + reaper
7. **StaticConfig vs DynamicConfig** — drivers не меняются на лету
8. **Resource policies** — eviction/TTL/pressure hooks
9. **Interface versioning** — schema_hash + SchemaMigration в registry
10. **Three presets** — desktop/selfhost/cloud = разные drivers, одно ядро
