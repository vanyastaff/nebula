# nebula-action

Execution abstraction layer for workflow nodes. Defines **what actions are** and **how they
communicate with the engine**. Follows the Ports & Drivers architecture: core types and
contracts live here; concrete execution environments (in-process, WASM sandbox) are
implemented as drivers by `nebula-runtime` and `nebula-sandbox`.

**Depends on:** `nebula-core`, `nebula-parameter`, `nebula-credential`, `nebula-resource`

---

## What Is an Action?

An action is a self-contained unit of work that:

- Has typed inputs and outputs (Rust structs / `serde_json::Value`)
- Is stateless **or** maintains serializable state between calls
- Declares its credential and resource dependencies via `ActionComponents`
- Returns an `ActionResult` controlling what the engine does next
- Produces observability signals through `ActionContext` (logging, metrics, tracing)

Actions are the leaf nodes of the workflow graph. They do the actual work; the engine
orchestrates them.

---

## Architecture

```
Action author                 Engine / Runtime
─────────────                 ────────────────
impl Action trait             reads ActionMetadata
  metadata()                  discovers ports, parameters, capabilities
  components()                resolves credentials + resources before exec

impl ProcessAction / etc.     calls execute(input, ctx)
  execute(input, ctx)

returns ActionResult          dispatches next step in workflow DAG
  with ActionOutput           stores / passes data downstream
```

The engine stores actions as `Arc<dyn Action>` in the registry. Sub-traits refine the
execution contract with typed input/output.

---

## Trait Hierarchy

```
Action (base: metadata + components)
├── StatelessAction (= ProcessAction)   Input → Output, pure function
├── StatefulAction                      persistent state between calls
│   ├── TriggerAction                   workflow starter, lives outside the DAG
│   ├── InteractiveAction  [DX]         human-in-the-loop
│   └── TransactionalAction [DX]        Saga / compensation
└── ResourceAction (= SupplyAction)     injects a resource into downstream nodes
    └── TriggerAction also extends StatefulAction:
        ├── WebhookAction  [DX]         inbound HTTP webhook
        └── PollAction     [DX]         cursor-based polling
```

The engine distinguishes **four core types** (`StatelessAction`, `StatefulAction`,
`TriggerAction`, `ResourceAction`). The DX types (`InteractiveAction`,
`TransactionalAction`, `WebhookAction`, `PollAction`) are convenience wrappers — an
experienced developer can implement the same behaviour via the corresponding core trait.

---

## Module Map

| Module | What it provides |
|---|---|
| `action` | `Action` — base trait (identity + dependency declaration) |
| `result` | `ActionResult<T>` — 9-variant flow-control enum |
| `output` | `ActionOutput<T>` — 7-variant data delivery enum |
| `error` | `ActionError` — retryable vs fatal distinction |
| `metadata` | `ActionMetadata`, `InterfaceVersion` |
| `components` | `ActionComponents` — credential + resource declarations |
| `port` | `InputPort`, `OutputPort`, `SupportPort`, `DynamicPort`, `ConnectionFilter` |
| `context` | `Context` trait base; `ActionContext` / `TriggerContext` implemented by engine |
| `prelude` | Convenience re-exports |

---

## Core Types

### `Action` — base trait

Object-safe; stored as `Arc<dyn Action>` in the registry.

```rust
pub trait Action: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;
    fn components(&self) -> ActionComponents;
}
```

---

### `StatelessAction` (formerly `ProcessAction`)

Pure function. No state, no coordination. The most common type.

```rust
pub trait StatelessAction: Action {
    type Input: DeserializeOwned;
    type Output: Serialize;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>>;
}
```

Valid `ActionResult` variants: `Success`, `Skip`, `Branch`, `Route`, `MultiOutput`, `Retry`.

**Use when:** data transforms, API calls, validation, content generation.

---

### `StatefulAction`

Persistent state between calls. The engine serializes `State` after each iteration and
restores it on the next call.

```rust
pub trait StatefulAction: Action {
    type State: Serialize + DeserializeOwned + Default;
    type Input: DeserializeOwned;
    type Output: Serialize;

    async fn execute(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>>;

    async fn initialize_state(
        &self,
        input: &Self::Input,
        ctx: &ActionContext,
    ) -> Result<Self::State> {
        Ok(Self::State::default())
    }

    fn state_version(&self) -> u32 { 1 }

    async fn migrate_state(
        &self,
        old_state: serde_json::Value,
        old_version: u32,
        new_version: u32,
    ) -> Result<Self::State>;
}
```

Additional `ActionResult` variants: `Continue { delay }`, `Break { reason }`,
`Wait { condition }`.

`Wait` suspends execution until an external event (human approval, HTTP callback) without
creating a separate action type:

```rust
Ok(ActionResult::Wait {
    condition: WaitCondition::Approval {
        approver: "manager@company.com".into(),
        message: "Approve deployment to production?".into(),
    },
    timeout: Some(Duration::from_secs(86400)),
    partial_output: None,
})
```

**Use when:** paginating large datasets, batch processing with resume, accumulation,
rate-limited operations, human-in-the-loop (via `Wait`).

---

### `TriggerAction`

Extends `StatefulAction`. Lives **outside** the execution DAG — spawns workflow
executions rather than processing data inside them. Managed by `TriggerManager`
independently of `WorkflowEngine`.

State is mandatory: the trigger must remember what it has already processed to avoid
duplicate executions.

```rust
pub trait TriggerAction: StatefulAction {
    type Event: Serialize;

    /// Called once when the workflow is deployed.
    async fn start(&self, ctx: &TriggerContext) -> Result<()>;

    /// Called on undeploy or shutdown.
    async fn stop(&self, ctx: &TriggerContext) -> Result<()>;
}
```

`TriggerContext` differs from `ActionContext` — it is **not bound to a specific
execution**; the trigger lives at the workflow level.

**Use when:** any external event source that initiates a workflow.

---

### `ResourceAction` (formerly `SupplyAction`)

Provides a capability (resource, service, tool) to its downstream nodes via the graph.
Analogous to n8n's `supplyData` — how an AI Agent node receives tools from connected
nodes.

The engine manages the lifecycle explicitly:

1. Calls `ResourceAction::configure()` **before** downstream nodes execute.
2. Creates a `Resource::Instance` via `nebula-resource` (with pooling and health checks).
3. Makes it available downstream via `ctx.resource::<R>()`.
4. After all downstream nodes finish, calls `cleanup()` with the owned instance —
   guaranteed that no other node still holds it.

```rust
pub trait ResourceAction: Action {
    type Resource: Resource;

    async fn configure(
        &self,
        ctx: &ActionContext,
    ) -> Result<<Self::Resource as Resource>::Config>;

    async fn cleanup(
        &self,
        resource: <Self::Resource as Resource>::Instance,
        ctx: &ActionContext,
    ) -> Result<()> {
        drop(resource);
        Ok(())
    }
}
```

Three reasons this is a **core type**, not DX:
- **Execution order** — the engine has topological knowledge that `ResourceAction` must
  precede its downstream nodes.
- **Scoped lifecycle** — the resource lives only while the downstream branch executes,
  then `cleanup` is called. Not global.
- **Branch isolation** — the resource is visible only to downstream nodes in that branch.

```
┌─────────────────────┐
│ PostgresPool        │  ← ResourceAction (configure + scoped lifecycle)
│ (ResourceAction)    │
└──────────┬──────────┘
           │  resource scoped to this branch only
           ▼
┌─────────────────────┐
│ QueryUsers          │  ← ctx.resource::<DatabasePool>()
│ (StatelessAction)   │
└─────────────────────┘
```

`ResourceAction` = dependency injection via the graph.
`ctx.resource()` = access to the resource registry in `nebula-resource`.

**Use when:** providing a DB connection pool, injecting AI tools into an agent node,
configuring a credentialed HTTP client for a specific branch.

---

## DX Types

Convenience wrappers. The engine uses the underlying core trait — the DX layer removes
boilerplate. Any DX type can be implemented manually via its corresponding core trait.

### `InteractiveAction` *(DX over StatefulAction)*

Human-in-the-loop. Declarative API over `ActionResult::Wait`.

Two patterns:

```rust
// Human IN the loop — must approve before workflow continues
WaitCondition::Approval {
    approver: "legal@company.com".into(),
    message: "Approve contract before sending to client".into(),
    on_timeout: OnTimeout::Escalate { to: "cto@company.com".into() },
}

// Human ON the loop — can intervene within a window; auto-continues if not
WaitCondition::Approval {
    approver: "ops@company.com".into(),
    message: "Deployment ready. Override within 10 min to cancel.".into(),
    on_timeout: OnTimeout::AutoApprove,
}
```

### `TransactionalAction` *(DX over StatefulAction)*

Saga pattern with automatic compensation management.

```rust
pub trait TransactionalAction: StatefulAction {
    type CompensationData: Serialize + DeserializeOwned;

    async fn execute_tx(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<(Self::Output, Self::CompensationData)>;

    async fn compensate(
        &self,
        data: Self::CompensationData,
        ctx: &ActionContext,
    ) -> Result<()>;

    fn max_compensation_retries(&self) -> u32 { 3 }
    fn step_kind(&self) -> SagaStepKind { SagaStepKind::Compensable }
}
```

Step kinds:

| Kind | Behaviour |
|---|---|
| `Compensable` | Can be rolled back — `compensate()` called if a later step fails |
| `Pivot` | Point of no return; nothing before it can be compensated after success |
| `Retryable` | After `Pivot`; must be idempotent, retried until success |

Example — order checkout:

```
[Reserve Inventory]   Compensable  → compensate: release inventory
[Charge Payment]      Pivot        → point of no return
[Update Order Status] Retryable    → forward-only, idempotent
[Send Confirmation]   Retryable    → forward-only, idempotent
```

### `WebhookAction` *(DX over TriggerAction)*

```rust
pub trait WebhookAction: TriggerAction {
    async fn register(&self, ctx: &TriggerContext) -> Result<WebhookRegistration>;

    async fn handle_request(
        &self,
        request: IncomingRequest,
        state: &Self::State,
        ctx: &TriggerContext,
    ) -> Result<Option<Self::Event>>;

    async fn verify_signature(&self, request: &IncomingRequest, secret: &str) -> Result<bool>;
}
```

State stores registration ID, endpoint URL, and secret. The engine calls `register` on
startup and `handle_request` on each incoming HTTP request.

### `PollAction` *(DX over TriggerAction)*

```rust
pub trait PollAction: TriggerAction {
    type Cursor: Serialize + DeserializeOwned + Default;

    fn poll_interval(&self) -> Duration;

    async fn poll(
        &self,
        cursor: &Self::Cursor,
        ctx: &TriggerContext,
    ) -> Result<PollResult<Self::Event, Self::Cursor>>;
}

pub struct PollResult<E, C> {
    pub events: Vec<E>,
    pub next_cursor: C,
    /// true = engine calls poll again immediately (no wait)
    pub has_more: bool,
}
```

The cursor is persisted only after events are successfully processed.

---

## Context System

Actions receive a context injected by the engine. There are two distinct context types:

### `ActionContext`

Provided to `StatelessAction`, `StatefulAction`, `ResourceAction`, and DX types.
Bound to a specific execution.

```rust
// Identity
ctx.execution_id()    // ExecutionId
ctx.workflow_id()     // WorkflowId
ctx.node_id()         // NodeId
ctx.check_cancelled() // Result<(), ActionError::Cancelled>

// Dependencies (declared via ActionComponents)
ctx.get_resource::<R>()      // Result<R::Instance>
ctx.get_credential("smtp")   // Result<AuthData>

// Observability
ctx.log_info("message")
ctx.log_warn("message")
ctx.log_error("message")
ctx.emit_metric("name", value)
ctx.publish_event(event)
```

The engine may wrap `ActionContext` in `SandboxedContext` which proxies every call through
capability checks declared in `ActionComponents`. A denied call surfaces as
`ActionError::SandboxViolation`.

### `TriggerContext`

Provided to `TriggerAction`, `WebhookAction`, `PollAction`. **Not bound to a specific
execution** — the trigger lives at the workflow level, spawning executions.

```rust
ctx.workflow_id()       // WorkflowId
ctx.log_info("...")
ctx.emit_metric(...)
// No execution_id — triggers are not inside an execution
```

---

## `ActionResult<T>`

Controls **workflow flow**. All output fields wrap `ActionOutput<T>`.

```rust
#[non_exhaustive]
pub enum ActionResult<T> {
    Success   { output: ActionOutput<T> },
    Skip      { reason: String, output: Option<ActionOutput<T>> },
    Continue  { output: ActionOutput<T>, progress: Option<f64>, delay: Option<Duration> },
    Break     { output: ActionOutput<T>, reason: BreakReason },
    Branch    { selected: BranchKey, output: ActionOutput<T>,
                alternatives: HashMap<BranchKey, ActionOutput<T>> },
    Route     { port: PortKey, data: ActionOutput<T> },
    MultiOutput { outputs: HashMap<PortKey, ActionOutput<T>>,
                  main_output: Option<ActionOutput<T>> },
    Wait      { condition: WaitCondition, timeout: Option<Duration>,
                partial_output: Option<ActionOutput<T>> },
    Retry     { after: Duration, reason: String },
}
```

### Variant semantics

| Variant | Engine behavior | Use case |
|---|---|---|
| `Success` | Pass output to dependent nodes | Normal completion |
| `Skip` | Skip downstream; reason logged | Filtered out, no work to do |
| `Continue` | Re-invoke after optional `delay` | Pagination, rate-limit cooldown |
| `Break` | Iteration complete; pass output | All pages fetched, batch done |
| `Branch` | Activate connections for `selected` key | if/else, switch |
| `Route` | Send `data` to a specific named port | Conditional routing |
| `MultiOutput` | Fan-out to multiple ports simultaneously | Audit + main, copy + transform |
| `Wait` | Suspend until `condition` met | Human approval, HTTP callback, schedule |
| `Retry` | Re-enqueue after `after` | Upstream not ready, voluntary backoff |

`Retry` is a **successful** signal (the action chose to wait); it is distinct from
`ActionError::Retryable` (engine-driven retry after a failure).

### Convenience constructors

```rust
ActionResult::success(value)
ActionResult::success_binary(data)
ActionResult::success_reference(ref)
ActionResult::success_empty()
ActionResult::success_output(output)
ActionResult::success_deferred(deferred)
ActionResult::skip("reason")
ActionResult::skip_with_output("reason", value)
```

### `BreakReason`

```rust
pub enum BreakReason {
    Completed,
    MaxIterations,
    ConditionMet,
    Custom(String),
}
```

### `WaitCondition`

```rust
pub enum WaitCondition {
    Webhook   { callback_id: String },
    Until     { datetime: DateTime<Utc> },
    Duration  { duration: Duration },
    Approval  { approver: String, message: String },
    Execution { execution_id: ExecutionId },
}
```

---

## `ActionOutput<T>`

Describes **data and its delivery state** — orthogonal to `ActionResult`.

```rust
#[non_exhaustive]
pub enum ActionOutput<T> {
    Value(T),
    Binary(BinaryData),
    Reference(DataReference),
    Deferred(Box<DeferredOutput>),
    Streaming(StreamOutput),
    Collection(Vec<ActionOutput<T>>),
    Empty,
}
```

`Deferred` and `Streaming` require resolution — `needs_resolution()` returns `true`. The
engine resolves them before passing data downstream.

### `DeferredOutput`

Used when an action has kicked off async work (AI generation, document export) but the
result isn't ready yet.

```rust
pub struct DeferredOutput {
    pub handle_id: String,
    pub resolution: Resolution,
    pub expected: ExpectedOutput,
    pub progress: Option<Progress>,
    pub producer: Producer,
    pub retry: Option<DeferredRetryConfig>,
    pub timeout: Option<Duration>,
}

pub enum Resolution {
    Poll    { target: PollTarget, interval: Duration, backoff: f64,
              max_interval: Option<Duration> },
    Await   { channel_id: String },
    Callback { endpoint: String, token: String },
    SubWorkflow { workflow_id: String, input: Option<serde_json::Value> },
    AwaitOrPoll { channel_id, fallback_after, poll_target, poll_interval },
}
```

### `StreamOutput`

```rust
pub enum StreamMode {
    Tokens  { model: String },          // LLM; engine concatenates into text
    Bytes   { content_type, total_size }, // file download, binary gen
    Deltas  { format: DeltaFormat },    // JSON patches (JsonMergePatch | JsonPatch)
    Events,                             // SSE-style typed events
    Custom  { protocol: String },
}
```

### Ergonomic constructors

```rust
ActionOutput::deferred_ai(handle_id, model, provider, resolution, expected)
ActionOutput::deferred_document(handle_id, content_type, resolution)
ActionOutput::deferred_callback(handle_id, endpoint, token, expected, timeout)
ActionOutput::llm_stream(stream_id, model)
ActionOutput::byte_stream(stream_id, content_type, total_size)
```

### `OutputEnvelope<T>` (engine-level)

Actions return `ActionOutput<T>`; the engine wraps it in `OutputEnvelope<T>` before
persisting and passing downstream:

```rust
pub struct OutputEnvelope<T> {
    pub output: ActionOutput<T>,
    pub meta: OutputMeta,   // origin, timing, cost (tokens), cache info, trace_id
}
```

---

## `ActionError`

```rust
#[non_exhaustive]
pub enum ActionError {
    Retryable {
        error: String,
        backoff_hint: Option<Duration>,
        partial_output: Option<serde_json::Value>,
    },
    Fatal {
        error: String,
        details: Option<serde_json::Value>,
    },
    Validation(String),
    SandboxViolation { capability: String, action_id: String },
    Cancelled,
    DataLimitExceeded { limit_bytes: u64, actual_bytes: u64 },
}
```

| Variant | `is_retryable()` | `is_fatal()` | Meaning |
|---|---|---|---|
| `Retryable` | `true` | `false` | Transient; engine applies retry policy |
| `Fatal` | `false` | `true` | Permanent; invalid creds, schema mismatch |
| `Validation` | `false` | `true` | Failed before execution started |
| `SandboxViolation` | `false` | `true` | Capability denied by sandbox |
| `Cancelled` | `false` | `false` | CancellationToken was triggered |
| `DataLimitExceeded` | `false` | `true` | Output too large |

```rust
ActionError::retryable("connection reset")
ActionError::retryable_with_backoff("rate limited", Duration::from_secs(5))
ActionError::retryable_with_partial("partial fail", json!({"done": 3}))
ActionError::fatal("invalid credentials")
ActionError::fatal_with_details("auth failed", json!({"field": "password"}))
ActionError::validation("email is required")
```

---

## `ActionComponents`

Declares runtime dependencies. The engine resolves and verifies them before calling
`execute`. Used also by `nebula-sandbox` for capability checks.

```rust
let components = ActionComponents::new()
    .credential(CredentialRef::of::<GithubToken>())
    .resource(ResourceRef::of::<PostgresDb>());
```

---

## `ActionMetadata`

Static descriptor used by the engine for discovery, schema validation, and versioning.

```rust
pub struct ActionMetadata {
    pub key: String,              // e.g. "http.request"
    pub name: String,             // e.g. "HTTP Request"
    pub description: String,
    pub version: InterfaceVersion,
    pub inputs: Vec<InputPort>,   // default: [InputPort::flow("in")]
    pub outputs: Vec<OutputPort>, // default: [OutputPort::flow("out")]
    pub parameters: ParameterCollection,
}

ActionMetadata::new("http.request", "HTTP Request", "Make HTTP calls")
    .with_version(2, 1)
    .with_inputs(vec![InputPort::flow("in")])
    .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("error")])
    .with_parameters(ParameterCollection::new().with(/* defs */))
```

`InterfaceVersion` is semver-like (`major.minor`). Compatibility:
`is_compatible_with(&required)` → major must match AND `actual.minor >= required.minor`.

---

## Port System

Ports describe connection topology — how nodes wire together in the workflow graph.

```
InputPort::Flow   — main data pipe
InputPort::Support — sub-node / supply slot (AI model, tool, memory)

OutputPort::Flow   — Main or Error output
OutputPort::Dynamic — config-driven ports (e.g. Switch node generates one per rule)
```

```rust
// Input
InputPort::flow("in")
InputPort::support("model", "AI Model", "Language model to use")
InputPort::Support(SupportPort {
    key: "tools".into(), name: "Tools".into(), description: "...",
    required: false, multi: true,
    filter: ConnectionFilter::new().with_allowed_tags(vec!["langchain_tool".into()]),
})

// Output
OutputPort::flow("out")                   // FlowKind::Main
OutputPort::error("error")                // FlowKind::Error
OutputPort::dynamic("rule", "rules")      // one port per element in config["rules"]
```

---

## Resource Consumption

All action types access resources through `ActionContext` — orthogonal to the type
hierarchy:

```rust
let db    = ctx.get_resource::<DatabaseResource>().await?;
let cache = ctx.get_resource::<CacheResource>().await?;
let key   = ctx.get_credential("api_key").await?;
```

This is **global access** to the resource registry. `ResourceAction` is about
**providing** a resource scoped to a graph branch — distinct concepts.

---

## Development Approaches

Three complementary ways to build actions:

**1. Simple approach** (`SimpleAction`) — minimal boilerplate, good for prototypes:

```rust
impl SimpleAction for SendEmailAction {
    type Input = EmailInput;
    type Output = EmailOutput;

    async fn execute_simple(&self, input: Self::Input, ctx: &ActionContext) -> Result<Self::Output> {
        let smtp = ctx.get_credential("smtp").await?;
        let id = EmailClient::new(&smtp).send(&input.to, &input.body).await?;
        Ok(EmailOutput { message_id: id })
    }
}
```

**2. Derive macros** (`nebula-derive`) — preferred for production:

```rust
#[derive(Action)]
#[action(id = "database.user_lookup", name = "User Lookup")]
#[resources([DatabaseResource, CacheResource])]
#[credentials(["database"])]
pub struct UserLookupAction;
```

**3. Trait-based** — maximum control over serialization, state migration, advanced
behaviour. Implement `StatelessAction`, `StatefulAction`, `TriggerAction`, or
`ResourceAction` directly.

All three implement the same core traits, so the engine treats them uniformly.

---

## Quick Selection Guide

```
Need an action?
│
├── Initiates a workflow from an external event?
│   └── yes → TriggerAction
│             ├── Inbound HTTP? → WebhookAction (DX)
│             └── Polls a source? → PollAction (DX)
│
├── Provides a resource to downstream nodes?
│   └── yes → ResourceAction
│
├── Needs state between calls?
│   └── yes → StatefulAction
│             ├── Saga/rollback? → TransactionalAction (DX)
│             └── Human input? → InteractiveAction (DX)
│                               (or just ActionResult::Wait directly)
│
└── no → StatelessAction (or SimpleAction for minimal boilerplate)
```

---

## Summary Table

| Type | Core/DX | State | Extends | Purpose |
|---|---|---|---|---|
| `StatelessAction` | Core | ❌ | `Action` | Input → Output, pure function |
| `StatefulAction` | Core | ✅ | `Action` | Iterative processing with state |
| `TriggerAction` | Core | ✅ | `StatefulAction` | Workflow starter |
| `ResourceAction` | Core | ❌ | `Action` | Injects resource into downstream nodes |
| `InteractiveAction` | DX | ✅ | `StatefulAction` | Human-in-the-loop, approvals |
| `TransactionalAction` | DX | ✅ | `StatefulAction` | Saga / compensation |
| `WebhookAction` | DX | ✅ | `TriggerAction` | Inbound HTTP webhook |
| `PollAction` | DX | ✅ | `TriggerAction` | Cursor-based source polling |

---

## Prelude

```rust
use nebula_action::prelude::*;
// Action, ActionComponents, Context, ActionError, ActionMetadata,
// ActionOutput, DeferredOutput, ExpectedOutput, Producer, ProducerKind,
// Progress, Resolution, StreamMode, StreamOutput,
// ConnectionFilter, DynamicPort, FlowKind, InputPort, OutputPort, SupportPort,
// ActionResult,
// ParameterCollection, ParameterDef
```

---

## Usage in the Crate Ecosystem

- **nebula-node** — groups actions and credentials into versioned, reusable nodes.
- **nebula-registry** — indexes available actions using `Action::metadata()`.
- **nebula-runtime** / **nebula-execution** — orchestrate execution, inject `ActionContext`,
  wrap results in `OutputEnvelope`.
- **nebula-sandbox** — enforces capability isolation based on `ActionComponents`;
  wraps `ActionContext` in `SandboxedContext` that proxies all resource/credential
  accesses through capability checks.
- **nebula-testing** — utilities for testing actions with mock context implementations.
- **nebula-sdk** — re-exports action APIs for external action authors.

### Archive references

- `crates/action/docs/` — internal design docs (Action Types, Action Result System, etc.)
- `docs/archive/nebula-action-types.md` — core/DX type system philosophy
- `docs/archive/layers-interaction.md` — ActionContext design: resource + credential chain
- `docs/archive/node-execution.md` — three development approaches, SandboxedContext

