# Nebula Architecture — Layers Interaction

## Interaction Principles

1.  **Ports & Drivers** — Core speaks only to trait interfaces (`ports`), drivers implement them.
2.  **ExecutionRepo = Truth** — `ports::ExecutionRepo` (journal + CAS) = single source of truth, events = projections.
3.  **Dependency Injection** — Bins assemble core + drivers via composition root.
4.  **Typed + Dynamic** — Typed Actions (first-party) + dynamic `serde_json::Value` (community).
5.  **Bounded Concurrency** — Semaphore + bounded channels, no unbounded spawns.
6.  **Execution Plan** — Plan with budget is built BEFORE execution.
7.  **Capability-based Sandbox** — Community actions are always fully isolated.

---

## 1. core ↔ ports (Fundamental Boundary)

**Rule:** Core crates NEVER depend on drivers. Only on `ports` (traits).

```rust
// core/engine depends on ports::TaskQueue (trait), not queue-redis (impl)
pub struct WorkflowEngine {
    execution_repo: Arc<dyn ExecutionRepo>,    // ports trait
    workflow_repo: Arc<dyn WorkflowRepo>,      // ports trait
    task_queue: Arc<dyn TaskQueue>,            // ports trait
    telemetry: Arc<Telemetry>,
}

// runtime depends on ports::SandboxRunner (trait), not sandbox-wasm (impl)
pub struct ActionRuntime {
    sandbox: Arc<dyn SandboxRunner>,           // ports trait
    action_registry: Arc<ActionRegistry>,
}

// Concrete implementations are substituted in bins (composition root)
```

**Why this is important:**
-   Desktop build does not compile postgres/redis/s3/wasmtime.
-   The same engine works with SQLite (desktop) and Postgres (cloud).
-   Core testing is done without external dependencies (in-memory mocks).

---

## 2. bins → drivers → ports (Composition Root)

**Pattern:** Each bin crate = composition root. Reads config, creates drivers, injects into core.

```rust
// bins/desktop/src/main.rs
#[tokio::main]
async fn main() -> Result<()> {
    let config: NebulaConfig = ConfigManager::load("config.desktop.toml").await?;

    // 1. Create drivers (concrete implementations of ports traits)
    let storage = SqliteStorage::new(&config.storage.path).await?;
    let blobs = FsBlobStore::new(&config.blobs.root)?;
    let queue = MemoryQueue::new(config.engine.max_concurrent_executions);
    let sandbox = InProcessSandbox::new();
    let secrets = LocalSecretsStore::new(&config.secrets.path)?;

    // 2. Assemble core (engine/runtime) with drivers via ports traits
    let engine = WorkflowEngine::builder()
        .execution_repo(Arc::new(storage.clone()) as Arc<dyn ExecutionRepo>)
        .workflow_repo(Arc::new(storage.clone()) as Arc<dyn WorkflowRepo>)
        .task_queue(Arc::new(queue) as Arc<dyn TaskQueue>)
        .build()?;

    let runtime = ActionRuntime::builder()
        .sandbox(Arc::new(sandbox) as Arc<dyn SandboxRunner>)
        .build()?;

    // 3. Start worker + API
    let worker = Worker::new(engine.task_queue(), runtime, config.engine.max_concurrent_executions);

    tokio::select! {
        _ = worker.run() => {},
        _ = start_api(&config.api.bind, engine) => {},
    }

    Ok(())
}

// bins/server/src/main.rs — same, but with postgres/redis drivers
// bins/worker/src/main.rs — only runtime + queue (no engine/API)
// bins/control-plane/src/main.rs — only engine + API (no runtime)
```

---

## 3. engine ↔ execution ↔ ports::ExecutionRepo

**Flow:** Engine builds plan → writes to ExecutionRepo → enqueues in TaskQueue.

```rust
impl WorkflowEngine {
    pub async fn execute_workflow(&self, workflow_id: WorkflowId, input: serde_json::Value) -> Result<ExecutionHandle> {
        let workflow = self.workflow_repo.get(&workflow_id).await?
            .ok_or(EngineError::WorkflowNotFound)?;

        // 1. Build ExecutionPlan (dependency graph, parallel groups, budget)
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

        // 3. Enqueue (via ports::TaskQueue)
        self.task_queue.enqueue(Task {
            execution_id: execution_id.clone(),
            workflow_id,
            input,
            budget: plan.budget,
        }).await?;

        // 4. Telemetry — PROJECTION, not source of truth
        self.telemetry.emit(ExecutionEvent::Started {
            execution_id: execution_id.clone(),
        }).await?;

        Ok(ExecutionHandle::new(execution_id))
    }
}
```

**Important:** On recovery after crash — state comes from ExecutionRepo, NOT from events.

---

## 4. worker ↔ ports::TaskQueue ↔ runtime ↔ ports::SandboxRunner

**Delivery Semantics: at-least-once.**
Consistency is achieved via `idempotency_key` in `NodeAttempt`.

**Mandatory Operation Order (worker loop):**
1.  `dequeue` — task received, visibility timeout started.
2.  `acquire_lease` in ExecutionRepo (if distributed).
3.  journal append: attempt started.
4.  execute action (via SandboxRunner).
5.  journal append: attempt completed/failed.
6.  `ack` (success) or `nack` (retry).

**Failure scenarios:**
-   Worker crashes between 3 and 6 → visibility timeout expires → task returns to queue → `idempotency_key` prevents duplicate execution.
-   Worker crashes before 3 → task returns, as if nothing happened.
-   Queue loses ack → re-delivery → idempotency check.

```rust
impl Worker {
    async fn run(&self) {
        // Reaper: periodically claim stale entries from dead consumers
        let reaper_handle = self.spawn_reaper();

        loop {
            // 1. Dequeue with visibility timeout
            let task = match self.task_queue.dequeue(Duration::from_secs(5)).await {
                Ok(Some(task)) => task,
                _ => continue,
            };

            // 2. Bounded concurrency — Semaphore (NEVER unbounded spawn)
            let permit = self.concurrency_semaphore
                .clone().acquire_owned().await
                .expect("semaphore closed");

            let runtime = self.runtime.clone();
            let queue = self.task_queue.clone();
            let execution_repo = self.execution_repo.clone();

            tokio::spawn(async move {
                let _permit = permit; // Drop = release

                // 3. Acquire lease (distributed mode)
                // In single-process (desktop) — no-op
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

                // 7. Ack/nack in queue (AFTER journal)
                // Fatal → ack (do not retry). Retryable → nack (returns to queue).
                match &result {
                    Ok(_) => queue.ack(&task.id).await.ok(),
                    Err(e) if e.is_retryable() => queue.nack(&task.id).await.ok(),
                    Err(_) => queue.ack(&task.id).await.ok(), // Fatal — do not retry
                };
            });
        }
    }

    /// Reaper: periodically claim stale entries from dead consumers
    fn spawn_reaper(&self) -> JoinHandle<()> {
        let queue = self.task_queue.clone();
        let execution_repo = self.execution_repo.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                // Claim tasks stuck with dead consumers
                if let Ok(stale) = queue.claim_stale(Duration::from_secs(60)).await {
                    for task in stale {
                        // Re-enqueue or handle
                        queue.nack(&task.id).await.ok();
                    }
                }
                // Also: check stale leases in ExecutionRepo
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

## 5. runtime ↔ sandbox ↔ action (Capability Enforcement + Data Limits)

**Flow:** Runtime determines IsolationLevel → creates SandboxedContext → calls SandboxRunner → enforces data limits.

**Return type:** `Result<ActionResult<serde_json::Value>, ActionError>`.
-   `Ok(ActionResult::*)` = flow control (Success, Branch, Continue, Wait, ...)
-   `Err(ActionError::Retryable)` → engine decides retry policy → nack
-   `Err(ActionError::Fatal)` → fail fast → ack (do not retry)
-   `Err(ActionError::Cancelled)` → execution cancelled → nack

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
                // Trusted builtin: direct execution
                action.execute(context).await?
            }
            IsolationLevel::CapabilityGated | IsolationLevel::Isolated => {
                let sandboxed = SandboxedContext::new(
                    context,
                    metadata.capabilities.clone(),
                );
                // SandboxRunner enforces capabilities + isolation
                self.sandbox.execute(action.as_ref(), sandboxed, metadata).await?
            }
        };

        // 2. Enforce data limits on output
        self.enforce_data_limits(&result)?;

        Ok(result)
    }
}

// SandboxedContext proxies calls through capability checks
impl SandboxedContext {
    pub async fn get_resource<R: Resource>(&self) -> Result<R::Instance, ActionError> {
        self.check_capability(&Capability::Resource(R::resource_id()))?;
        self.inner.get_resource::<R>().await
    }

    pub async fn get_credential(&self, id: &str) -> Result<AuthData, ActionError> {
        self.check_capability(&Capability::Credential(CredentialId::new(id)))?;
        self.inner.get_credential(id).await
    }

    /// CancellationToken — action checks this in long operations
    pub fn check_cancelled(&self) -> Result<(), ActionError> {
        self.inner.check_cancelled()
    }
}

// Violation example:
// Action "community.risky_plugin" requests get_resource::<DatabaseResource>()
// but it does not have Capability::Resource("database")
// → ActionError::SandboxViolation { capability: "database", action_id: "community.risky_plugin" }
```

---

## 6. telemetry ↔ execution (Events as Projections)

**Pattern:** ExecutionRepo transition → emit event.
Events — "best effort", can be lost. ExecutionRepo — recoverable.

```rust
// execution: after state transition → emit via telemetry
impl ExecutionContext {
    pub async fn complete_node(&self, node_id: &NodeId, output: serde_json::Value) -> Result<()> {
        // 1. ExecutionRepo — source of truth (atomic CAS + journal)
        self.execution_repo.transition(
            &self.execution_id,
            ExecutionStatus::Running,
            ExecutionStatus::Running, // not final yet
            JournalEntry::NodeAttempt(/* ... */),
        ).await?;

        // 2. Telemetry — projection
        self.telemetry.emit(NodeEvent::Completed {
            execution_id: self.execution_id.clone(),
            node_id: node_id.clone(),
            duration: elapsed,
        }).await?;

        Ok(())
    }
}

// telemetry: subscribers receive projections
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

// IMPORTANT: on recovery — from ExecutionRepo (journal), NOT from events
```

---

## 7. config → preset → drivers selection

**Pattern:** Config defines behavior, cargo features define capabilities.

**Important: StaticConfig vs DynamicConfig.**
Not everything can be changed on the fly. Hot-reload is allowed only for DynamicConfig.

```rust
/// Only at process start. Change requires restart.
pub struct StaticConfig {
    pub storage_backend: String,      // "sqlite" | "postgres"
    pub queue_backend: String,        // "memory" | "redis"
    pub sandbox_driver: String,       // "inprocess" | "wasm"
    pub blob_backend: String,         // "fs" | "s3"
    pub api_bind: String,
}

/// Can be changed on the fly via hot-reload.
pub struct DynamicConfig {
    pub max_concurrent_executions: usize,
    pub default_timeout: Duration,
    pub rate_limits: RateLimitConfig,
    pub tenant_quotas: HashMap<TenantId, TenantQuota>,
    pub sandbox_limits: SandboxLimitsConfig,  // memory, cpu time
}

// Engine/runtime accepts only DynamicConfig for hot-reload:
impl WorkflowEngine {
    pub fn apply_dynamic_config(&self, config: &DynamicConfig) {
        self.scheduler.update_concurrency(config.max_concurrent_executions);
        self.rate_limiter.update(config.rate_limits);
        // DO NOT change drivers on the fly — this is StaticConfig
    }
}
```

```rust
// In bin crate: StaticConfig → driver selection at start
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

**Pattern:** SystemMonitor detects pressure → ResourceManager reacts via policy.

```rust
impl ResourceManager {
    pub fn connect_pressure_hooks(&self, system_monitor: &SystemMonitor) {
        let manager = self.clone();
        system_monitor.on_pressure(move |level| {
            match level {
                PressureLevel::Warning => {
                    // Evict idle resources with policy = EvictIdle
                    manager.evict_by_policy(PressureAction::EvictIdle);
                }
                PressureLevel::Critical => {
                    // Evict everything except active
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

**Pattern:** Registry stores ActionMetadata with interface_version and schema_hash.
WorkflowDefinition binds to interface_version, not package version.

```rust
// On Action update
impl Registry {
    fn check_compatibility(&self, existing: &ActionMetadata, new: &ActionMetadata) -> Result<()> {
        // If schema_hash matches — contract unchanged, ok
        if existing.schema_hash == new.schema_hash {
            return Ok(());
        }

        // Schema changed — interface_version.major must be bumped
        if new.interface_version.major <= existing.interface_version.major {
            return Err(RegistryError::IncompatibleUpdate {
                reason: "Schema changed but interface_version.major not bumped".into(),
            });
        }

        // Check that migration rules exist
        if new.migrations.is_empty() {
            warn!("Schema changed without migration rules");
        }

        Ok(())
    }
}

// On workflow start — engine checks if interface_version is compatible
// and applies SchemaMigration (rename param, add default, etc.) if needed
```

---

## Summary Diagram

```
User
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
        │ enqueue
        ▼
┌───────────────┐
│  TaskQueue    │
│ (ports trait) │
│ ═══════════   │
│ Memory impl   │
│ Redis impl    │
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

## Key Patterns (Summary)

1.  **Core → ports (traits) → drivers (impls)** — strict dependency direction.
2.  **Bins = composition roots** — glue core + drivers based on config/features.
3.  **ExecutionRepo = truth** — single source of truth (journal + CAS), events = projections.
4.  **Execution Plan → bounded budget** — no unbounded spawns.
5.  **Sandbox: community = Isolated always** — CapabilityGated for first-party, Isolated (WASM) for community.
6.  **Queue: at-least-once + idempotency** — formal journal/ack + reaper order.
7.  **StaticConfig vs DynamicConfig** — drivers cannot be changed on the fly.
8.  **Resource policies** — eviction/TTL/pressure hooks.
9.  **Interface versioning** — schema_hash + SchemaMigration in registry.
10. **Three presets** — desktop/selfhost/cloud = different drivers, same core.
