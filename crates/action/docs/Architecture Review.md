# nebula-action Architecture Review

> Date: 2026-02-14 (updated 2026-02-15)
> Rounds: 2 (Round 1: competitor research + assessment; Round 2: design synthesis)
> Scope: Deep architectural analysis of crates/action with competitor research, Rust pattern assessment, evolved design, and integration with parameter, resource, idempotency, and SDK systems
> Sources: actionv2.md, action_resource.md, Idempotency.md, tmp/ document collection (~35 docs + 2 prototype crates)

---

## Executive Summary

The nebula-action crate has a **solid foundation** (typed action traits, strong error model, cancellation support) that needs to be completed end-to-end. The deliberate design of multiple action types (Process, Stateful, Trigger, Streaming, Transactional, Interactive) provides **type safety and ergonomics** that no competitor offers. The path forward is to build adapters, executors, and engine support for all action types incrementally, while evolving ActionResult with a separate ActionOutput enum.

**Design philosophy:** Rich action types + rich ActionResult = developers build nodes easily, the core engine handles everything.

**Key decisions from Round 2:**
- Keep ALL action types — each serves a distinct use case with distinct safety guarantees
- Separate ActionOutput from ActionResult flow control
- Typed contexts (`ProcessContext: ActionContext`) with generic execute methods
- Blanket impls (`impl<T: ProcessAction> TypedAction for T`) to eliminate boilerplate
- Per-type Executors (not adapters) as the coordination layer
- Per-type internal handler traits with `HandlerDispatch` enum
- Frontier-based engine execution (replacing static level-by-level)

**Additions from document collection (tmp/):**
- 18-type parameter system with expression pipeline and ResourceParameter universal loader
- Trait-based resource management with pooling, health monitoring, and tier-aware configuration
- Multi-level idempotency (Action, Workflow, Request, Transaction) with executor composition
- Marker-based resource context (`ActionContext<R: ResourceType>`)
- SDK vision: `#[derive(Node)]`, test framework, dev server, CLI tooling
- Production targets: <10ms node latency, 1000 exec/sec, 99.9% uptime
- Two prototype crates (nebula-idempotency ~55%, nebula-resource ~45%) validating trait-first design

---

## Part 1: Competitor Analysis

### n8n (TypeScript, node-based workflow automation)

**Core abstraction:** `INodeType` interface with optional methods (`execute`, `poll`, `trigger`, `webhook`, `supplyData`). A single interface covers all node types via optional methods.

**What they do well:**
- Declarative node description (`INodeTypeDescription`) with display name, inputs/outputs, properties — auto-generates UI
- Data model: `INodeExecutionData[][]` — array of arrays enabling multiple outputs per port
- `continueOnFail` at node level — practical error resilience
- Trigger model cleanly separates poll/webhook/trigger via separate optional methods
- Low boilerplate: JSON descriptor + execute function = ~15 lines
- AI chain support (`supplyData` for LangChain-style nodes)
- Rich parameters system with display conditions and dynamic loading

**Weaknesses:**
- No type safety (JavaScript/TypeScript with runtime validation)
- `execute` receives a god-object context (`IExecuteFunctions`) with 30+ helper methods
- No compile-time guarantee that a node handles all its declared operations
- Single `execute()` with resource/operation branching creates deeply nested code

**Takeaway for Nebula:** Our multi-trait approach is architecturally superior to n8n's single-interface-with-optional-methods. We get compile-time type safety AND per-type ergonomics. Adopt n8n's strengths: declarative parameters, AI chain support (via SupplyAction), rich UI generation from schemas.

---

### Temporal (Go/Rust, durable execution platform)

**Core abstraction:** Activities (short-lived functions) and Workflows (long-running durable processes).

**What they do well:**
- Minimal boilerplate: an activity is literally a function
- Rich retry policies: `initial_interval`, `backoff_coefficient`, `maximum_interval`, `maximum_attempts`, `non_retryable_error_types`
- 4 timeout types: `ScheduleToStart`, `StartToClose`, `ScheduleToClose`, `Heartbeat`
- Heartbeating for long-running activities with progress recovery
- Cancellation types: `TRY_CANCEL`, `WAIT_CANCELLATION_COMPLETED`, `ABANDON`
- Interceptor middleware: `before_execute` / `after_execute` / `on_error`
- Event sourcing for workflow state reconstruction

**Takeaway for Nebula:** Adopt declarative retry policy (5-parameter model) and 4-timeout taxonomy on ActionMetadata. Heartbeat-based progress recovery is essential for long-running actions. Interceptor pattern validates middleware at execution boundary.

---

### Windmill (Rust backend, multi-language scripts)

**What they do well:**
- Auto-inferred JSON schemas from function signatures — zero manual schema definition
- Rich approval/suspend mechanism: `required_events`, `timeout`, `resume_form`, `user_auth_required`
- `failure_module` — dedicated error handler per flow step
- `RetryStatus` tracking attempt count and previous result

**Takeaway for Nebula:** Auto-inferred schemas from Rust types via `JsonSchema` derive is a killer feature. Windmill's approval flow is best-in-class — our InteractiveAction has the right shape. Their `failure_module` concept (dedicated error handler per step) is worth adopting.

---

### Airflow, Prefect, Dagger

**Airflow:** XCom's explicit push/pull data passing model is better than shared mutable variables. Data passing should be explicit and typed, not a shared mutable bag.

**Prefect:** `@task` decorator-to-function approach confirms: lowest-boilerplate = "a function IS an action." Our derive macro should aspire to this simplicity.

**Dagger:** Content-addressed caching via `hash(action_key, version, input_hash)` for automatic memoization. Could inform `nebula-memory` crate design.

---

### Competitor Comparison Matrix

| Dimension | n8n | Temporal | Windmill | Nebula (target) |
|-----------|-----|----------|----------|-----------------|
| Core unit | INodeType | Activity (function) | Script (code) | Action (typed traits) |
| Boilerplate | ~15 lines | ~5 lines | ~10 lines | **~15 lines** (with derive) |
| Type safety | Runtime | Runtime (protobuf) | Runtime (JSON) | **Compile-time** |
| Action types | 1 interface | 1 function | 1 script | **10+ specialized traits** |
| Retry policy | Per-node config | Declarative 5-param | Per-step config | **Declarative on metadata** |
| Flow control | Separate routing | Workflow-level | Flow modules | **Exhaustive enum** |
| Approval flow | Partial | Workflow signals | Full suspend | **InteractiveAction** |
| Data types | JSON items | Protobuf Payload | JSON + S3 | **ActionOutput (Value/Binary/Reference/Stream)** |
| AI chain | supplyData | N/A | N/A | **SupplyAction** |
| Streaming | N/A | N/A | N/A | **StreamingAction** |
| Transactions | N/A | Saga pattern | N/A | **TransactionalAction (2PC)** |
| Parameters | 15+ types, display conditions | Config-based | JSON schema | **18 types, expression pipeline** |
| Idempotency | N/A | Built-in (event sourcing) | N/A | **4-level system** |
| Resources | Built-in per node | Activity-managed | Script-managed | **Trait-based pooling** |

**Nebula's unique advantage:** Compile-time type safety with specialized traits per action type. No other workflow engine offers this combination.

---

### Cross-Cutting Patterns to Adopt

1. **Schema-from-code (Windmill):** `JsonSchema` derive on Rust input/output types
2. **Declarative retry policy (Temporal):** 5-parameter config on ActionMetadata
3. **Timeout taxonomy (Temporal):** 4 timeout types for different failure modes
4. **Content-addressed caching (Dagger):** `hash(action_key, version, input_hash)`
5. **Failure module (Windmill):** Dedicated error handler per workflow step
6. **Interceptor middleware (Temporal):** `before_execute` / `after_execute` hooks at execution boundary
7. **Explicit data passing (Airflow XCom):** Replace shared mutable variables with explicit input/output model
8. **Expression pipeline (n8n):** Three-phase expression → validation → processing

---

## Part 2: Current Architecture Assessment

### Critical Issues (Must Fix)

#### 1. `expect()` in ProcessActionAdapter (Severity: Critical)

At `crates/action/src/adapters/process.rs:61`:
```rust
serde_json::to_value(output).expect("action output must be serializable")
```

This panics the runtime if output serialization fails. Needs `try_map_output()` on `ActionResult<T>`:

```rust
impl<T> ActionResult<T> {
    pub fn try_map_output<U, E>(self, f: impl FnMut(T) -> Result<U, E>) -> Result<ActionResult<U>, E> {
        // fallible version of map_output
    }
}
```

#### 2. No Retry Mechanism (Severity: High)

`ActionError::Retryable` exists with `backoff_hint`, but nothing in the runtime or engine actually retries. The resilience crate has `RetryPolicy` — it's not connected.

**Solution:** Declarative retry policy + timeout taxonomy on `ActionMetadata`:
```rust
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_interval: Duration,
    pub backoff_coefficient: f64,
    pub max_interval: Duration,
    pub non_retryable_errors: Vec<String>,
}

pub struct TimeoutPolicy {
    pub schedule_to_start: Option<Duration>,
    pub start_to_close: Option<Duration>,
    pub schedule_to_close: Option<Duration>,
    pub heartbeat: Option<Duration>,
}
```

#### 3. Only 1 of 6 Action Types Has a Working Pipeline

| Trait | Adapter | Runtime | Engine |
|-------|---------|---------|--------|
| ProcessAction | ProcessActionAdapter | Yes | Yes |
| StatefulAction | **None** | No | No |
| TriggerAction | **None** | No | No |
| StreamingAction | **None** | No | No |
| TransactionalAction | **None** | No | No |
| InteractiveAction | **None** | No | No |

All 6 types must work end-to-end. Build adapters/executors incrementally as each type gains its first consumer.

---

## Part 3: Evolved Design (Round 2 Consensus)

### 3.1 Trait Hierarchy

The full trait hierarchy from the design document, organized by tier:

```
Action (base trait)
├── ProcessAction (stateless operations — ~80% of nodes)
├── StatefulAction (iteration, accumulation, caching)
├── TriggerAction (event sources — poll, webhook, cron)
│   ├── WebhookAction : TriggerAction (webhook-specific convenience)
│   └── PollingAction : TriggerAction (polling-specific convenience)
├── SupplyAction (resource management, AI chain data providers)
├── StreamingAction (real-time data processing)
├── TransactionalAction (distributed transactions, 2PC/saga)
├── InteractiveAction (human-in-the-loop, forms, approvals)
└── EventSourcingAction (CQRS/event sourcing pattern)
```

**Helper traits:**
- `SimpleAction` — returns `Value`, auto-wraps in `ActionResult::Success(ActionOutput::Value(v))`

**Tier strategy:**
- **Tier 1 (build now):** Action, SimpleAction, ProcessAction, StatefulAction, TriggerAction
- **Tier 2 (build when first consumer exists):** SupplyAction, StreamingAction, InteractiveAction
- **Tier 3 (library crate, not core):** TransactionalAction, EventSourcingAction

**Lifecycle hooks on base Action trait:**
```rust
async fn on_init(&mut self, context: &InitContext) -> Result<()> { Ok(()) }
async fn on_destroy(&mut self) -> Result<()> { Ok(()) }
```

### 3.2 ActionOutput Separation

Current: `ActionResult<T>` uses `T` directly for both data and flow control.

**Evolved:** Separate `ActionOutput<T>` enum for data representation:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[non_exhaustive]
pub enum ActionOutput<T> {
    /// Structured data (primary case)
    Value(T),
    /// Binary data (images, PDFs, files)
    Binary(BinaryData),
    /// Reference to large data (S3, datasets)
    Reference(DataReference),
    /// Stream reference for streaming data
    Stream(StreamReference),
    /// No output
    Empty,
}
```

`ActionResult<T>` then uses `ActionOutput<T>` instead of bare `T`:

```rust
#[non_exhaustive]
pub enum ActionResult<T> {
    Success(ActionOutput<T>),
    Skip { reason: String },
    Retry { after: Duration, reason: String },
    Continue {
        output: ActionOutput<T>,
        progress: LoopProgress,
        delay: Option<Duration>,
    },
    Break {
        output: ActionOutput<T>,
        reason: BreakReason,
    },
    Branch {
        branch: BranchSelection,
        output: ActionOutput<T>,
        decision_metadata: Option<Value>,
    },
    MultiOutput(HashMap<PortKey, ActionOutput<T>>),
    Wait {
        wait_condition: WaitCondition,
        timeout: Option<Duration>,
        partial_output: Option<ActionOutput<T>>,
    },
    // Specialized results for advanced action types
    StreamItem { output: ActionOutput<T>, stream_metadata: StreamMetadata },
    TransactionPrepared { transaction_id: String, rollback_data: ActionOutput<T>, vote: TransactionVote },
    InteractionRequired { interaction_request: InteractionRequest, state_output: ActionOutput<T> },
    EventsEmitted { events: Vec<ActionOutput<T>>, aggregate_version: u64 },
}
```

**Benefits:**
- `map_output` on `ActionOutput` becomes trivial (one level, not 8-arm match)
- Binary data, references, and streams are first-class
- Engine can handle data routing uniformly regardless of output type
- `#[non_exhaustive]` allows adding new variants without breaking downstream

**BinaryData with inline/stored strategy:**
```rust
pub struct BinaryData {
    pub content_type: String,
    pub size: usize,
    pub storage: BinaryStorage,
    pub metadata: Option<Value>,
}

pub enum BinaryStorage {
    Inline(Vec<u8>),           // < 1MB
    Stored { storage_type: StorageType, path: String, checksum: Option<String> },
}
```

### 3.3 Typed Contexts and Executors

**Problem:** Current `ActionContext` is one struct for all action types. Different types need different capabilities (e.g., triggers need event emission, stateful actions need state persistence).

**Solution:** Per-type context traits extending a base:

```rust
pub trait ActionContext: Send + Sync {
    fn execution_id(&self) -> &ExecutionId;
    fn node_id(&self) -> &NodeId;
    fn check_cancelled(&self) -> Result<(), ActionError>;
    fn logger(&self) -> &dyn ActionLogger;
    fn metrics(&self) -> &dyn ActionMetrics;
    fn credentials(&self) -> &dyn CredentialProvider;
}

pub trait ProcessContext: ActionContext {}

pub trait StatefulContext: ActionContext {
    fn persist_state(&self, state: &[u8]) -> Result<(), ActionError>;
}

pub trait TriggerContext: ActionContext {
    fn emit_event(&self, event: TriggerEvent<Value>) -> Result<(), ActionError>;
}
```

**Per-type Executors (not adapters):** Executors are coordinators that handle the full lifecycle for each action type. They are NOT wrappers — they orchestrate:

```rust
pub struct ProcessExecutor;
impl ProcessExecutor {
    pub async fn execute<A, C>(
        &self, action: &A, context: &C, input: A::Input,
    ) -> Result<ActionResult<A::Output>, ActionError>
    where
        A: ProcessAction + Send + Sync,
        C: ProcessContext + Send + Sync,
    {
        action.validate_input(&input).await?;
        action.execute(input, context).await
    }
}

pub struct StatefulExecutor;
impl StatefulExecutor {
    /// Runs the full Continue/Break loop internally.
    /// Engine NEVER sees Continue — only Break (final) or Success.
    pub async fn execute<A, C>(
        &self, action: &A, context: &C, input: A::Input,
    ) -> Result<ActionResult<A::Output>, ActionError>
    where
        A: StatefulAction + Send + Sync,
        C: StatefulContext + Send + Sync,
    {
        let mut state = action.initialize_state(&input, context).await?;
        loop {
            let result = action.execute_with_state(input.clone(), &mut state, context).await?;
            match &result {
                ActionResult::Continue { delay, .. } => {
                    if let Some(d) = delay { tokio::time::sleep(*d).await; }
                    continue;
                }
                ActionResult::Break { .. } => return Ok(result),
                other => return Ok(other.clone()),
            }
        }
    }
}
```

**Key insight from Round 2:** Executors resolve `Continue` internally — the engine only sees 6 simplified behaviors (Success, Skip, Break, Branch, MultiOutput, Wait), not 8+. This dramatically simplifies engine design.

### 3.4 Blanket Impls and Boilerplate Reduction

**Blanket impl pattern** eliminates the manual `Action` impl for each action type:

```rust
impl<T: ProcessAction> TypedAction for T {
    fn action_type(&self) -> ActionType { ActionType::Process }
}

impl<T: StatefulAction> TypedAction for T {
    fn action_type(&self) -> ActionType { ActionType::Stateful }
}

// ... for each action type
```

Combined with `#[derive(Action)]` proc macro:

```rust
// BEFORE (current): ~40 lines
struct HttpRequest { meta: ActionMetadata }
impl Action for HttpRequest {
    fn metadata(&self) -> &ActionMetadata { &self.meta }
    fn action_type(&self) -> ActionType { ActionType::Process }
    fn parameters(&self) -> Option<&ParameterCollection> { self.meta.parameters.as_ref() }
}

// AFTER: ~15 lines
#[derive(Action)]
#[action(key = "http.request", name = "HTTP Request", category = "network")]
struct HttpRequest;
```

The blanket impl means the derive macro only needs to generate the `Action` base trait impl (metadata, parameters), not the type discriminant.

### 3.5 Per-Type Internal Handlers

**Problem:** Current single `InternalHandler` trait is one-size-fits-all. Different action types need different handler signatures.

**Solution:** Per-type handler traits + `HandlerDispatch` enum:

```rust
#[async_trait]
pub trait ProcessHandler: Send + Sync + 'static {
    async fn execute(&self, input: Value, ctx: ActionContext)
        -> Result<ActionResult<Value>, ActionError>;
    fn metadata(&self) -> &ActionMetadata;
}

#[async_trait]
pub trait StatefulHandler: Send + Sync + 'static {
    async fn execute_with_state(&self, input: Value, state: &mut Value, ctx: ActionContext)
        -> Result<ActionResult<Value>, ActionError>;
    async fn initialize_state(&self, input: &Value, ctx: &ActionContext)
        -> Result<Value, ActionError>;
    fn metadata(&self) -> &ActionMetadata;
}

#[async_trait]
pub trait TriggerHandler: Send + Sync + 'static {
    async fn poll(&self, config: &Value, last_state: Option<Value>, ctx: &ActionContext)
        -> Result<Vec<TriggerEvent<Value>>, ActionError>;
    async fn handle_webhook(&self, config: &Value, request: WebhookRequest, ctx: &ActionContext)
        -> Result<TriggerEvent<Value>, ActionError>;
    fn kind(&self, config: &Value) -> TriggerKind;
    fn metadata(&self) -> &ActionMetadata;
}

/// Dispatch enum for runtime registration
pub enum HandlerDispatch {
    Process(Box<dyn ProcessHandler>),
    Stateful(Box<dyn StatefulHandler>),
    Trigger(Box<dyn TriggerHandler>),
    // ... one variant per action type
}
```

**NodeComponents** gets per-type registration:

```rust
impl NodeComponents {
    pub fn process_action<A>(mut self, action: A) -> Self
    where A: ProcessAction + Send + Sync + 'static,
          A::Input: DeserializeOwned, A::Output: Serialize,
    {
        self.handler = Some(HandlerDispatch::Process(
            Box::new(ProcessActionAdapter::new(action))
        ));
        self
    }

    pub fn stateful_action<A>(mut self, action: A) -> Self { /* ... */ }
    pub fn trigger_action<A>(mut self, action: A) -> Self { /* ... */ }
}
```

### 3.6 Engine: Frontier-Based Execution

**Problem:** Current engine uses static level-by-level execution. Cannot handle Branch (activate specific path), Wait (pause until event), or dynamic topology changes.

**Solution:** Frontier-based execution:

```rust
struct ExecutionFrontier {
    /// Nodes ready to execute (all dependencies satisfied)
    ready: VecDeque<NodeId>,
    /// Nodes waiting for external events
    waiting: HashMap<NodeId, WaitCondition>,
    /// Completed nodes with their outputs
    completed: HashMap<NodeId, ActionResult<Value>>,
}

impl WorkflowEngine {
    async fn execute_workflow(&self, workflow: &Workflow) -> Result<WorkflowResult> {
        let mut frontier = ExecutionFrontier::from_topology(workflow);

        while let Some(node_id) = frontier.next_ready() {
            let result = self.execute_node(&node_id, &frontier.completed).await?;

            match &result {
                ActionResult::Success { .. } | ActionResult::Break { .. } => {
                    frontier.complete(node_id, result);
                    frontier.activate_dependents(&node_id, workflow);
                }
                ActionResult::Branch { selected, .. } => {
                    frontier.complete(node_id, result);
                    frontier.activate_branch(&node_id, selected, workflow);
                }
                ActionResult::Wait { condition, .. } => {
                    frontier.park(node_id, condition.clone());
                }
                ActionResult::Skip { .. } => {
                    frontier.skip(node_id);
                    frontier.skip_dependents(&node_id, workflow);
                }
                // Continue is never seen — resolved by StatefulExecutor
                _ => unreachable!("executor should resolve this"),
            }
        }

        Ok(frontier.into_result())
    }
}
```

### 3.7 Parameter System

From the comprehensive parameter design docs. The parameter system defines how actions declare their configurable inputs to the UI.

#### 18 Parameter Types

| Type | Storage | Use Case |
|------|---------|----------|
| TextParameter | String | Text, emails, URLs |
| SecretParameter | String (encrypted) | API keys, passwords |
| NumberParameter | Number | Timeouts, prices, counts |
| BooleanParameter | Boolean | Toggles, flags |
| SelectParameter | String | Fixed choice lists |
| MultiSelectParameter | Array | Multiple selections |
| RadioParameter | String | Exclusive choice UI |
| DateTimeParameter | DateTime | Date/time/both |
| CodeParameter | String | JS, SQL, JSON, HTML |
| ResourceParameter | String (resource ID) | Dynamic external data (Slack channels, DB tables) |
| FileParameter | File | Document/image uploads |
| ColorParameter | String (hex/rgb) | Color selection |
| HiddenParameter | Any | Internal state, system params |
| NoticeParameter | None (UI-only) | Info messages, warnings |
| CheckboxParameter | Boolean | Checkbox UI vs toggle |
| DateParameter | DateTime | Date-only input |
| TimeParameter | String | Time-only input |

#### Three-Phase Expression Pipeline

```
User Input → Transform Expressions → Validate → Process Action
```

1. **Transform:** Resolve `{{$json.field}}`, `{{$node.name.data}}`, `{{$workflow.id}}` expressions
2. **Validate:** Check transformed values against parameter definitions (min/max, required, patterns)
3. **Process:** Execute action with clean, validated values

#### Key Design Principles

**Platform core handles automatically:**
- Expression toggle button + mode detection
- Variable autocomplete (`$json`, `$node`, `$workflow`)
- Standard sizing, responsive layout, color themes
- Validation feedback, loading states, accessibility

**Parameter types only declare behavioral options:**
- Data constraints (`min/max`, `required`)
- Input validation types (`TextInputType::Email`)
- Code language (`CodeLanguage::JavaScript`)
- Critical behaviors (`multiline: true`)
- Business logic (`creatable: true` for tags)

**Do NOT add UI options for:** sizes, heights, styling, colors, padding, char counters, autocomplete config, preview settings.

#### ResourceParameter — Universal Loader

The most powerful pattern for dynamic external data:

```rust
ResourceParameter::dependent_resource()
    .metadata(ParameterMetadata::required("channel_id", "Slack Channel")?)
    .depends_on(vec!["workspace_id", "credential"])
    .load_with(|ctx| Box::pin(async move {
        // Custom loader — ANY external source
        // Returns Vec<ResourceItem> with id, label, description, icon, metadata
    }))
    .cache_key(|ctx| format!("slack_channels_{}", ctx.dependencies.get("workspace_id")))
    .cache(Duration::minutes(10))
    .loading_strategy(LoadingStrategy::OnDemand)
    .build()?
```

No vendor lock-in, no hardcoded resource types. Works with Slack, databases, Google Drive, GitHub, file systems, any API.

#### Integration with Action Traits

Parameters connect to actions via `parameter_schema()`:

```rust
pub trait Action: Send + Sync + 'static {
    fn metadata(&self) -> ActionMetadata;
    fn parameter_schema(&self) -> ParameterCollection;
    fn validate_parameters(&self, params: &ParameterCollection) -> Result<()> {
        self.parameter_schema().validate(params)
    }
}
```

With the derive macro:
```rust
#[derive(Action)]
#[action(key = "http.request", name = "HTTP Request")]
struct HttpRequest;

// Parameters defined via builder or auto-inferred from Input type via JsonSchema
```

### 3.8 Resource Management

From the resource management design docs and the nebula-resource prototype crate.

#### Core Trait Architecture

```
Resource (factory trait)
    ↓ creates
ResourceInstance (lifecycle-managed instance)
    ↓ pooled by
ResourcePool (pooling abstraction)
    ↓ orchestrated by
ResourceManager (central coordinator)
```

#### Key Traits

```rust
#[async_trait]
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Instance: ResourceInstance;

    fn metadata(&self) -> ResourceMetadata;
    async fn create(&self, config: &Self::Config, ctx: &ResourceContext)
        -> Result<Self::Instance, ResourceError>;
    fn validate_config(&self, config: &Self::Config) -> Result<(), ResourceError>;
    fn estimate_requirements(&self, config: &Self::Config) -> ResourceRequirements;
    fn supports_pooling(&self) -> bool;
}

#[async_trait]
pub trait ResourceInstance: Send + Sync + 'static {
    fn id(&self) -> &ResourceInstanceId;
    async fn health_check(&self) -> Result<HealthStatus, ResourceError>;
    async fn cleanup(&mut self) -> Result<(), ResourceError>;
    fn metrics(&self) -> ResourceMetrics;
    fn is_reusable(&self) -> bool;
    async fn reset(&mut self) -> Result<(), ResourceError>;
}
```

#### Health Monitoring

Resources self-report health:
```rust
pub enum HealthStatus {
    Healthy,
    Degraded { reason: String, performance_impact: f64 },
    Unhealthy { reason: String, recoverable: bool },
}
```

Automatic recovery triggers, performance impact assessment, health aggregation across resource types.

#### Pooling Strategies

Pluggable strategies: FIFO, LIFO, LRU, LFU, Weighted, Custom.

```rust
pub struct PoolConfig {
    pub min_size: usize,
    pub max_size: usize,
    pub idle_timeout: Duration,
    pub health_check_interval: Duration,
    pub strategy: PoolingStrategyType,
    pub tier_overrides: HashMap<DeploymentTier, PoolConfigOverride>,
}
```

#### Tier-Aware Configuration

```rust
pub enum DeploymentTier { Personal, Enterprise, Cloud }
```

Configuration auto-adjusts per tier (e.g., Personal: max 5 connections, Enterprise: max 20+, Cloud: auto-scaled).

#### Integration with Actions

Actions access resources via the context's `ResourceManager`:

```rust
#[async_trait]
impl ProcessAction for HttpRequestNode {
    async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
        let http_client = ctx.resource_manager()
            .get_instance::<HttpClientInstance>("http_client", &self.config)
            .await?;
        let response = http_client.get(&input.url).await?;
        Ok(ActionResult::success(response.into()))
    }
}
```

Built-in resource types: PostgreSQL (connection pool, SSL, health checks), HTTP Client (retry, compression, connection pooling), Message Queue (AMQP, channel caching).

### 3.9 Idempotency System

From the idempotency design doc and the nebula-idempotency prototype crate.

#### Multi-Level Architecture

```rust
pub enum IdempotencyLevel {
    /// Single action deduplication
    Action { deduplication_window: Duration, key_strategy: IdempotencyKeyStrategy },
    /// Workflow-level checkpointing
    Workflow { checkpoint_strategy: CheckpointStrategy, conflict_resolution: ConflictResolution },
    /// HTTP request idempotency (Idempotency-Key header)
    Request { http_key_header: String, result_cache_ttl: Duration },
    /// ACID transaction guarantees
    Transaction { distributed: bool, isolation: IsolationLevel },
}
```

#### IdempotentAction Trait

```rust
#[async_trait]
pub trait IdempotentAction: Send + Sync {
    type Input: Send + Sync;
    type Output: Send + Sync + Clone;

    fn idempotency_config(&self) -> IdempotencyConfig;
    async fn execute(&self, input: Self::Input) -> Result<Self::Output, IdempotencyError>;
}
```

#### Key Strategies

```rust
pub enum IdempotencyKeyStrategy {
    /// Auto-generate from input content hash
    ContentBased { include_fields: Vec<String>, hash_algorithm: HashAlgorithm },
    /// User provides key
    UserProvided { required: bool, validation_pattern: Option<String> },
    /// Combination: user key prefix + content hash suffix
    Hybrid { user_key_prefix: bool, content_suffix: bool },
    /// Time-windowed key
    TimeBased { window: Duration, granularity: TimeGranularity },
}

pub enum ConflictBehavior {
    ReturnPrevious,     // Return cached result
    WaitForCompletion,  // Wait for in-flight execution
    ExecuteAndMerge,    // Execute and merge results
    RejectWithError,    // Fail with conflict error
    CancelAndReplace,   // Cancel in-flight, re-execute
}
```

#### Executor Composition Pattern

The idempotency system wraps actions via composition, not inheritance:

```rust
pub struct IdempotentExecutor<A: IdempotentAction, S: IdempotencyStorage> {
    action: A,
    storage: S,
}

impl<A, S> IdempotentExecutor<A, S> {
    pub async fn execute(&self, input: A::Input) -> Result<A::Output, IdempotencyError> {
        let key = self.action.generate_idempotency_key(&input);
        // 1. Check cache
        if let Some(cached) = self.storage.get(&key).await? {
            return Ok(cached);
        }
        // 2. Execute
        let result = self.action.execute(input).await?;
        // 3. Store result
        self.storage.set(key, result.clone()).await?;
        Ok(result)
    }
}
```

#### Workflow Checkpointing

```rust
pub struct WorkflowCheckpoint {
    pub workflow_id: WorkflowId,
    pub completed_nodes: HashSet<NodeId>,
    pub state: WorkflowState,
    pub node_outputs: HashMap<NodeId, Value>,
    pub variables: HashMap<String, Value>,
}
```

Checkpoint strategies: PerNode (after every node), PerPhase (after each parallel phase), OnBranch (at decision points), Periodic (time-based).

#### Pluggable Storage

Trait-based with multiple backends:
- **InMemory** — Personal tier, development
- **PostgreSQL** — Enterprise tier, durable
- **Redis** — Cloud tier, distributed
- Storage backend selected per `DeploymentTier`

### 3.10 Marker-Based Resource Context

From the action_resource.md design doc. Type-level encoding of whether an action uses resources.

```rust
/// Marker for actions without resources
pub struct NoResources;

/// Marker for actions with resources
pub struct WithResources<R> { _phantom: PhantomData<R> }

/// Base trait parameterized by resource type
pub trait ActionContext<R: ResourceType = NoResources>: Send + Sync {
    fn execution_id(&self) -> &ExecutionId;
    fn node_id(&self) -> &NodeId;
    fn log(&self) -> &dyn Logger;
    fn metrics(&self) -> &dyn MetricsCollector;
    fn is_cancelled(&self) -> bool;
}

/// Extension for resource-bearing contexts
pub trait ResourceContext<R: ResourceType>: ActionContext<R> {
    fn resources(&self) -> &R;
}
```

**Pragmatic approach:** Since negative trait bounds (`R: !NoResources`) require nightly, use an extension trait:

```rust
pub trait ContextResourceExt<R: ResourceType> {
    fn resources(&self) -> Result<&R, ActionError>;
}

// Uniform API: context.resources()?
```

With derive macro for declaring resource requirements:
```rust
#[derive(Resources)]
pub struct HttpNodeResources {
    #[resource(HttpResources)]
    http: HttpClient,
    #[resource(optional)]
    cache: Option<CacheClient>,
}
```

---

## Part 4: Workspace Architecture

The full Nebula workspace vision from the architecture documents. The action crate is one piece of a larger system.

### Crate Map (12+ crates)

```
┌─────────────────────────────────────────────────────────────────────┐
│                            nebula-ui (egui)                        │
│                            nebula-api (axum REST/GraphQL/WS)       │
├─────────────────────────────────────────────────────────────────────┤
│ nebula-sdk      │ nebula-worker    │ nebula-runtime                 │
│ (builder APIs,  │ (pool, scaling,  │ (triggers, scheduling,         │
│  testing, CLI)  │  sandboxing)     │  leader election)              │
├─────────────────┼──────────────────┼────────────────────────────────┤
│ nebula-engine   │ nebula-registry  │ nebula-storage                 │
│ (DAG execution, │ (plugin loading, │ (PostgreSQL, caching)          │
│  frontier)      │  version mgmt)   │                                │
├─────────────────┴──────────────────┴────────────────────────────────┤
│ nebula-action      │ nebula-expression │ nebula-template             │
│ (THIS CRATE:       │ (expression lang, │ (Handlebars,                │
│  traits, adapters,  │  variable access) │  template funcs)            │
│  executors)        │                   │                              │
├────────────────────┴───────────────────┴─────────────────────────────┤
│ nebula-core        │ nebula-memory      │ nebula-log                  │
│ (IDs, scope,       │ (arenas, pooling,  │ (structured logging,        │
│  error types)      │  caching, interning)│  tracing)                  │
├────────────────────┼────────────────────┼─────────────────────────────┤
│ nebula-credential  │ nebula-resilience  │ nebula-resource             │
│ (storage, rotation)│ (retry, circuit    │ (pooling, health,           │
│                    │  breaker, rate)    │  tier management)           │
└────────────────────┴────────────────────┴─────────────────────────────┘
```

### Dependency Flow

```
UI/API → SDK/Runtime/Worker → Engine/Registry → Action/Expression/Template → Core/Memory/Log
```

**Key boundary:** The action crate defines traits and types. The engine crate executes them. The runtime crate schedules them. The SDK crate makes them easy to write.

### Expression System

The expression evaluator resolves `{{...}}` expressions in parameters before action execution:

- **Variables:** `$json` (previous output), `$node["name"]` (specific node), `$workflow` (workflow context), `$env` (environment)
- **Operators:** Arithmetic, comparison, logical, ternary
- **Functions:** String manipulation, date math, array operations
- **Pipeline:** Parse → Resolve variables → Evaluate → Type-check → Return value

Lives in `nebula-expression` crate, invoked during the Transform phase of the parameter pipeline.

---

## Part 5: SDK & Developer Experience

From the SDK docs, examples, and getting-started guide.

### Macro-Based Authoring (Target DX)

```rust
use nebula_sdk::prelude::*;

#[derive(Node)]
#[node(id = "http.request", name = "HTTP Request", category = "Network")]
pub struct HttpRequestNode {
    #[parameter(description = "Target URL", required)]
    url: String,

    #[parameter(description = "HTTP method", default = "GET")]
    method: String,

    #[parameter(description = "Request timeout in seconds", default = 30)]
    timeout_seconds: u64,
}

#[async_trait]
impl ProcessAction for HttpRequestNode {
    type Input = HttpRequestInput;
    type Output = HttpResponse;

    async fn execute(&self, input: Self::Input, ctx: &ActionContext)
        -> Result<ActionResult<Self::Output>, ActionError>
    {
        ctx.check_cancelled()?;
        let client = ctx.resource_manager().get_instance::<HttpClient>(...).await?;
        let response = client.request(&input).await?;
        Ok(ActionResult::success(response))
    }
}
```

### Testing Framework

```rust
#[cfg(test)]
mod tests {
    use nebula_sdk::testing::*;

    #[tokio::test]
    async fn test_http_request() {
        let node = HttpRequestNode { /* ... */ };
        let ctx = TestContext::new()
            .with_mock(MockHttpClient::returning(json!({"status": 200})));

        let result = node.execute(input, &ctx).await.unwrap();
        assert!(result.is_success());
    }
}

// Declarative test macro
test_node!(
    http_tests,
    HttpRequestNode::new(),
    get_request: json!({"url": "https://example.com"}) => is_success,
    bad_url: json!({"url": ""}) => is_error,
);

// Integration testing
let result = MockExecution::new()
    .add_node("fetch", HttpRequestNode::new())
    .add_node("transform", JsonTransformNode::new())
    .connect("fetch", "transform")
    .with_input(json!({"url": "https://api.example.com"}))
    .execute()
    .await?;
```

### Development Server

Hot-reload dev server for rapid iteration:
- Watch Rust source files for changes
- Auto-rebuild and reload nodes
- Interactive playground at `localhost:PORT/playground`
- WebSocket for live updates
- Auto-generated API docs for each node

### CLI Tooling

```bash
nebula-sdk init my-nodes --template trigger    # Scaffold project
nebula-sdk build --release                     # Build optimized
nebula-sdk test                                # Run node tests
nebula-sdk package --output my-nodes.nbp       # Package for distribution
```

### Code Generation

- **From OpenAPI specs:** Auto-generate ProcessAction nodes from API definitions
- **Workflow types:** Generate typed Input/Output structs from workflow definitions
- **Documentation:** Auto-generate markdown docs from node metadata and parameter schemas

---

## Part 6: Rust-Specific Advantages

### Patterns to ADOPT

| Pattern | Where | Why |
|---------|-------|-----|
| Native async traits | All action type traits | Removes proc macro from action author path |
| `#[non_exhaustive]` | ActionResult, ActionOutput, ActionType, ActionError, BreakReason, WaitCondition | Forward compatibility at zero cost |
| Derive macro `#[derive(Action)]` | Action base trait | 3-4x boilerplate reduction |
| `try_map_output` | ActionResult | Fix the `expect()` bug |
| `JsonSchema` derive | Action Input/Output types | Auto-inferred parameter schemas |
| Blanket impls | TypedAction for each action type trait | Eliminates manual action_type() dispatch |
| Per-type executors | ProcessExecutor, StatefulExecutor, etc. | Clean separation of coordination from logic |
| Marker types | `NoResources`/`WithResources<R>` | Compile-time resource requirement encoding |
| Compositor pattern | `IdempotentExecutor<A, S>` | Cross-cutting concerns via wrapping, not inheritance |

### Patterns to SKIP

| Pattern | Why Not |
|---------|---------|
| Tower Service/Layer | ActionResult's rich flow control is incompatible with tower's pass-through model |
| Typestate lifecycle | Actions use `dyn` dispatch at runtime — typestate requires compile-time known state |
| Effect system | Exponential type complexity; optional ports on ActionContext is pragmatic |
| Zero-copy data passing | JSON serialization is not the bottleneck; HTTP/DB latency dominates |
| Compile-time registration (inventory/linkme) | Incompatible with WASM sandbox and dynamic loading goals |
| Const generics for limits | Action configuration is dynamic (from workflow JSON) |
| Negative trait bounds | Unstable; use extension traits instead for resource context |

### Where Rust Gives Us an Edge

1. **Compile-time type safety for action I/O.** No other workflow engine catches input/output type mismatches at compile time.
2. **Zero-cost abstractions.** Trait hierarchy compiles to static dispatch; type erasure only at adapter boundary.
3. **Fearless concurrency.** `Send + Sync + 'static` bounds + `Arc<RwLock<>>` give data-race freedom.
4. **Exhaustive enum-based flow control.** Compiler forces engine to handle every ActionResult variant.
5. **Memory safety without GC.** Deterministic memory for long-running workflow engines.
6. **Rich type system for action types.** Each action trait encodes its invariants at the type level — impossible in TypeScript/Python.
7. **Marker types for resource safety.** `ActionContext<NoResources>` vs `ActionContext<WithResources<R>>` prevents resource misuse at compile time.

---

## Part 7: Production Readiness

From the phase 4-5 roadmaps and production docs.

### Performance Targets

| Metric | Target |
|--------|--------|
| Node execution latency | < 10ms |
| Workflow startup time | < 100ms |
| Throughput | 1,000 executions/sec |
| Memory per worker | < 1GB |
| API latency | < 50ms |

### Reliability Targets

| Metric | Target |
|--------|--------|
| Uptime | 99.9% |
| Data durability | 99.999% |
| Recovery time (RTO) | < 5 minutes |
| Backup frequency | Daily snapshots |

### Observability Stack

- **Metrics:** Prometheus with custom counters/histograms (`workflow_executions_total`, `workflow_execution_duration`, `active_workers`, `node_execution_errors`)
- **Tracing:** OpenTelemetry with span context propagation, sampling
- **Logging:** Structured JSON with correlation IDs, dynamic log levels, ELK/Datadog integration

### Security Model

- **Auth:** JWT + OAuth2, MFA (TOTP/WebAuthn), SSO (SAML/OIDC)
- **RBAC:** Role-based with inheritance, fine-grained per-resource permissions
- **Encryption:** TLS 1.3 minimum, AES-256-GCM at rest, envelope encryption
- **Node sandboxing:** Process isolation, memory/CPU limits, network policies, filesystem restrictions

### Deployment

- Kubernetes-native: Helm charts, HPA for workers (CPU 70%, custom metric: max 30 pending tasks)
- Blue-green deployment with automated rollback
- Multi-region with active-passive failover

---

## Part 8: Implementation Roadmap

### Phase 1: Fix Critical Issues + Foundation

1. Add `try_map_output()` to `ActionResult` — fix the `expect()` bug in `ProcessActionAdapter`
2. Add `#[non_exhaustive]` to all public enums (`ActionResult`, `BreakReason`, `WaitCondition`, `ActionType`, `ActionError`)
3. Add `into_primary_output()` method on `ActionResult` — eliminate duplication between runtime and engine
4. Add `Retry` variant to `ActionResult`
5. Add `RetryPolicy` and `TimeoutPolicy` to `ActionMetadata`

### Phase 2: ActionOutput + Blanket Impls

6. Introduce `ActionOutput<T>` enum (Value, Binary, Reference, Stream, Empty)
7. Introduce `BinaryData` with inline/stored strategy
8. Migrate `ActionResult<T>` to use `ActionOutput<T>`
9. Add blanket impls: `impl<T: ProcessAction> TypedAction for T`
10. Add `SimpleAction` helper trait

### Phase 3: Stateful + Trigger End-to-End

11. Build `StatefulActionAdapter` (bridges `StatefulAction` to `StatefulHandler`)
12. Build `StatefulExecutor` (manages Continue/Break loop)
13. Build `TriggerActionAdapter` (bridges `TriggerAction` to `TriggerHandler`)
14. Add `HandlerDispatch` enum to replace single `InternalHandler`
15. Update `NodeComponents` with per-type registration methods
16. Update runtime to dispatch by `HandlerDispatch` variant
17. Update engine to frontier-based execution

### Phase 4: Parameter System Integration

18. Define `ParameterCollection` with all 18 parameter types
19. Implement three-phase expression pipeline (Transform → Validate → Process)
20. Implement `ResourceParameter` with universal loader, caching, dependency resolution
21. Wire parameter validation into `ProcessExecutor` (before `execute()`)
22. Auto-infer JSON schemas from Rust types via `JsonSchema` derive

### Phase 5: Resource Management

23. Implement `Resource` and `ResourceInstance` traits (from nebula-resource prototype)
24. Implement `ResourcePool` with FIFO strategy (expand to LRU/LFU later)
25. Build `ResourceManager` with registry, pooling, and health monitoring
26. Integrate `ResourceManager` into `ActionContext` (actions access resources via context)
27. Implement built-in resource types: HTTP Client, PostgreSQL, Message Queue

### Phase 6: Idempotency

28. Implement `IdempotentAction` trait and `IdempotencyStorage` trait (from nebula-idempotency prototype)
29. Implement `IdempotentExecutor<A, S>` with cache check → execute → store pattern
30. Implement `InMemoryIdempotencyStorage` with TTL support
31. Implement `WorkflowCheckpoint` and `CheckpointManager` with recovery
32. Wire idempotency into executor pipeline (composable wrapper)

### Phase 7: Ergonomics + SDK

33. Create `nebula-action-derive` crate with `#[derive(Action)]`, `#[derive(Node)]`
34. Migrate action type traits to native async traits (Rust 2024 — remove `#[async_trait]`)
35. Ship test doubles: `MockCredentialProvider`, `RecordingLogger`, `NoopMetrics`, `TestContext`
36. Implement `test_node!` macro and `MockExecution` builder
37. Marker-based resource context with `#[derive(Resources)]`

### Phase 8: Advanced Action Types (When Consumer Exists)

38. Build `SupplyAction` adapter + executor (for AI chain / resource management)
39. Build `StreamingAction` adapter + executor (for real-time data)
40. Build `InteractiveAction` adapter + executor (for approval flows)
41. Build `TransactionalAction` adapter + executor (for distributed transactions)
42. Build `EventSourcingAction` as a separate library crate (not core)

### Phase 9: Advanced Infrastructure

43. Typed context traits (`ProcessContext`, `StatefulContext`, `TriggerContext`)
44. Connect retry policy to action execution in runtime
45. Interceptor middleware on handlers for cross-cutting concerns
46. Replace shared mutable variables with explicit NodeInputs model
47. WASM sandbox integration (IsolationLevel::Isolated)

---

## Part 9: Prototype Assessment

Two prototype crates in `tmp/` validate the trait-first approach and provide starting points for implementation.

### nebula-idempotency (~55% complete)

| Component | Status | Notes |
|-----------|--------|-------|
| `IdempotentAction` trait | Complete | Clean associated types, per-action config |
| `IdempotencyStorage` trait | Complete | Pluggable get/set/remove |
| `IdempotentExecutor<A, S>` | Skeleton | Basic cache-check-execute-store logic |
| `InMemoryIdempotencyStorage` | Complete | Working MVP, no TTL |
| `WorkflowCheckpoint` | Skeleton | Data structures present, recovery missing |
| `CheckpointManager` | Skeleton | Async stubs, TODO comments |
| HTTP request idempotency | Stub | Module structure only |
| Transactional integration | Stub | Module structure only |

**Reusable:** Trait definitions, storage interface, executor pattern. **Needs work:** TTL support, recovery logic, persistence backends.

### nebula-resource (~45% complete)

| Component | Status | Notes |
|-----------|--------|-------|
| `Resource` trait | Complete | Full factory interface |
| `ResourceInstance` trait | Complete | Health check, cleanup, metrics, reset |
| `ResourceConfig` trait | Complete | Validate + tier adjustment |
| `ResourcePool` trait | Complete | try_acquire/release/stats/maintain |
| `GenericResourcePool` | Skeleton | VecDeque-based FIFO, no health checking |
| `ResourceRegistry` | Complete | DashMap-based, type erasure |
| `ResourceManager` | Skeleton | Methods exist, sub-systems are stubs |
| `HealthMonitor` | Stub | Empty struct |
| Concrete types (DB, HTTP, MQ) | Stub | Module structure only |

**Reusable:** All trait definitions, registry. **Needs work:** Pool implementation, health monitoring, concrete resource types.

---

## Risk Assessment

| Decision | Risk | Mitigation |
|---|---|---|
| Keep all 10+ action types | Large API surface | Tier strategy — only Tier 1 in v1, others behind feature flags |
| ActionOutput separation | Breaking change to ActionResult | Phase migration; keep old T-based API temporarily |
| 12+ ActionResult variants | Complex match sites | Executors resolve specialized variants; engine sees ~6 |
| Per-type handlers | Multiple handler traits to maintain | They're thin; each ~10 lines. Better than one fat trait |
| Frontier-based engine | Significant engine rewrite | Can coexist with level-based during transition |
| Derive macro | Proc macro maintenance burden | Keep it simple — only generates Action impl |
| Native async traits | Mixed async-trait/native is confusing | Clear rule: author traits = native, dyn traits = async-trait |
| 18 parameter types | Broad surface to implement | Start with 6 core types (Text, Number, Boolean, Select, Code, Secret), add rest incrementally |
| Resource pooling complexity | Over-engineering risk | Start with simple FIFO pool, add LRU/health when needed |
| Idempotency at 4 levels | Scope creep | Start with Action-level only, add Workflow checkpointing second |
| Marker-based resources | Negative trait bounds unstable | Use extension trait pattern (stable Rust) |
| Two prototype crates | Code may drift from final design | Use as reference for trait shapes, rewrite implementations |

---

## Design Principles

1. **Every abstraction must have a concrete consumer.** Don't ship a trait without at least one adapter, one executor, and one test.
2. **Type safety is our moat.** Every design decision should amplify compile-time guarantees.
3. **Rich ActionResult for developers.** Developers return expressive results; the engine handles the complexity.
4. **Executors resolve complexity.** Specialized action behaviors (Continue loops, transaction phases, interaction steps) are resolved by executors BEFORE reaching the engine.
5. **Incremental completion, not speculative expansion.** Build Tier 1 end-to-end before starting Tier 2.
6. **Leverage Rust 2024.** Native async traits, `#[non_exhaustive]`, derive macros, JsonSchema — use the language's strengths.
7. **Platform handles presentation.** Parameters declare behavioral constraints, not UI styling. The platform core owns layout, theming, expression UX.
8. **Composition over inheritance.** Idempotency wraps actions (IdempotentExecutor), resources compose into context (ResourceManager) — no deep trait hierarchies.
9. **Health-driven infrastructure.** Resources, pools, and executors monitor and report health. Degraded is better than crashed.
10. **Tier-aware defaults.** Configuration adapts to deployment tier (Personal/Enterprise/Cloud) without action author intervention.
