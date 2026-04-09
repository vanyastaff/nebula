# nebula-action v2 — Design Specification

> Consolidated spec replacing all prior plans (01–08), design docs, and superpowers action-v2-design.
> Single source of truth for the action system architecture.

**Date:** 2026-04-08
**Status:** Draft
**Crate:** `nebula-action` (`crates/action/`)
**Depends on:** nebula-core, nebula-parameter, nebula-credential, nebula-resource
**Depended by:** nebula-engine, nebula-runtime, nebula-plugin, nebula-sdk

---

## 1. Goals & Philosophy

### What nebula-action IS

- **Protocol, not runtime.** Defines contracts for executable workflow nodes. Concrete execution environments live in nebula-runtime and nebula-engine. This crate never spawns tasks, opens sockets, or touches the filesystem.
- **5 core traits.** `StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`, `AgentAction`. Each has distinct engine semantics — the engine treats them differently at scheduling time.
- **DX layer on top of core.** Convenience types (`PaginatedAction`, `WebhookAction`, `EventTrigger`, etc.) desugar into core traits via sealed marker traits (preferred) or newtype wrappers (fallback). Engine never sees DX types — only core traits.
- **Derive = boilerplate reduction.** `#[derive(Action)]` generates metadata + dependencies + handler adapter from `#[action(...)]` attributes. Developer writes the execution logic (the `execute` method) themselves.

### What nebula-action is NOT

- Not a runtime — no executor, no scheduler, no I/O
- Not a plugin system — plugin packaging/distribution is nebula-plugin's job
- Not an expression engine — expressions are nebula-expression's domain
- Not backwards-compatible at all costs — correct API > stable wrong API

### Design Principles

1. **Retry semantics are caller-side.** Actions signal retryability via `ActionError::Retryable`; engine decides retry policy. This matches Temporal's validated approach where the *workflow* (caller) configures retry policies, not the activity itself.
2. **Credentials by type, not string.** `ctx.credential::<TelegramBotKey>()` — type IS the key. No string keys, no key mismatch bugs. One type = one credential per action (duplicates are compile errors). Scoping enforced at runtime via `ScopedCredentialAccessor`.
3. **Context is composition.** `ActionContext` and `TriggerContext` are concrete structs wrapping `BaseContext` from nebula-core (identity, tenant, cancellation) and composed with capability trait objects (`ResourceAccessor`, `CredentialAccessor`, `ActionLogger`). `HasContext` trait from core is the base contract. Runtime injects real implementations; tests inject mocks/no-ops.
4. **Type safety where it matters.** Action authors work with concrete Rust types (derive, generics, `AuthScheme`). Engine runtime uses JSON type erasure via `ActionHandler` enum (5 variant-specific handler traits). The adapter layer bridges the two.
5. **ActionKey is the action name only.** Examples: `if`, `request`, `send_message`, `query`. Plugin association is via `PluginKey` — the plugin declares which actions it owns. At registration time: `PluginKey("core")` + `ActionKey("if")`. This separation allows the same action to be reused across plugins and avoids baking plugin identity into the action itself.

---

## 2. Current State & Target Changes (as of 2026-04-08)

### Implemented (5,577 LOC across 17 files)

| Module | LOC | Status | Current | Target |
|--------|-----|--------|---------|--------|
| `action.rs` | 21 | Stable | Base `Action` trait (metadata + dependencies) | No change |
| `metadata.rs` | 361 | Stable | `ActionMetadata`, `IsolationLevel`, `InterfaceVersion`, builder, compat validation | No change |
| `execution.rs` | 137 | Stable | 4 core traits: `StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction` | Add `AgentAction` (5th core trait) |
| `context.rs` | 475 | Stable | `ActionContext`, `TriggerContext`, `Context` trait | Wraps `BaseContext`, impl `HasContext`, type-based credentials via `ScopedCredentialAccessor` |
| `capability.rs` | 170 | Stable | 5 capability traits + no-op impls | No change |
| `result.rs` | 1,118 | Stable | `ActionResult` with 9 variants | 6 variants + `Routing` sub-enum (routing orthogonal to success) |
| `output.rs` | 1,215 | Stable | `ActionOutput<T>` with 8 variants, deferred/streaming/binary models | No change |
| `error.rs` | 269 | Stable | `ActionError` with 6 variants (String-based) | `Arc<anyhow::Error>` + `ErrorCode` semantic codes |
| `port.rs` | 508 | Stable | `InputPort`, `OutputPort`, `SupportPort`, `DynamicPort`, `FlowKind` | Add `OutputPort::Provide` variant |
| `handler.rs` | 222 | Stable | `InternalHandler` trait, `StatelessActionAdapter` | `ActionHandler` enum (5 variant-specific handler traits) |
| `registry.rs` | 334 | Stable | `ActionRegistry` — version-aware, `Send + Sync` | Version coexistence via `VersionedActionKey` |
| `dependency.rs` | 62 | Stable | `ActionDependencies` trait with credential/resource declarations | No change |
| `validation.rs` | 182 | Stable | `validate_action_package()` — metadata/port validation | No change |
| `authoring.rs` | 119 | Stable | `FnStatelessAction`, `stateless_fn()` — closure-based actions | No change |
| `testing.rs` | 233 | Stable | `TestContextBuilder`, `SpyLogger`, `TestCredentialAccessor` | `StatefulTestHarness`, `TriggerTestHarness`, assertion macros |
| `prelude.rs` | 31 | Stable | Convenience re-exports | No change |
| `lib.rs` | 120 | Stable | Module organization, public API surface | No change |

### Not Yet Implemented

- `AgentAction` (5th core trait) — designed, not coded
- `#[derive(Action)]` proc macro — macro crate exists (`nebula-action-macros`) but limited
- DX convenience types — `PaginatedAction`, `WebhookAction`, `EventTrigger`, etc.
- DataTag registry — 58+ tags designed, no runtime implementation
- `OutputPort::Provide` — supply-side port declarations
- `Task<T>` structured concurrency — `ctx.spawn()` helper
- `CostMetrics` on `ActionResult` — AI token/cost tracking
- Full canonical examples — only test-level examples exist

---

## 3. Type Hierarchy — 5 Core Traits + DX Layer

### 3.1 Core Traits (engine-visible)

Engine schedules and orchestrates based on these. Each has distinct execution semantics.

```
Action (base trait — metadata + dependencies)
├── StatelessAction       — pure function: input → output
├── StatefulAction        — iteration: input + state → output (Continue/Break)
├── TriggerAction         — workflow starter: start/stop lifecycle
├── ResourceAction        — graph-scoped DI: configure/cleanup
└── AgentAction           — autonomous agent: internal loop with budget
```

#### StatelessAction — 80% of all nodes

One-shot execution. Input in, output out. No persistent state between invocations.

```rust
pub trait StatelessAction: Action {
    type Input: DeserializeOwned + Send;
    type Output: Serialize + Send;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError>;
}
```

Use cases: HTTP request, data transform, send email, if/switch branching, code execution.

#### StatefulAction — iteration with engine-driven loop

Engine calls `execute` repeatedly until it returns `Break`. State persists between iterations and survives restarts (engine checkpoints it).

```rust
pub trait StatefulAction: Action {
    type Input: DeserializeOwned + Send;
    type Output: Serialize + Send;
    type State: Serialize + DeserializeOwned + Clone + Send;

    fn init_state(&self) -> Self::State;

    async fn execute(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError>;
}
```

Use cases: pagination, batch processing, polling loops, retry-with-state.

**Cancel safety contract:** If `execute` is cancelled at any `.await`, `state` retains its value from the last completed iteration. Action authors must perform state mutations atomically — update a local copy, then assign to `*state` at the end.

#### TriggerAction — workflow starters

Lives *outside* any execution — it's a long-lived workflow-scoped process that *spawns* executions. Uses `TriggerContext` (not `ActionContext`) because it has scheduling/emission capabilities instead of execution identity.

```rust
pub trait TriggerAction: Action {
    async fn start(&self, ctx: &TriggerContext) -> Result<(), ActionError>;
    async fn stop(&self, ctx: &TriggerContext) -> Result<(), ActionError>;
}
```

Use cases: webhook listener, cron schedule, event subscription, polling trigger.

#### ResourceAction — graph-level dependency injection

Runs before downstream nodes. Provides a scoped resource (e.g., database pool, API client) visible only to its downstream subtree. Cleanup runs after all downstream nodes complete.

```rust
pub trait ResourceAction: Action {
    type Config: DeserializeOwned + Send;
    type Instance: Send + Sync + 'static;

    async fn configure(
        &self,
        config: Self::Config,
        ctx: &ActionContext,
    ) -> Result<Self::Instance, ActionError>;

    async fn cleanup(
        &self,
        instance: Self::Instance,
        ctx: &ActionContext,
    ) -> Result<(), ActionError>;
}
```

Use cases: `PostgresPool` providing connections, `BrowserSession` for Playwright, `S3Client` for storage.

**ResourceAction vs `ctx.resource()`:**
- `ctx.resource()` → global resource registry (nebula-resource `Manager`)
- `ResourceAction` → graph-level DI scoped to downstream branch only

#### AgentAction — autonomous agents (NEW, not yet implemented)

Distinct from StatefulAction: agent drives its own internal loop (tool calls, reasoning steps). Engine provides budget constraints and observes from outside.

```rust
pub trait AgentAction: Action {
    type Input: DeserializeOwned + Send;
    type Output: Serialize + Send;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &AgentContext,
    ) -> Result<AgentOutcome<Self::Output>, ActionError>;
}

pub enum AgentOutcome<T> {
    /// Agent completed — final output
    Complete(T),
    /// Agent needs external input — park and resume later
    Park {
        reason: ParkReason,
        partial: Option<T>,
    },
}

pub enum ParkReason {
    /// Needs human approval
    Approval { prompt: String },
    /// Waiting for external callback
    Callback { handle: String },
    /// Budget exhausted — can be resumed with more budget
    BudgetExhausted,
}
```

**AgentContext** extends ActionContext with:

```rust
pub struct AgentContext {
    // All ActionContext capabilities plus:
    pub budget: AgentBudget,
    pub usage: AgentUsage,
    // tool invocation (tools provided via SupportPort connections)
}

pub struct AgentBudget {
    pub max_iterations: Option<u32>,
    pub max_tokens: Option<u64>,
    pub max_tool_calls: Option<u32>,
    pub max_duration: Option<Duration>,
    pub max_cost_usd: Option<f64>,
}

pub struct AgentUsage { /* fields private — only framework can increment */ }

impl AgentUsage {
    pub fn iterations(&self) -> u32 { /* ... */ }
    pub fn prompt_tokens(&self) -> u64 { /* ... */ }
    pub fn completion_tokens(&self) -> u64 { /* ... */ }
    pub fn tool_calls(&self) -> u32 { /* ... */ }
    pub fn estimated_cost_usd(&self) -> u64 { /* microdollars */ }
    pub fn elapsed(&self) -> Duration { /* ... */ }

    // Only the framework (engine/adapter) can mutate:
    pub(crate) fn add_tokens(&self, prompt: u64, completion: u64) { /* ... */ }
    pub(crate) fn increment_iterations(&self) { /* ... */ }
    pub(crate) fn increment_tool_calls(&self) { /* ... */ }
    pub(crate) fn add_cost(&self, microdollars: u64) { /* ... */ }
}
```

**Why a separate trait (not StatefulAction)?**
- StatefulAction: engine drives the loop, calls `execute` repeatedly, checkpoints state between iterations.
- AgentAction: agent drives its own internal loop. Engine doesn't see individual iterations — only the final outcome or a park request. Budget enforcement happens inside `AgentContext`, not via engine scheduling.

**Budget enforcement is hard-stop.** The engine wraps `AgentAction::execute` in a budget-aware wrapper that cancels via `CancellationToken` when any budget dimension is exceeded. The agent can check `is_budget_exceeded()` for graceful pre-emption, but the hard-stop is the safety net. When hard-stopped, the agent receives `AgentOutcome::Park { reason: ParkReason::BudgetExhausted }` with any partial output.

### 3.2 DX Layer — Convenience Types (sealed markers over core)

DX types reduce boilerplate for common patterns. They desugar into core traits via sealed marker traits (preferred) or newtype wrappers (fallback). Engine never sees DX types — only core traits flow through the handler/registry system.

The exact mechanism (sealed marker traits vs newtype wrappers) will be determined during implementation — both approaches solve the coherence conflict that arises from blanket impls across crate boundaries. Sealed markers are preferred because they avoid the indirection cost.

**Analogy:** `BufReader` is DX over `Read`. It adds ergonomics without changing what `Read` means.

```
DX over StatelessAction:
  (none needed — StatelessAction is already simple enough)

DX over StatefulAction:
├── PaginatedAction      — cursor-driven pagination loop
├── BatchAction           — process items in fixed-size chunks
├── InteractiveAction     — human approval / Wait { Approval }
└── TransactionalAction   — saga pattern: execute + compensate

DX over TriggerAction:
├── WebhookAction         — HTTP webhook lifecycle (register/handle/verify/unregister)
├── PollAction            — periodic polling with cursor persistence
├── EventTrigger          — event source subscription with auto-reconnect
└── ScheduledTrigger      — cron/interval scheduling

DX over AgentAction:
├── ReActAgent            — tool-use reasoning loop
├── PlanExecuteAgent      — plan → execute → synthesize
├── SupervisorAgent       — delegate to sub-agents
└── RouterAgent           — classify input → route to handler
```

#### PaginatedAction (DX over StatefulAction)

Eliminates the boilerplate of cursor management, progress tracking, and Continue/Break decisions.

```rust
pub trait PaginatedAction: Action {
    type Input: DeserializeOwned + Send;
    type Output: Serialize + Send;
    type Cursor: Serialize + DeserializeOwned + Clone + Send;

    fn max_pages(&self) -> u32 { 100 }

    async fn fetch_page(
        &self,
        input: &Self::Input,
        cursor: Option<&Self::Cursor>,
        ctx: &ActionContext,
    ) -> Result<PageResult<Self::Output, Self::Cursor>, ActionError>;
}

pub struct PageResult<T, C> {
    pub data: T,
    pub next_cursor: Option<C>,
}

// Blanket impl — engine sees StatefulAction
impl<A: PaginatedAction> StatefulAction for A {
    type Input = A::Input;
    type Output = A::Output;
    type State = PaginationState<A::Cursor>;

    fn init_state(&self) -> PaginationState<A::Cursor> {
        PaginationState { cursor: None, pages_fetched: 0 }
    }

    async fn execute(
        &self,
        input: Self::Input,
        state: &mut PaginationState<A::Cursor>,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let result = self.fetch_page(&input, state.cursor.as_ref(), ctx).await?;
        state.cursor = result.next_cursor.clone();
        state.pages_fetched += 1;

        if result.next_cursor.is_some() && state.pages_fetched < self.max_pages() {
            let progress = state.pages_fetched as f64 / self.max_pages() as f64;
            Ok(ActionResult::r#continue(result.data, Some(progress)))
        } else {
            Ok(ActionResult::break_completed(result.data))
        }
    }
}
```

#### BatchAction (DX over StatefulAction)

Process a collection of items in fixed-size chunks with per-item error handling.

```rust
pub trait BatchAction: Action {
    type Input: DeserializeOwned + Send;
    type Item: Serialize + DeserializeOwned + Clone + Send;
    type Output: Serialize + Send;

    fn batch_size(&self) -> usize { 50 }

    fn extract_items(&self, input: &Self::Input) -> Vec<Self::Item>;

    async fn process_item(
        &self,
        item: Self::Item,
        ctx: &ActionContext,
    ) -> Result<Self::Output, ActionError>;

    fn merge_results(&self, results: Vec<Result<Self::Output, ActionError>>) -> Self::Output;
}
```

#### WebhookAction (DX over TriggerAction)

Full webhook lifecycle with signature verification, route registration, and state persistence.

```rust
pub trait WebhookAction: Action {
    type State: Serialize + DeserializeOwned + Default + Send;

    /// Register webhook with external service. Returns state to persist.
    async fn on_activate(
        &self,
        ctx: &TriggerContext,
    ) -> Result<Self::State, ActionError>;

    /// Verify incoming request signature (HMAC, etc.)
    async fn verify_signature(
        &self,
        request: &WebhookRequest,
        state: &Self::State,
        ctx: &TriggerContext,
    ) -> Result<bool, ActionError>;

    /// Handle verified webhook payload → emit execution(s)
    async fn handle_request(
        &self,
        request: WebhookRequest,
        state: &Self::State,
        ctx: &TriggerContext,
    ) -> Result<WebhookResponse, ActionError>;

    /// Unregister webhook on deactivation
    async fn on_deactivate(
        &self,
        state: Self::State,
        ctx: &TriggerContext,
    ) -> Result<(), ActionError>;
}

pub struct WebhookRequest {
    pub method: HttpMethod,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Bytes,
    pub query: HashMap<String, String>,
}

pub struct WebhookResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Bytes,
}
```

Blanket impl bridges to `TriggerAction::start()` / `stop()` with state checkpointing via `TriggerContext`.

#### EventTrigger (DX over TriggerAction)

For event sources (SSE, WebSocket, message queues) — author writes only `next_event()`, framework handles reconnection/health/shutdown.

```rust
pub trait EventTrigger: Action {
    type Connection: Send;
    type Event: Serialize + Send;

    /// Establish connection to event source
    async fn connect(&self, ctx: &TriggerContext) -> Result<Self::Connection, ActionError>;

    /// Receive next event (blocks until available)
    async fn next_event(
        &self,
        conn: &mut Self::Connection,
        ctx: &TriggerContext,
    ) -> Result<Option<Self::Event>, ActionError>;

    /// Error handling policy
    fn on_error(&self, _error: &ActionError) -> EventErrorPolicy {
        EventErrorPolicy::Reconnect { delay: Duration::from_secs(5) }
    }
}

pub enum EventErrorPolicy {
    /// Reconnect after delay
    Reconnect { delay: Duration },
    /// Skip this event, continue listening
    Skip,
    /// Stop the trigger
    Stop,
}
```

Blanket impl to `TriggerAction`: `start()` spawns a loop calling `connect()` + `next_event()` with auto-reconnect. `stop()` cancels via `CancellationToken`.

#### PollAction (DX over TriggerAction)

Periodic polling with cursor persistence — author writes only `poll()`.

```rust
pub trait PollAction: Action {
    type Cursor: Serialize + DeserializeOwned + Default + Send;
    type Event: Serialize + Send;

    fn poll_interval(&self) -> Duration;

    async fn poll(
        &self,
        cursor: &mut Self::Cursor,
        ctx: &TriggerContext,
    ) -> Result<Vec<Self::Event>, ActionError>;
}
```

#### InteractiveAction (DX over StatefulAction)

Human-in-the-loop approval/input with declarative UI spec.

```rust
pub trait InteractiveAction: Action {
    type Input: DeserializeOwned + Send;
    type Output: Serialize + Send;
    type FormData: DeserializeOwned + Send;

    /// Build the form to present to the user
    fn approval_form(&self, input: &Self::Input) -> ApprovalForm;

    /// Process the user's response
    async fn on_response(
        &self,
        input: Self::Input,
        form_data: Self::FormData,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError>;
}

pub struct ApprovalForm {
    pub title: String,
    pub description: String,
    pub fields: Vec<FormField>,
    pub timeout: Option<Duration>,
}
```

Desugars to StatefulAction returning `ActionResult::Wait { condition: WaitCondition::Approval, .. }`.

#### TransactionalAction (DX over StatefulAction)

Saga pattern — execute with automatic compensation on downstream failure.

```rust
pub trait TransactionalAction: Action {
    type Input: DeserializeOwned + Send;
    type Output: Serialize + Send;
    type CompensationData: Serialize + DeserializeOwned + Send;

    /// Forward execution — returns output + data needed for compensation
    async fn execute_tx(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<(Self::Output, Self::CompensationData), ActionError>;

    /// Compensating action — undo the forward execution
    async fn compensate(
        &self,
        data: Self::CompensationData,
        ctx: &ActionContext,
    ) -> Result<(), ActionError>;
}
```

**Note:** Engine-level saga orchestration is post-v1. For v1, if node N succeeds but N+1 fails, action N handles its own compensation. The trait provides the structure, but engine orchestration of the full saga comes later.

#### Agent DX Types (over AgentAction)

```rust
/// ReAct: Observation → Thought → Action → repeat
pub trait ReActAgent: Action {
    type Input: DeserializeOwned + Send;
    type Output: Serialize + Send;

    fn system_prompt(&self) -> &str;
    fn available_tools(&self) -> Vec<ToolSpec>;
    fn max_iterations(&self) -> u32 { 10 }

    async fn think(
        &self,
        messages: &[Message],
        tools: &[ToolSpec],
        ctx: &AgentContext,
    ) -> Result<AgentStep, ActionError>;

    async fn execute_tool(
        &self,
        tool_call: ToolCall,
        ctx: &AgentContext,
    ) -> Result<Value, ActionError>;

    fn is_complete(&self, step: &AgentStep) -> Option<Self::Output>;
}

pub enum AgentStep {
    ToolCall(ToolCall),
    Response(Value),
}
```

---

## 4. ActionResult — Control Flow Vocabulary

`ActionResult<T>` is how actions tell the engine *what happened* and *what to do next*. The engine interprets these signals — actions don't decide scheduling.

### 4.1 Variant Table

```rust
#[non_exhaustive]
pub enum ActionResult<T> {
    /// Normal completion — routing determines downstream delivery
    Success { output: T, routing: Routing },
    /// Conditional skip — downstream nodes skipped
    Skip { reason: String, output: Option<T> },
    /// StatefulAction not done — checkpoint state, re-invoke
    Continue { output: T, progress: Option<f64>, delay: Option<Duration> },
    /// StatefulAction done — complete iteration
    Break { output: T, reason: BreakReason },
    /// External event needed — pause execution
    Wait { condition: WaitCondition, timeout: Option<Duration>, partial: Option<T> },
    /// Action requests retry — engine retries after delay
    Retry { after: Duration, reason: String },
}

/// Routing determines how Success output reaches downstream nodes.
/// Replaces the former Branch/Route/MultiOutput variants — routing is
/// orthogonal to success/failure, not a separate control flow signal.
#[non_exhaustive]
pub enum Routing {
    /// Default: output goes to all connected downstream nodes
    Default,
    /// Conditional branching: activate selected, deactivate alternatives
    Branch { selected: String, alternatives: Vec<String> },
    /// Send output to a specific named output port
    Port(PortKey),
    /// Fan-out: different outputs to different ports
    Multi(HashMap<PortKey, Value>),
}
```

| Variant | When to use | Engine behavior |
|---------|------------|----------------|
| `Success { output, routing: Default }` | Normal completion | Pass output to all downstream nodes |
| `Success { output, routing: Branch { .. } }` | Conditional routing | Activate selected branch, deactivate alternatives |
| `Success { output, routing: Port(key) }` | Port-specific output | Send output to named output port |
| `Success { output, routing: Multi(map) }` | Fan-out | Send different outputs to different ports |
| `Skip { reason, output }` | Conditional skip | Skip downstream nodes, optionally pass data |
| `Continue { output, progress, delay }` | StatefulAction not done | Checkpoint state, re-invoke after delay |
| `Break { output, reason }` | StatefulAction done | Complete iteration, pass final output |
| `Wait { condition, timeout, partial }` | External event needed | Pause execution, resume on condition |
| `Retry { after, reason }` | Action requests retry | Engine retries after specified delay |

### 4.2 Convenience Constructors

```rust
ActionResult::success(value)                    // Success with Routing::Default
ActionResult::success_empty()                   // Success with Value::Null, Routing::Default
ActionResult::skip("reason")                    // Skip without data
ActionResult::skip_with_output("reason", data)  // Skip with data
ActionResult::branch("true", data)              // Success with Routing::Branch
ActionResult::route(PortKey::from("output_1"), data) // Success with Routing::Port
ActionResult::multi(map)                        // Success with Routing::Multi
ActionResult::r#continue(data, Some(0.5))       // Continue with 50% progress
ActionResult::break_completed(data)             // Break — normal completion
ActionResult::wait_approval("Please review", timeout)  // Wait for human
ActionResult::retry_after(Duration::from_secs(30), "rate limited")
```

### 4.3 Transformations

```rust
result.map_output(|v| transform(v))              // Transform output in any variant
result.try_map_output(|v| fallible_transform(v))  // Fallible transform
result.into_primary_output()                      // Extract output value
result.into_primary_value()                       // Extract as serde_json::Value
```

---

## 5. ActionOutput — Data Payload Vocabulary

`ActionOutput<T>` describes *what form* the data takes. Orthogonal to `ActionResult` (which describes *control flow*).

### 5.1 Variant Table

| Variant | Use case | Runtime requirement |
|---------|----------|-------------------|
| `Value(T)` | Structured JSON data | None — native pass-through |
| `Binary(BinaryData)` | Files, images, audio | Downstream must handle binary or receive reference |
| `Reference(DataReference)` | External data pointer | Runtime fetches/streams on demand |
| `Deferred(DeferredOutput)` | Async result not yet ready | Runtime resolves before delivering to downstream |
| `Streaming(StreamOutput)` | Incremental chunks | Requires stream-aware downstream |
| `Collection(Vec<ActionOutput<T>>)` | Batch/fan-out | Runtime flattens or passes as array |
| `Empty` | No output | Downstream handles missing payload |

### 5.2 Deferred Resolution

When an action returns `Deferred`, it provides a resolution strategy:

```rust
pub struct DeferredOutput {
    pub handle_id: String,           // Unique handle for idempotency
    pub resolution: Resolution,       // How to resolve
    pub expected: ExpectedOutput,     // Schema hint for downstream
    pub progress: Option<Progress>,   // Completion tracking
    pub producer: Producer,           // Who produced this
    pub retry: Option<RetryConfig>,   // Retry on resolution failure
    pub timeout: Option<Duration>,    // Max wait time
}

pub enum Resolution {
    Poll(PollTarget),     // Periodically check for completion
    Await,                // Wait for async notification
    Callback(String),     // External callback URL
    SubWorkflow(String),  // Sub-workflow execution
    AwaitOrPoll {         // Try await, fall back to polling
        poll: PollTarget,
        await_timeout: Duration,
    },
}
```

**Contract:**
1. Producer returns `Deferred(handle_id, resolution)` and is done — it doesn't resolve the output.
2. Runtime owns resolution lifecycle.
3. Engine persists deferred output before scheduling downstream.
4. Idempotent by `handle_id` — duplicate deferred outputs with same handle are deduplicated.
5. On restart, engine resumes pending resolutions from persisted state.

### 5.3 Streaming

```rust
pub struct StreamOutput {
    pub stream_id: String,
    pub mode: StreamMode,
    pub state: StreamState,
    pub buffer: BufferConfig,
    pub metadata: Option<OutputMeta>,
}

pub enum StreamMode {
    Tokens,     // LLM token stream
    Bytes,      // Binary stream
    Deltas(DeltaFormat), // JSON patches (RFC 7396 or RFC 6902)
    Events,     // SSE-style events
    Custom(String),
}

pub struct BufferConfig {
    pub capacity: usize,
    pub overflow: Overflow,
}

pub enum Overflow {
    Block,       // Backpressure — producer waits
    DropOldest,  // Ring buffer
    DropNewest,  // Drop incoming
    Error,       // Fail the stream
}
```

**Backpressure contract:**
1. Bounded buffering — never unbounded growth.
2. Consumer tolerates partial delivery + terminal states.
3. Per-stream FIFO ordering.

### 5.4 Cost & Metadata Tracking

```rust
pub struct OutputMeta {
    pub origin: OutputOrigin,
    pub timing: Option<Timing>,
    pub cost: Option<Cost>,
    pub cache: CacheInfo,
}

pub struct Cost {
    pub usd_cents: Option<f64>,
    pub tokens: Option<TokenUsage>,
}

pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cached: Option<u64>,
}

pub enum OutputOrigin {
    Computed,    // Deterministic transform
    AI,          // LLM/ML model
    External,    // API call
    Cached,      // Cache hit
    Human,       // User input
    Passthrough, // Data unchanged
}
```

**CostMetrics on ActionResult (v1.1 addition):**

Optional cost field on `ActionResult` for AI actions that want to report token usage without using the full `OutputMeta` system:

```rust
pub struct CostMetrics {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub model_id: Option<String>,
    pub estimated_cost_usd: Option<f64>,
}
```

Engine aggregates per-execution. No pricing registry in core — pricing is a plugin/UI concern.

---

## 6. ActionError — Failure Vocabulary

### 6.1 Variants

```rust
/// Semantic error codes for smarter engine retry/recovery decisions.
/// Engine can use these for targeted behavior (e.g., `RateLimited` → respect
/// `Retry-After` header, `AuthExpired` → refresh credential before retry).
#[non_exhaustive]
pub enum ErrorCode {
    RateLimited,
    Conflict,
    AuthExpired,
    UpstreamUnavailable,
    UpstreamTimeout,
    InvalidInput,
    QuotaExhausted,
    ActionPanicked,
}

#[non_exhaustive]
pub enum ActionError {
    /// Transient failure — engine may retry
    Retryable {
        error: Arc<anyhow::Error>,
        code: Option<ErrorCode>,
        backoff_hint: Option<Duration>,
        partial_output: Option<Value>,
    },
    /// Permanent failure — do not retry
    Fatal {
        error: Arc<anyhow::Error>,
        code: Option<ErrorCode>,
        details: Option<Value>,
    },
    /// Input validation failed
    Validation(String),
    /// Action tried to access undeclared capability
    SandboxViolation {
        capability: String,
        action_id: String,
    },
    /// Execution was cancelled
    Cancelled,
    /// Output exceeded size limit
    DataLimitExceeded {
        limit_bytes: u64,
        actual_bytes: u64,
    },
}
```

`Arc` wrapper on `anyhow::Error` enables `Clone` while preserving the full error chain. `ErrorCode` provides machine-readable classification; `Display` provides human-readable messages.

No `Custom(String)` variant on `ErrorCode` — stringly-typed codes defeat the purpose. If a new semantic code is needed, add a variant and bump the minor version. `#[non_exhaustive]` makes this non-breaking.

### 6.2 Factory Methods

```rust
ActionError::retryable(error)                     // Retryable, no hint
ActionError::retryable_with_backoff(error, dur)   // Retryable, suggest delay
ActionError::retryable_with_partial(error, data)  // Retryable, with partial output
ActionError::fatal(error)                         // Fatal
ActionError::fatal_with_details(error, details)   // Fatal with JSON details
ActionError::validation("field X is required")    // Validation
```

### 6.3 Error Classification Integration

`ActionError` implements `nebula_error::Classify`:
- `Retryable` → `ErrorClass::Transient`
- `Fatal` → `ErrorClass::Permanent`
- `Validation` → `ErrorClass::Permanent`
- `SandboxViolation` → `ErrorClass::Permanent`
- `Cancelled` → `ErrorClass::Cancelled`
- `DataLimitExceeded` → `ErrorClass::Permanent`

### 6.4 ActionResultExt

Fluent error conversion for action authors:

```rust
pub trait ActionResultExt<T> {
    fn retryable(self) -> Result<T, ActionError>;
    fn fatal(self) -> Result<T, ActionError>;
    fn retryable_with_code(self, code: ErrorCode) -> Result<T, ActionError>;
    fn fatal_with_code(self, code: ErrorCode) -> Result<T, ActionError>;
}

impl<T, E: Into<anyhow::Error>> ActionResultExt<T> for Result<T, E> {
    fn retryable(self) -> Result<T, ActionError> {
        self.map_err(|e| ActionError::Retryable {
            error: Arc::new(e.into()),
            code: None,
            backoff_hint: None,
            partial_output: None,
        })
    }

    fn fatal(self) -> Result<T, ActionError> {
        self.map_err(|e| ActionError::Fatal {
            error: Arc::new(e.into()),
            code: None,
            details: None,
        })
    }

    fn retryable_with_code(self, code: ErrorCode) -> Result<T, ActionError> {
        self.map_err(|e| ActionError::Retryable {
            error: Arc::new(e.into()),
            code: Some(code),
            backoff_hint: None,
            partial_output: None,
        })
    }

    fn fatal_with_code(self, code: ErrorCode) -> Result<T, ActionError> {
        self.map_err(|e| ActionError::Fatal {
            error: Arc::new(e.into()),
            code: Some(code),
            details: None,
        })
    }
}

// Usage:
let response = client.get(url).await.retryable()?;
let data = response.json::<MyData>().await.fatal()?;
let resp = client.post(url).await.retryable_with_code(ErrorCode::UpstreamUnavailable)?;
```

---

## 7. Context & Capability Model

### 7.1 ActionContext

Injected into `StatelessAction`, `StatefulAction`, `ResourceAction`.

`ActionContext` wraps `BaseContext` from `nebula-core`, which provides identity and cancellation. Capability trait objects are composed alongside it.

```rust
/// From nebula-core — shared identity for all context types.
/// All identity fields return Option — not all contexts have all identifiers
/// (e.g., TriggerContext has no execution_id or node_id).
pub trait HasContext {
    fn execution_id(&self) -> Option<&ExecutionId>;
    fn workflow_id(&self) -> Option<&WorkflowId>;
    fn node_id(&self) -> Option<&NodeId>;
    fn user_id(&self) -> Option<&UserId>;
    fn tenant_id(&self) -> Option<&TenantId>;
    fn cancellation(&self) -> &CancellationToken;
}

pub struct ActionContext {
    base: BaseContext,  // from nebula-core: identity, tenant, cancellation
    resources: Arc<dyn ResourceAccessor>,
    credentials: Arc<dyn CredentialAccessor>,
    logger: Arc<dyn ActionLogger>,
}

impl HasContext for ActionContext {
    fn execution_id(&self) -> Option<&ExecutionId> { self.base.execution_id() }
    fn workflow_id(&self) -> Option<&WorkflowId> { self.base.workflow_id() }
    fn node_id(&self) -> Option<&NodeId> { self.base.node_id() }
    fn user_id(&self) -> Option<&UserId> { self.base.user_id() }
    fn tenant_id(&self) -> Option<&TenantId> { self.base.tenant_id() }
    fn cancellation(&self) -> &CancellationToken { self.base.cancellation() }
}

impl ActionContext {
    // Typed credential access — type IS the key
    pub fn credential<S: AuthScheme>(&self) -> Result<CredentialGuard<S>, ActionError>;
    pub fn credential_opt<S: AuthScheme>(&self) -> Result<Option<CredentialGuard<S>>, ActionError>;

    // Resource access
    pub fn resource(&self, key: &str) -> Result<Box<dyn Any + Send>, ActionError>;
    pub fn has_resource(&self, key: &str) -> bool;

    // Logging
    pub fn log(&self, level: ActionLogLevel, message: &str);
}
```

`CredentialGuard<S>` is returned from credential access — see section 6.6.

### 7.2 TriggerContext

Injected into `TriggerAction`. Distinct from `ActionContext` because triggers live outside executions — they're long-lived, workflow-scoped processes.

```rust
pub struct TriggerContext {
    base: BaseContext,  // from nebula-core (no execution_id for triggers)
    trigger_id: String,
    scheduler: Arc<dyn TriggerScheduler>,
    emitter: Arc<dyn ExecutionEmitter>,
    credentials: Arc<dyn CredentialAccessor>,
    logger: Arc<dyn ActionLogger>,
}

// TriggerContext: returns None for execution_id and node_id
// (triggers live outside executions and are not graph nodes)
impl HasContext for TriggerContext {
    fn execution_id(&self) -> Option<&ExecutionId> { None }
    fn workflow_id(&self) -> Option<&WorkflowId> { self.base.workflow_id() }
    fn node_id(&self) -> Option<&NodeId> { None }
    fn user_id(&self) -> Option<&UserId> { self.base.user_id() }
    fn tenant_id(&self) -> Option<&TenantId> { self.base.tenant_id() }
    fn cancellation(&self) -> &CancellationToken { self.base.cancellation() }
}

impl TriggerContext {
    // Scheduling
    pub async fn schedule_after(&self, delay: Duration) -> Result<(), ActionError>;

    // Execution emission
    pub async fn emit_execution(&self, input: Value) -> Result<(), ActionError>;

    // Typed credentials (same API as ActionContext — type IS the key)
    pub fn credential<S: AuthScheme>(&self) -> Result<CredentialGuard<S>, ActionError>;
    pub fn credential_opt<S: AuthScheme>(&self) -> Result<Option<CredentialGuard<S>>, ActionError>;
}
```

### 7.3 AgentContext (new)

`AgentContext` uses `Deref<Target = ActionContext>` instead of manual delegation — all `ActionContext` methods (credentials, resources, logging, identity) are available transparently.

```rust
pub struct AgentContext {
    action_ctx: ActionContext,
    budget: AgentBudget,
    usage: AgentUsage,
    tools: Vec<ToolSpec>,
}

impl Deref for AgentContext {
    type Target = ActionContext;
    fn deref(&self) -> &ActionContext { &self.action_ctx }
}

impl AgentContext {
    // Agent-specific methods only — ActionContext methods available via Deref
    pub fn budget(&self) -> &AgentBudget;
    pub fn usage(&self) -> &AgentUsage;
    pub fn is_budget_exceeded(&self) -> bool;
    pub fn available_tools(&self) -> &[ToolSpec];
}

// HasContext is available via Deref<Target=ActionContext>, but AgentContext
// also implements HasContext explicitly for clarity and trait-object compatibility:
impl HasContext for AgentContext {
    fn execution_id(&self) -> Option<&ExecutionId> { self.action_ctx.execution_id() }
    fn workflow_id(&self) -> Option<&WorkflowId> { self.action_ctx.workflow_id() }
    fn node_id(&self) -> Option<&NodeId> { self.action_ctx.node_id() }
    fn user_id(&self) -> Option<&UserId> { self.action_ctx.user_id() }
    fn tenant_id(&self) -> Option<&TenantId> { self.action_ctx.tenant_id() }
    fn cancellation(&self) -> &CancellationToken { self.action_ctx.cancellation() }
}
```

### 7.4 Capability Traits

All capability traits are object-safe (`dyn`-compatible) with no-op defaults for graceful degradation.

```rust
pub trait ResourceAccessor: Send + Sync {
    fn acquire(&self, key: &str) -> Result<Box<dyn Any + Send>, ActionError>;
    fn exists(&self, key: &str) -> bool;
}

pub trait CredentialAccessor: Send + Sync {
    fn get(&self, id: &CredentialId) -> Result<CredentialSnapshot, ActionError>;
    fn get_by_type(&self, type_id: TypeId) -> Result<CredentialSnapshot, ActionError>;
    fn has(&self, id: &CredentialId) -> bool;
}

pub trait ActionLogger: Send + Sync {
    fn log(&self, level: ActionLogLevel, message: &str);
}

pub trait TriggerScheduler: Send + Sync {
    fn schedule_after(&self, delay: Duration) -> Result<(), ActionError>;
}

pub trait ExecutionEmitter: Send + Sync {
    fn emit(&self, input: Value) -> Result<(), ActionError>;
}
```

No-op implementations provided for each: `NoopResourceAccessor`, `NoopCredentialAccessor`, `NoopActionLogger`, `NoopTriggerScheduler`, `NoopExecutionEmitter`.

### 7.5 CredentialGuard

`CredentialGuard<S>` wraps a credential value with security guarantees:

```rust
/// S must implement Zeroize — AuthScheme implementations must derive/impl Zeroize
/// to ensure credential data is wiped from memory on drop.
pub struct CredentialGuard<S: AuthScheme + Zeroize> {
    inner: S,
}

impl<S: AuthScheme + Zeroize> Deref for CredentialGuard<S> {
    type Target = S;
    fn deref(&self) -> &S { &self.inner }
}

impl<S: AuthScheme + Zeroize> Drop for CredentialGuard<S> {
    fn drop(&mut self) {
        self.inner.zeroize(); // Wipe plaintext from memory
    }
}

// CredentialGuard does NOT implement Serialize — prevents accidental
// inclusion in action output or state. If the action tries to serialize
// a CredentialGuard, it gets a compile error.
```

Similar to the existing resource guard pattern. The guard ensures:
1. **Zeroize on drop** — plaintext wiped from memory when guard is dropped
2. **No serialization** — `!Serialize` prevents accidental leakage into output/state
3. **Transparent access** — `Deref<Target = S>` for ergonomic field access

### 7.6 ScopedCredentialAccessor

Engine wraps the real `CredentialAccessor` with `ScopedCredentialAccessor` that enforces declared credential types from `ActionDependencies`.

**Mapping flow:**
1. `ActionDependencies` declares credential types: e.g., `[TelegramBotKey, HmacSecret]`
2. At registration time, the engine maps `TypeId` to `CredentialId` (from the credential system)
3. `ScopedCredentialAccessor` holds: `mapping: HashMap<TypeId, CredentialId>` + `inner: Arc<dyn CredentialAccessor>`
4. `credential::<S>()` looks up `TypeId::of::<S>()` in the mapping, gets the `CredentialId`, then calls `inner.get(id)`

```rust
pub struct ScopedCredentialAccessor {
    inner: Arc<dyn CredentialAccessor>,
    allowed_types: HashSet<TypeId>,  // From ActionDependencies declaration
    mapping: HashMap<TypeId, CredentialId>,  // TypeId → CredentialId resolved at registration
}

impl ScopedCredentialAccessor {
    /// Only allows access to credential types declared in #[action(credential = ...)]
    pub fn credential<S: AuthScheme + 'static>(&self) -> Result<CredentialGuard<S>, ActionError> {
        if !self.allowed_types.contains(&TypeId::of::<S>()) {
            return Err(ActionError::SandboxViolation {
                capability: format!("credential type {}", std::any::type_name::<S>()),
                action_id: /* from context */,
            });
        }
        // Look up CredentialId from TypeId mapping, then delegate to inner
        let cred_id = self.mapping.get(&TypeId::of::<S>())
            .ok_or_else(|| ActionError::fatal(
                anyhow::anyhow!("no credential mapping for {}", std::any::type_name::<S>()),
            ))?;
        let snapshot = self.inner.get(cred_id)?;
        // Project snapshot into typed S via AuthScheme::from_snapshot
        // ...
    }
}
```

This ensures actions cannot access credentials they didn't declare — the `TypeId` check matches against the types listed in `ActionDependencies`.

---

## 8. Derive Macro — `#[derive(Action)]`

### 8.1 What It Generates

`#[derive(Action)]` reads `#[action(...)]` attributes and generates:
1. `Action` trait impl (returns `ActionMetadata` built from attributes)
2. `ActionDependencies` trait impl (credential/resource keys from attributes)
3. Handler adapter registration helper

`#[derive(Action)]` does NOT generate:
- Execution logic — developer writes `impl StatelessAction` etc.
- Parameter parsing — developer derives `Parameters` separately or implements `HasParameters`

### 8.2 Attribute Syntax

```rust
// Action identity
#[action(key = "request")]                   // Required: action name (plugin binds separately)
#[action(name = "HTTP Request")]             // Required: display name
#[action(description = "...")]               // Optional: description
#[action(version = "1.0")]                   // Optional: interface version

// Credential declarations — by TYPE, not string
#[action(credential = TelegramBotKey)]                    // Single required credential type
#[action(credential(optional) = HmacSecret)]              // Single optional credential type
#[action(credentials = [TelegramBotKey, HmacSecret])]     // Multiple credential types

// Resource declarations — by TYPE
#[action(resource = HttpResource)]           // Single resource type
#[action(resources = [HttpResource, DbPool])]// Multiple resource types

// Isolation
#[action(isolation = "capability_gated")]    // IsolationLevel
```

Duplicate credential or resource types are **compile errors** — the derive macro checks for uniqueness. This catches copy-paste mistakes at build time.

### 8.3 Combined with Parameters

```rust
#[derive(Action, Parameters, Deserialize)]
#[action(key = "request", name = "HTTP Request")]
#[action(credential = BearerSecret)]
struct HttpRequest {
    #[param(label = "URL", hint = "url")]
    #[validate(required, url)]
    url: String,

    #[param(default = "GET")]
    method: HttpMethod,

    #[param(label = "Headers")]
    headers: Option<Vec<Header>>,

    #[param(label = "Timeout (s)")]
    #[validate(range(1..=300))]
    timeout: Option<u32>,
}

// Plugin registration binds action to plugin:
// PluginKey("http") + ActionKey("request") → fully qualified "http.request"

// Developer writes execution:
impl StatelessAction for HttpRequest {
    type Input = Self;      // struct IS the input
    type Output = Value;

    async fn execute(&self, _input: Self, ctx: &ActionContext)
        -> Result<ActionResult<Value>, ActionError>
    {
        let cred = ctx.credential::<BearerSecret>()?;
        // ... HTTP call using self.url, self.method, etc.
        Ok(ActionResult::success(json!({ "status": 200, "body": body })))
    }
}
```

### 8.4 Semver Contract

`#[derive(Action)]` output stability is governed by semver on:
- `Action` trait
- `ActionMetadata` struct
- `ActionDependencies` trait

Breaking changes to these types = major version bump.

Macro internals (code generation patterns) are NOT public API — only the generated trait impls are. The macro may change how it generates code as long as the resulting trait impls are equivalent.

### 8.5 Manual Registration (no proc macros)

For users who prefer no proc macros (e.g., Bevy-style):

```rust
let metadata = ActionMetadata::builder("request", "HTTP Request")
    .with_version(InterfaceVersion::new(1, 0))
    .with_parameters(ParameterCollection::builder()
        .string("url", |s| s.label("URL").required())
        .select("method", |s| s.label("Method").default("GET"))
        .build())
    .build();

registry.register_handler(
    metadata.key.clone(),
    metadata.version,
    Arc::new(FnHandler::new(metadata, |input: Value, ctx: &ActionContext| async move {
        Ok(ActionResult::success(input))
    })),
);
```

---

## 9. Handler Layer — Type Erasure

### 9.1 ActionHandler Enum

Engine stores actions as `ActionHandler` — an enum with 5 variant-specific handler traits instead of a single polymorphic trait. This gives the engine type-level knowledge of *which kind* of action it's dealing with, enabling variant-specific scheduling.

```rust
pub enum ActionHandler {
    Stateless(Arc<dyn StatelessHandler>),
    Stateful(Arc<dyn StatefulHandler>),
    Trigger(Arc<dyn TriggerHandler>),
    Resource(Arc<dyn ResourceHandler>),
    Agent(Arc<dyn AgentHandler>),
}

pub trait StatelessHandler: Send + Sync {
    fn metadata(&self) -> &ActionMetadata;
    async fn execute(&self, input: Value, ctx: &ActionContext)
        -> Result<ActionResult<Value>, ActionError>;
}

pub trait StatefulHandler: Send + Sync {
    fn metadata(&self) -> &ActionMetadata;
    fn init_state(&self) -> Value;
    async fn execute(&self, input: Value, state: &mut Value, ctx: &ActionContext)
        -> Result<ActionResult<Value>, ActionError>;
}

pub trait TriggerHandler: Send + Sync {
    fn metadata(&self) -> &ActionMetadata;
    async fn start(&self, ctx: &TriggerContext) -> Result<(), ActionError>;
    async fn stop(&self, ctx: &TriggerContext) -> Result<(), ActionError>;
}

pub trait ResourceHandler: Send + Sync {
    fn metadata(&self) -> &ActionMetadata;
    async fn configure(&self, config: Value, ctx: &ActionContext)
        -> Result<Box<dyn Any + Send + Sync>, ActionError>;
    async fn cleanup(&self, instance: Box<dyn Any + Send + Sync>, ctx: &ActionContext)
        -> Result<(), ActionError>;
}

pub trait AgentHandler: Send + Sync {
    fn metadata(&self) -> &ActionMetadata;
    async fn execute(&self, input: Value, ctx: &AgentContext)
        -> Result<AgentOutcome<Value>, ActionError>;
}
```

### 9.2 Adapters

Each core trait has an adapter that bridges typed action → variant-specific handler:

```rust
pub struct StatelessActionAdapter<A> {
    action: A,
}

impl<A> StatelessHandler for StatelessActionAdapter<A>
where
    A: StatelessAction,
    A::Input: DeserializeOwned,
    A::Output: Serialize,
{
    async fn execute(&self, input: Value, ctx: &ActionContext)
        -> Result<ActionResult<Value>, ActionError>
    {
        // 1. Deserialize JSON → typed input
        let typed_input: A::Input = serde_json::from_value(input)
            .map_err(|e| ActionError::validation(e.to_string()))?;

        // 2. Call typed execute
        let result = self.action.execute(typed_input, ctx).await?;

        // 3. Map output T → Value
        result.try_map_output(|output| {
            serde_json::to_value(output)
                .map_err(|e| ActionError::fatal(e))
        })
    }
}
```

Similarly: `StatefulActionAdapter` → `StatefulHandler`, `TriggerActionAdapter` → `TriggerHandler`, `ResourceActionAdapter` → `ResourceHandler`, `AgentActionAdapter` → `AgentHandler`.

### 9.3 Deserialization Safety

Depth and size limits are enforced at the **API/webhook ingress boundary** (where raw bytes are first parsed into `serde_json::Value`), not at the adapter layer. By the time input reaches an adapter, it is already a parsed `Value` — re-applying depth limits on an in-memory tree is meaningless.

The adapter's only validation responsibility is **schema conformance**: does the `Value` deserialize into `A::Input`? Type mismatches, missing fields, and constraint violations are caught by serde + `#[validate]` attributes.

---

## 10. ActionRegistry — Version-Aware Lookup

```rust
pub struct ActionRegistry {
    handlers: HashMap<ActionKey, Vec<ActionEntry>>,
}

struct ActionEntry {
    version: InterfaceVersion,
    handler: ActionHandler,
}

impl ActionRegistry {
    pub fn new() -> Self;

    /// Register an action handler
    pub fn register<A>(&mut self, action: A) -> Result<(), RegistryError>
    where
        A: Action + StatelessAction + Send + Sync + 'static;

    /// Get handler for latest version
    pub fn get(&self, key: &ActionKey) -> Option<&ActionHandler>;

    /// Get handler for specific version
    pub fn get_versioned(
        &self,
        key: &ActionKey,
        version: &InterfaceVersion,
    ) -> Option<&ActionHandler>;

    /// List all registered action keys
    pub fn keys(&self) -> Vec<&ActionKey>;

    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}
```

**Version pinning on NodeDefinition:**

`NodeDefinition` in the workflow graph gains `action_version: Option<InterfaceVersion>`. If set, engine uses `get_versioned()` — never `get_latest()` during execution. `get_latest()` is editor-only for workflow construction UI. This prevents silent action upgrades on running workflows.

**Thread safety:** `ActionRegistry` is `Send + Sync`. Use `Arc<ActionRegistry>` for read-only sharing (typical), `Arc<RwLock<ActionRegistry>>` for mutation after sharing (hot reload).

---

## 11. Port System — Graph Topology Contracts

### 11.1 Input Ports

```rust
pub enum InputPort {
    /// Main data flow
    Flow { key: PortKey },
    /// Sub-node supply (tools, models, memory for agents)
    Support(SupportPort),
}

pub struct SupportPort {
    pub key: PortKey,
    pub name: String,
    pub description: String,
    pub required: bool,
    pub multi: bool,      // Accept multiple connections
    pub filter: Option<ConnectionFilter>,
}
```

### 11.2 Output Ports

```rust
pub enum OutputPort {
    /// Data or error flow
    Flow { key: PortKey, kind: FlowKind },
    /// Config-driven dynamic outputs (e.g., switch branches)
    Dynamic(DynamicPort),
}

pub enum FlowKind {
    Main,
    Error,
}

pub struct DynamicPort {
    pub key: PortKey,
    pub source_field: String,   // Parameter field that generates port names
    pub label_field: Option<String>,
    pub include_fallback: bool, // Add "other" port for unmatched
}
```

### 11.3 OutputPort::Provide (planned)

Supply-side capability declaration — symmetric to `InputPort::Support`:

```rust
pub enum OutputPort {
    Flow { key: PortKey, kind: FlowKind },
    Dynamic(DynamicPort),
    /// NEW: This action provides a capability to support ports
    Provide(ProvidePort),
}

pub struct ProvidePort {
    pub key: PortKey,
    pub kind: ProvideKind,
    pub name: String,
    pub description: String,
    pub tags: Vec<DataTag>,
}

pub enum ProvideKind {
    /// Provides data (e.g., system prompt, context documents)
    Data,
    /// Provides a callable tool
    Tool,
    /// Provides a resource instance
    Resource,
}
```

### 11.4 ConnectionFilter

```rust
pub struct ConnectionFilter {
    pub allowed_node_types: Option<Vec<String>>,
    pub allowed_tags: Option<Vec<DataTag>>,
}
```

### 11.5 Tool Provision via Port

Actions that provide tools to agents declare them via `ProvidePort`:

```rust
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,  // JSON Schema
    pub hints: ToolHints,
}

pub struct ToolHints {
    pub idempotent: bool,
    pub read_only: bool,
    pub estimated_latency: Option<Duration>,
}
```

The providing action connects its `Provide(Tool)` port to the agent's `Support` port. Engine collects all connected tools and injects them into `AgentContext.tools`.

---

## 12. DataTag Registry

> **Status: Designed, not prioritized.** The tag system below is fully designed but deferred from the immediate implementation roadmap. It will be implemented when port compatibility checking becomes a priority (likely post engine v2). The design is retained here for reference.

Semantic type tags for port compatibility checking. Tags form a hierarchy where compatibility flows upward — `json` is the universal acceptor.

### 12.1 Core Tags (nebula-core)

| Tag | Description |
|-----|-------------|
| `json` | Universal — any JSON value |
| `text` | String content |
| `number` | Numeric value |
| `boolean` | True/false |
| `array` | JSON array |
| `object` | JSON object |
| `binary` | Raw bytes |
| `file` | File with path/name/mime |
| `stream` | Streaming data |

### 12.2 Domain Tags

**Media (nebula-media):** `image`, `image.svg`, `audio`, `video`, `pdf`, `spreadsheet`, `document`, `archive`, `font`

**AI/ML (nebula-ai):**
- Models: `ai.model`, `ai.model.llm`, `ai.model.vision`, `ai.model.tts`, `ai.model.stt`, `ai.model.embedding`
- Data: `ai.messages`, `ai.embedding`, `ai.prompt`, `ai.tool_calls`, `ai.completion`
- Diffusion: `ai.latent`, `ai.conditioning`, `ai.clip`, `ai.vae`, `ai.controlnet`, `ai.lora`, `ai.mask`

**Data (nebula-data):** `data.rows`, `data.row`, `data.cursor`, `data.schema`, `data.connection`

**Communication (nebula-comm):** `email`, `email.address`, `html`, `markdown`, `xml`, `url`, `datetime`, `cron`

### 12.3 Integration-Specific Tags

Format: `service.concept` — e.g., `slack.message`, `github.event`, `github.pr`, `discord.embed`.

### 12.4 Compatibility Rules

- Child is compatible with parent: `image.svg` connects to `image` port
- `json` accepts everything
- `text` accepts `html`, `markdown`, `xml`, `email`, `url`
- `binary` accepts all media types

### 12.5 Naming Convention

| Scope | Pattern | Examples |
|-------|---------|---------|
| Core | single word | `json`, `text`, `binary` |
| Domain | `domain.concept` | `ai.model`, `data.rows` |
| Integration | `service.concept` | `slack.message`, `github.pr` |

---

## 13. Serialization Strategy (action-relevant parts)

### 13.1 Action I/O Boundary

**Actions produce `serde_json::Value`.** This is the stable public API and will never change.

Internally, engine wraps output in `NodeOutput(Arc<RawValue>)` for zero-copy fan-out:

```rust
pub struct NodeOutput {
    pub raw: Arc<RawValue>,         // Shared across downstream consumers
    parsed: OnceLock<Value>,         // Lazy parse on first access
}
```

**Impact:** 3-node chain on 10KB payload: current = 3 full parses + 2 deep clones. With `Arc<RawValue>` = 1 serialize + 0 parses (pass-through) or 1 partial parse.

Actions never see `RawValue` — conversion happens in the adapter layer.

### 13.2 Binary Data

`BinaryData` uses `bytes::Bytes` internally for zero-copy clone on fan-out:

```rust
pub struct BinaryPayload {
    pub data: Bytes,          // Arc-backed, clone = refcount bump
    pub content_type: String,
    pub size: u64,
}
```

Action authors can still use `Vec<u8>` — conversion to `Bytes` happens in the adapter.

### 13.3 Execution State Persistence

StatefulAction state is checkpointed via `rmp-serde` (MessagePack) to `BYTEA` column in Postgres — 30-50% smaller than JSON, 2x faster serialize/deserialize. Legacy JSON format supported for migration.

### 13.4 Duration Serialization

All `Duration` fields in `ActionResult` and `ActionOutput` serialize as milliseconds (`u64`) for JSON compatibility. Custom serde modules: `duration_ms`, `duration_opt_ms`.

---

## 14. Testing Infrastructure

### 14.1 TestContextBuilder

```rust
pub struct TestContextBuilder {
    credentials: HashMap<String, Box<dyn Any + Send + Sync>>,
    resources: HashMap<String, Box<dyn Any + Send + Sync>>,
    input_data: Value,
    logger: SpyLogger,
}

impl TestContextBuilder {
    pub fn new() -> Self;
    pub fn with_credential<S: AuthScheme + 'static>(self, scheme: S) -> Self;
    pub fn with_credential_snapshot(self, id: CredentialId, snapshot: CredentialSnapshot) -> Self;
    pub fn with_resource<R: Any + Send + Sync>(self, key: &str, resource: R) -> Self;
    pub fn with_input(self, data: Value) -> Self;
    pub fn spy_logger(&self) -> SpyLogger;
    pub fn build(self) -> ActionContext;
}
```

### 14.2 SpyLogger

```rust
pub struct SpyLogger { /* captures log entries */ }

impl SpyLogger {
    pub fn new() -> Self;
    pub fn entries(&self) -> Vec<(ActionLogLevel, String)>;
    pub fn messages(&self) -> Vec<String>;
    pub fn contains(&self, substring: &str) -> bool;
    pub fn count(&self) -> usize;
}
```

### 14.3 Assertion Macros

```rust
assert_success!(result);                    // ActionResult::Success
assert_success!(result, expected_value);    // Success with specific output
assert_branch!(result, "true");             // Branch to specific key
assert_continue!(result);                   // Continue (StatefulAction)
assert_break!(result);                      // Break (StatefulAction)
assert_skip!(result);                       // Skip
assert_retryable!(result);                  // ActionError::Retryable
assert_fatal!(result);                      // ActionError::Fatal
assert_validation_error!(result);           // ActionError::Validation
```

### 14.4 StatefulTestHarness (planned)

```rust
pub struct StatefulTestHarness<A: StatefulAction> {
    action: A,
    state: A::State,
    ctx: ActionContext,
    iterations: u32,
}

impl<A: StatefulAction> StatefulTestHarness<A> {
    pub fn new(action: A, ctx: ActionContext) -> Self;

    /// Run one iteration, return result + updated state
    pub async fn step(&mut self, input: A::Input)
        -> Result<ActionResult<A::Output>, ActionError>;

    /// Run until Break or max iterations
    pub async fn run_to_completion(&mut self, input: A::Input, max: u32)
        -> Result<Vec<ActionResult<A::Output>>, ActionError>;

    pub fn state(&self) -> &A::State;
    pub fn iterations(&self) -> u32;
}
```

### 14.5 TriggerTestHarness (planned)

```rust
pub struct TriggerTestHarness<A: TriggerAction> {
    action: A,
    ctx: TriggerContext,
    emitted: Vec<Value>,
    scheduled: Vec<Duration>,
}

impl<A: TriggerAction> TriggerTestHarness<A> {
    pub fn new(action: A, ctx: TriggerContext) -> Self;
    pub async fn start(&mut self) -> Result<(), ActionError>;
    pub async fn stop(&mut self) -> Result<(), ActionError>;
    pub fn emitted_executions(&self) -> &[Value];
    pub fn scheduled_delays(&self) -> &[Duration];
}
```

---

## 15. Competitive Analysis & Design Validation

Research of n8n, Temporal, Prefect, Activepieces, and Windmill validates and informs several design decisions.

### 15.1 Where Nebula Leads

| Area | Nebula | Best Competitor | Advantage |
|------|--------|----------------|-----------|
| Error classification | 6 variants with partial output, backoff hints | Temporal (3 variants) | Most granular — engine makes smarter retry decisions |
| Credential type safety | `ctx.credential::<S>()` — type IS the key | Activepieces (runtime typed) | Compile-time guarantee, no string keys, scoped enforcement |
| Execution model diversity | 5 core traits with distinct semantics | n8n (execute/trigger/poll) | `StatefulAction`, `ResourceAction`, `AgentAction` are novel |
| Control flow vocabulary | 6 ActionResult variants + Routing sub-enum | n8n (success/error) | Routing orthogonal to success, Wait, Continue/Break — rich intent |
| Output taxonomy | 8 ActionOutput variants with deferred/streaming | None comparable | First-class async outputs, backpressure, cost tracking |

### 15.2 Where Competitors Lead (and What to Adopt)

#### Minimal boilerplate (Temporal, Prefect, Windmill)

**Problem:** Current Nebula requires 4 steps: define struct, impl `ActionDependencies`, impl `Action`, impl `StatelessAction` with associated types.
- Temporal: one closure registered by name
- Prefect: one `@task` decorator
- Windmill: one `main()` function

**Solution:** `#[derive(Action)]` + `stateless_fn()` + combined `#[derive(Action, Parameters)]` reduces to 1-2 steps. The proc-macro approach is the idiomatic Rust way to match closure/decorator ergonomics while maintaining type safety.

#### Dynamic properties (Activepieces, n8n)

**Problem:** UI-driven workflow builders need conditional field visibility and dynamic dropdowns (e.g., "select a Slack channel" loads options from API).
**Activepieces solution:** `Property.Dropdown({ refreshers: ['auth'], options: async ({ auth }) => { ... } })`

**Nebula solution (planned):** Extend `ParameterDefinition` with:

```rust
pub struct ParameterDefinition {
    // ... existing fields ...
    pub visibility: Option<VisibilityCondition>,
    pub options_loader: Option<OptionsLoader>,
}

pub enum VisibilityCondition {
    /// Show when another field equals a value
    FieldEquals { field: String, value: Value },
    /// Show when another field is non-empty
    FieldPresent(String),
    /// Custom expression
    Expression(String),
}

pub enum OptionsLoader {
    /// Static options
    Static(Vec<SelectOption>),
    /// Load from action execution (passes current form values)
    Dynamic {
        /// Which other fields trigger a reload
        refreshers: Vec<String>,
    },
}
```

This is a nebula-parameter concern but affects how `#[derive(Parameters)]` generates metadata.

#### Test ergonomics (Prefect)

**Problem:** Prefect tasks are plain Python functions — you just call them in tests. Nebula requires building a `TestContextBuilder`.

**Nebula improvement:** `stateless_fn()` + `TestContextBuilder::minimal()` for zero-ceremony testing:

```rust
#[test]
async fn test_simple() {
    let action = stateless_fn(
        ActionMetadata::new("echo", "Echo"),
        |input: Value| async move { Ok(ActionResult::success(input)) },
    );
    let ctx = TestContextBuilder::minimal().build();
    let result = action.execute(json!({"hello": "world"}), &ctx).await;
    assert_success!(result, json!({"hello": "world"}));
}
```

### 15.3 Validated Design Decisions

| Decision | Validation source |
|----------|------------------|
| Retry config on caller side (not action) | Temporal — industry best practice, separates concerns |
| Separate TriggerContext from ActionContext | n8n, Activepieces — triggers have different lifecycles |
| ActionError signals retryability, engine decides policy | Temporal `ActivityError`, Prefect state model |
| Port-based topology (not implicit wiring) | n8n, Node-RED — explicit connections enable graph tooling |
| Version-aware registry | n8n node versioning — prevents silent upgrades |
| `stateless_fn` for closures | Temporal `register_activity` — closure registration is king for simple cases |

### 15.4 Deliberately Not Adopted

| Pattern | Platform | Why rejected |
|---------|----------|-------------|
| Untyped parameter access (`getNodeParameter` + cast) | n8n | Type safety is a core Nebula value |
| Language-native functions as actions (no framework) | Windmill | Nebula is an embedded library, not a platform — type safety matters |
| Mixin-based context extension | n8n `IExecuteFunctions` | Composition > inheritance — capability trait objects are cleaner |
| Implicit retry config on action | Prefect `@task(retries=3)` | Couples action to deployment policy — Temporal pattern is better |
| Global store on context | Activepieces `context.store` | Nebula uses ResourceAction for scoped state — more explicit |

---

## 16. Idempotency & Durable Execution

### 16.1 IdempotencyManager (v1 blocker for financial workflows)

`IdempotencyManager` must be backed by durable `Storage` (Postgres in production), not in-memory. Keys are deterministic: `{execution_id}:{node_id}:{attempt}` — survives process restarts.

```rust
pub trait IdempotencyManager: Send + Sync {
    /// Atomically check if operation already completed AND reserve the slot if not.
    /// Returns `Ok(Some(value))` if already completed (cached result).
    /// Returns `Ok(None)` if slot was reserved — caller must execute and then `complete()`.
    /// This is atomic to prevent TOCTOU: separate check() + record() would allow
    /// concurrent executions to both pass the check before either records.
    async fn check_and_reserve(
        &self,
        key: &IdempotencyKey,
        ttl: Duration,
    ) -> Result<Option<Value>, StorageError>;

    /// Record the result for a previously reserved slot.
    /// Called after successful execution. If the action fails, the reservation
    /// expires after `ttl` and the slot becomes available for retry.
    async fn complete(
        &self,
        key: &IdempotencyKey,
        result: &Value,
    ) -> Result<(), StorageError>;
}

pub struct IdempotencyKey {
    pub execution_id: ExecutionId,
    pub node_id: NodeId,
    pub attempt: u32,
}
```

**Implementation note:** `check_and_reserve` uses `INSERT ... ON CONFLICT` (Postgres) or equivalent atomic operation. The reservation has a TTL — if the action crashes before calling `complete()`, the slot expires and allows retry.

### 16.2 Compensating Transactions

No engine-level saga for v1. Document: if node N succeeds but N+1 fails, action N handles compensation via `TransactionalAction::compensate()`. Engine saga orchestration (automatic rollback across nodes) is post-v1.

---

## 17. Execution Timeout

Engine wraps every `action.execute()` call in `tokio::time::timeout`. Actions do not manage their own timeouts.

```rust
// Engine-side (simplified):
let timeout_duration = action_metadata
    .timeout
    .unwrap_or(engine_config.default_action_timeout);

// Timeout produces Retryable — a timeout is often transient (upstream may be
// slow temporarily). The engine's retry policy decides whether to actually retry.
let result = tokio::time::timeout(timeout_duration, handler.execute(input, &ctx))
    .await
    .map_err(|_| ActionError::Retryable {
        error: Arc::new(anyhow::anyhow!("action execution timed out after {:?}", timeout_duration)),
        code: Some(ErrorCode::UpstreamTimeout),
        backoff_hint: None,
        partial_output: None,
    })?;
```

**Timeout source priority:**
1. `ActionMetadata.timeout` — action author declares expected maximum duration
2. `EngineConfig.default_action_timeout` — engine-wide fallback (default: 5 minutes)
3. Per-node override in workflow definition (future — not in v1)

**Why actions don't self-timeout:** Consistent enforcement, no forgotten timeouts, engine controls the cancellation path (drops the future, triggering RAII cleanup). Actions that need progress-based deadlines use `ActionResult::Continue` with delay.

---

## 18. Plugin Manifest Integration

Actions are packaged in plugins. Each plugin declares its actions in a manifest:

```toml
# nebula-plugin.toml
[plugin]
name = "slack"
version = "1.0.0"
nebula_version = ">=1.0, <2.0"

[actions]
"send_message" = { version = "1.0" }
"create_channel" = { version = "1.0" }
"list_channels" = { version = "1.0" }
# Fully qualified at runtime: PluginKey("slack") + ActionKey("send_message") → "slack.send_message"

[credentials]
"slack_oauth2" = { pattern = "OAuth2" }

[data_tags]
produces = ["comm.slack.message"]
consumes = ["text", "json"]
```

Plugin owns its actions. `PluginKey("slack")` + `ActionKey("send_message")` → fully qualified `"slack.send_message"` at runtime. Enforced at registration time — an action cannot register without a plugin context.

---

## Post-Conference Amendments (Round 3)

Findings from conference review sessions. Each item has a tracking ID (C1-C6) for cross-referencing with the roadmap.

### C1. Action-level idempotency key method (Stripe feedback)

For actions making external side-effect calls (payments, etc.), the engine-level `IdempotencyKey` includes `attempt` which changes on retry. Action authors need a stable key for external APIs.

**Solution:** Add optional method to `Action` trait:

```rust
pub trait Action {
    fn metadata(&self) -> &ActionMetadata;

    /// Stable idempotency key for external calls.
    /// Default: None (no action-level idempotency).
    /// Engine passes this to ActionContext so the action can use it for external API calls.
    /// Key is derived from execution_id + node_id (WITHOUT attempt).
    fn external_idempotency_key(&self) -> Option<String> { None }
}
```

Engine provides this via `ctx.external_idempotency_key()` — stable across retries.

### C2. CostMetrics as Vec\<CostEntry\> (Vercel AI feedback)

One action may make multiple LLM calls with different models. Single `Cost` struct is insufficient.

**Solution:** Change `Cost` to support multiple entries:

```rust
pub struct Cost {
    pub entries: Vec<CostEntry>,
    pub total_usd_cents: Option<f64>,
}

pub struct CostEntry {
    pub model_id: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: Option<u64>,
    pub estimated_cost_usd: Option<f64>,
}
```

### C3. StatefulAction::migrate_state() for schema evolution (Airflow feedback)

When action version changes, persisted state from v1 may not deserialize into v2's `State` type.

**Solution:** Add default method to `StatefulAction`:

```rust
pub trait StatefulAction: Action {
    // ... existing methods ...

    /// Migrate state from a previous version.
    /// Called when direct deserialization fails.
    /// Default: returns None (no migration, error propagated).
    fn migrate_state(&self, old: Value) -> Option<Self::State> { None }
}
```

Engine: try deserialize -> on fail, call `migrate_state(raw_json)` -> on `None`, propagate error.

### C4. HTTP as core resource, not declarative action (n8n feedback, refined)

Instead of `HttpDeclarativeAction` (removed from scope), HTTP functionality is provided as a core resource:

```rust
// Available to any action via resource system
let http = ctx.resource::<HttpResource>()?;
let response = http.get(&url)
    .bearer(cred)
    .timeout(Duration::from_secs(30))
    .send()
    .await
    .retryable()?;
```

`HttpResource` is a pre-configured HTTP client provided by the runtime. Plugin authors don't need to manage `reqwest` directly. `HttpResource` handles:
- Connection pooling
- Default timeouts
- Proxy configuration
- TLS settings
- Request/response logging via `ActionLogger`

This is a nebula-resource concern (not nebula-action), but noted here because it directly affects action authoring DX.

### C5. Explicit threat model documentation (Datadog security feedback)

**Threat Model:**

- **In-process plugins (v1):** Trusted code. Same address space as engine. Has access to filesystem, network, env vars. `ScopedCredentialAccessor`/`ScopedResourceAccessor` enforce access to *Nebula-managed* resources only. No syscall-level isolation.
- **WASM plugins (v2):** Untrusted code. Sandboxed execution. No ambient authority. All capabilities explicitly granted via context.
- **Community plugins without WASM:** Must pass automated security pipeline (`cargo-audit`, `cargo-geiger`, `cargo-deny`). NOT sandboxed. Users assume risk.

### C6. Publish cargo expand output for derive macros (Figma DX feedback)

Phase 2a deliverable: alongside the derive macro, publish `cargo expand` output showing what `#[derive(Action)]` generates for canonical examples. This builds trust with plugin authors who want to understand the magic.

See roadmap Phase 2a for delivery details.

### C7. FailAfterAll error strategy (Netflix/Conductor feedback)

Current engine has 3 error strategies: FailFast, ContinueOnError, IgnoreErrors. Missing fourth strategy for data pipelines.

**Problem:** When node 3 fails, nodes 4-5 (independent of node 3) should still execute. ContinueOnError does this but marks execution as success. FailAfterAll executes remaining independent nodes, then marks execution as FAILED.

**Difference:**
- ContinueOnError: "execution succeeded, some nodes had errors" (status: Success)
- FailAfterAll: "execution failed, but we maximized useful work" (status: Failed)

**Solution:** Add fourth variant to engine error strategy enum:
```rust
pub enum ErrorStrategy {
    FailFast,         // Stop on first failure
    ContinueOnError,  // Continue, final status = Success
    IgnoreErrors,     // Treat failures as success
    FailAfterAll,     // NEW: Continue independent nodes, final status = Failed
}
```
This is a nebula-engine concern, noted here because it affects how ActionError propagation works end-to-end.

### C8. SecretString lint for plugin certification (HashiCorp/Vault feedback)

AuthScheme implementations must derive Zeroize, but `Zeroize` on `String` only zeroizes the stack pointer — heap allocation may be copied by the allocator. Real secure zeroization requires `secrecy::SecretString`.

**Problem:** Plugin author writes:
```rust
struct MyCredential {
    token: String,  // Zeroize technically works but heap not guaranteed clean
}
```
Instead of:
```rust
struct MyCredential {
    token: SecretString,  // Proper zeroization via mlock + explicit overwrite
}
```

**Solution:** Add to plugin certification pipeline (automated checks, not compile-time):
- WARN if AuthScheme struct contains `String` field with name matching: `token`, `key`, `secret`, `password`, `api_key`, `access_token`, `refresh_token`, `signing_key`, `private_key`
- Recommendation: use `secrecy::SecretString` for these fields
- Not a compile error — certification warning only. Some credentials legitimately use String (e.g., username, base_url).

### C9. Dynamic credential resolution (Zapier feedback) — future consideration

Current design: credentials declared statically by type (`#[action(credential = SlackOAuthToken)]`). But some workflows need runtime credential selection based on upstream data (e.g., "use credential for account_id from previous node's output").

**Current workaround:** Use ResourceAction to resolve credential dynamically and inject into downstream context. Works but adds a DAG node for credential routing.

**Future design (post-v1):** First-class dynamic credential resolution:
```rust
#[action(credential(dynamic) = SlackOAuthToken)]

// In execute:
let account_id = input["account_id"].as_str().unwrap();
let cred = ctx.credential_dynamic::<SlackOAuthToken>(account_id)?;
// Engine resolves: TypeId(SlackOAuthToken) + account_id → specific credential instance
```
This requires credential system to support multi-instance per type (keyed by tenant/account), which is a nebula-credential v4 concern. Deferred to post-v1.

---

## 19. Not In Scope

| Feature | Reason | When |
|---------|--------|------|
| `InteractiveAction` | Needs engine suspended execution support | Post engine v2 |
| `HttpDeclarativeAction` | Config-driven HTTP actions — separate crate/spec, not core action concern | Separate spec |
| `StreamProcessor` DX | Real-time audio/video pipelines — needs dedicated runtime | Post v2 |
| `ProcessAction` (external process) | Needs sandbox Phase 2 (nsjail/WASM) | Post sandbox |
| `QueueAction` | Deployment topology not finalized | Post v2 |
| `CachePolicy` (incremental re-exec) | Engine DAG concern, complex invalidation | Post engine v2 |
| `Task<T>` structured concurrency | `ctx.spawn()` helper — needs runtime integration | v1.1 |
| Full expression sandbox | nebula-expression concern | Separate spec |
| WASM plugin loading | nebula-plugin v2 concern | Separate spec |
| Multi-language SDK | Python/TS → WASM compilation | Post WASM plugins |

---

## 20. Compatibility & Migration

### 20.1 Schema-Stable Types

These types have frozen JSON serialization — changes require major version bump:
- `ActionResult<T>` — tagged enum, Duration as milliseconds
- `Routing` — sub-enum of ActionResult
- `ActionOutput<T>` — tagged enum
- `ActionError` — tagged enum
- `ErrorCode` — semantic error codes
- `FlowKind`, `InputPort`, `OutputPort`, `SupportPort`, `DynamicPort`
- `BreakReason`, `WaitCondition`

Contract tests in `crates/action/tests/contracts.rs` enforce stability.

### 20.2 Deprecation Policy

Minimum 1 minor version cycle. `#[deprecated(since = "0.x.0", note = "use Y instead")]` with replacement path.

### 20.3 Extension Trait Safety

Extension traits (new methods) always have default implementations — existing code never breaks.

---

## 21. Open Questions

1. **DataTag registry location** — nebula-core (available everywhere) or nebula-action (closer to usage)?
2. **Dynamic property options loader** — how does the UI invoke the loader? Via engine API? Direct action call?
3. **Provide port → Support port resolution** — how does engine collect tools from multiple connected providers into `AgentContext.tools`?
