# 27 — `nebula-action` redesign

> **Status:** DRAFT
> **Authority:** subordinate to `docs/PRODUCT_CANON.md`. Canon wins on conflict.
> **Parent:** [`./README.md`](./README.md)
> **Scope:** Align `nebula-action` with specs 23 (cross-crate foundation),
> 24 (nebula-core redesign), 25/26 (resource/credential redesign). Replace
> Context trait, ActionContext/TriggerContext structs → traits, unify
> dependencies, remove false capabilities, leverage Rust 1.94.
> **Depends on:** 23, 24, 25, 26, 09 (retry cascade), 21 (schema)
> **Consumers:** `nebula-engine`, `nebula-testing`, plugin crates, action authors

## 1. Problem

`nebula-action` (22 files, ~14.5K LOC) has a strong trait hierarchy for action
types but predates specs 23/24 and has these issues:

### 1.1 Canon violations

| Issue | Canon ref | Severity |
|---|---|---|
| `anyhow` in library crate Cargo.toml | CLAUDE.md: "typed errors in library crates" | 🔴 |
| `AgentHandler` stub with no engine support | §14: "Framework before product" | 🔴 |

### 1.2 Spec misalignments

| Issue | Spec ref |
|---|---|
| `nebula_action::Context` trait name-collides with `nebula_core::Context` | 23 |
| `ActionContext` is a struct (should be umbrella marker trait) | 23 |
| `TriggerContext` is a struct (should be umbrella marker trait) | 23 |
| `ActionDependencies` has 5 methods (should be `DeclaresDependencies` with 1) | 23 |
| `capability::ResourceAccessor` is string-based untyped (should use core accessor) | 25 |
| `ParameterCollection` from nebula-parameter (should be `Schema` from nebula-schema) | 21 |
| Direct deps on nebula-credential + nebula-resource (should use core accessor traits) | 23 |

### 1.3 Rust 1.94 improvements available

| Issue | Improvement |
|---|---|
| `async_trait` on 5 handler traits + 3 capability traits | RPITIT for non-dyn; explicit `Pin<Box<dyn Future>>` for dyn-safe |
| `anyhow::Error` in ActionError variants | `Arc<dyn std::error::Error + Send + Sync>` |
| No `#[diagnostic::on_unimplemented]` on key traits | Better compile errors for action authors |

### 1.4 What stays unchanged

- **Action trait hierarchy** — StatelessAction, StatefulAction, TriggerAction,
  ResourceAction, PaginatedAction, BatchAction, WebhookAction, PollAction,
  ControlAction — well-designed, each serves distinct use case ✓
- **ActionResult<T>** — rich enum (Success/Skip/Drop/Continue/Break/Branch/
  Route/MultiOutput/Wait/Retry/Terminate) ✓ (Retry stays, spec 09 implements)
- **ActionOutput<T>** — Value/Binary/Reference/Stream ✓
- **ActionError** — typed with RetryHintCode + ValidationReason + Classify ✓
  (anyhow→dyn Error refactor only)
- **Port system** — InputPort/OutputPort/DynamicPort ✓
- **Webhook module** — HMAC verification, type-safe ✓
- **Poll module** — cursor-based with dedup ✓
- **Handler pattern** — typed action → dyn handler → ActionHandler enum dispatch ✓
- **Validation module** — validate_action_package() ✓
- **Testing utilities** — SpyEmitter, SpyLogger, SpyScheduler ✓

## 2. Decision

### 2.1 Context system migration (spec 23)

**Delete** `nebula_action::Context` trait. Action traits use `nebula_core::Context`
as base + capability traits as supertrait bounds.

**Replace** `ActionContext` struct → `ActionContext` umbrella marker trait:

```rust
/// Umbrella trait for action execution contexts.
/// Blanket-impl'd — any type implementing all capabilities IS an ActionContext.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement all required action capabilities",
    note = "ActionContext requires: Context + HasResources + HasCredentials + HasLogger + HasMetrics + HasEventBus + HasNodeIdentity"
)]
pub trait ActionContext:
    nebula_core::Context
    + HasResources
    + HasCredentials
    + HasLogger
    + HasMetrics
    + HasEventBus
    + HasNodeIdentity
{}

impl<T> ActionContext for T where
    T: nebula_core::Context
    + HasResources + HasCredentials + HasLogger
    + HasMetrics + HasEventBus + HasNodeIdentity
{}
```

**Replace** `TriggerContext` struct → `TriggerContext` umbrella marker trait:

```rust
pub trait TriggerContext:
    nebula_core::Context
    + HasResources
    + HasCredentials
    + HasLogger
    + HasMetrics
    + HasEventBus
    + HasTriggerScheduling
{}

impl<T> TriggerContext for T where
    T: nebula_core::Context
    + HasResources + HasCredentials + HasLogger
    + HasMetrics + HasEventBus + HasTriggerScheduling
{}
```

**New traits defined in this crate:**

```rust
/// Action-specific capability: node identity within workflow graph.
pub trait HasNodeIdentity {
    fn node_key(&self) -> &NodeKey;
    fn attempt_id(&self) -> &AttemptId;
}

/// Trigger-specific capability: scheduling and event emission.
pub trait HasTriggerScheduling {
    fn scheduler(&self) -> &dyn TriggerScheduler;
    fn emitter(&self) -> &dyn ExecutionEmitter;
    fn health(&self) -> &TriggerHealth;
}
```

### 2.2 Action trait signature updates

```rust
// Before:
pub trait StatelessAction: Action {
    type Input: Send + Sync;
    type Output: Send + Sync;
    fn execute(&self, input: Self::Input, ctx: &impl Context)
        -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send;
}

// After:
pub trait StatelessAction: Action {
    type Input: Send + Sync;
    type Output: Send + Sync;
    fn execute(&self, input: Self::Input, ctx: &(impl ActionContext + ?Sized))
        -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send;
}
```

Same pattern for StatefulAction, ResourceAction, ControlAction.
TriggerAction uses `&(impl TriggerContext + ?Sized)`.

`+ ?Sized` allows passing `&dyn ActionContext` (trait objects) in addition
to concrete types.

### 2.3 ActionDependencies → DeclaresDependencies

**Delete** `dependency.rs` (112 lines). `Action` supertrait changes:

```rust
// Before:
pub trait Action: ActionDependencies + Send + Sync + 'static { ... }

// After:
pub trait Action: DeclaresDependencies + Send + Sync + 'static { ... }
```

`DeclaresDependencies` from `nebula-core` with single `fn dependencies() -> Dependencies`.

`#[derive(Action)]` macro generates `DeclaresDependencies` from attributes:

```rust
#[derive(Action)]
#[action(key = "http_request", category = "data")]
#[uses_credential(ApiKeyCredential)]
#[uses_resource(HttpResource)]
struct HttpRequestAction { meta: ActionMetadata }
```

### 2.4 Capability traits cleanup

**Delete** local capability traits that move to core/domain crates:

| Current (nebula-action) | Replacement | Location |
|---|---|---|
| `capability::ResourceAccessor` | `nebula_core::ResourceAccessor` | core |
| `capability::ActionLogger` | `nebula_core::Logger` | core |
| `capability::ActionLogLevel` | `nebula_core::LogLevel` | core |

**Keep** trigger-specific capabilities (no core equivalent):

| Trait | Stays | Reason |
|---|---|---|
| `TriggerScheduler` | ✅ | Trigger-domain, no generic equivalent |
| `ExecutionEmitter` | ✅ | Trigger-domain |
| `TriggerHealth` | ✅ | Trigger-domain (atomic counters) |

### 2.5 Handler traits — remove async_trait

Replace `#[async_trait]` with explicit `Pin<Box<dyn Future>>` on all dyn-safe
handler traits. Rust 1.94 — no proc macro needed.

```rust
// Before:
#[async_trait]
pub trait StatelessHandler: Send + Sync {
    fn metadata(&self) -> &ActionMetadata;
    async fn execute(&self, input: Value, ctx: &ActionContext)
        -> Result<ActionResult<Value>, ActionError>;
    async fn validate_input(&self, input: &Value) -> Result<(), ActionError>;
}

// After:
pub trait StatelessHandler: Send + Sync {
    fn metadata(&self) -> &ActionMetadata;
    fn execute(
        &self,
        input: Value,
        ctx: &dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionResult<Value>, ActionError>> + Send + '_>>;
    fn validate_input(
        &self,
        input: &Value,
    ) -> Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send + '_>>;
}
```

Same for StatefulHandler, TriggerHandler, ResourceHandler.

### 2.6 ActionError — anyhow removal

```rust
// Before:
pub enum ActionError {
    Retryable { error: Arc<anyhow::Error>, hint: Option<RetryHintCode>, retry_after: Option<Duration> },
    Fatal { error: Arc<anyhow::Error>, hint: Option<RetryHintCode> },
    ...
}

// After:
pub enum ActionError {
    Retryable { error: Arc<dyn std::error::Error + Send + Sync>, hint: Option<RetryHintCode>, retry_after: Option<Duration> },
    Fatal { error: Arc<dyn std::error::Error + Send + Sync>, hint: Option<RetryHintCode> },
    ...
}
```

Constructor methods (`ActionError::retryable(error)`) accept
`impl std::error::Error + Send + Sync + 'static` — same API, no anyhow.

### 2.7 AgentHandler — delete

Remove `AgentHandler` trait and `ActionHandler::Agent` variant from handler.rs.
Add back with engine support when Phase 9 implements. Canon §14 compliance.

### 2.8 InterfaceVersion — receives from core

Per spec 24: `InterfaceVersion` moves from nebula-core to nebula-action.
Already re-exported from `metadata.rs`:
```rust
pub use nebula_core::InterfaceVersion; // currently
```
After spec 24: defined here, not imported:
```rust
// metadata.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InterfaceVersion { pub major: u32, pub minor: u32 }
```

### 2.9 ParameterCollection → Schema

```rust
// metadata.rs
// Before:
pub parameters: ParameterCollection,

// After:
pub input: nebula_schema::Schema,
pub output: OutputSchema,
```

`nebula-parameter` dependency removed, `nebula-schema` added.

### 2.10 Dependency cleanup

```toml
# Cargo.toml changes:
[dependencies]
# REMOVED:
# anyhow
# async-trait
# nebula-credential  (accessor from core now)
# nebula-resource     (accessor from core now)
# nebula-parameter    (replaced by nebula-schema)
# zeroize             (was for CredentialGuard re-export — guard from credential now)

# ADDED:
nebula-schema = { path = "../schema" }

# KEPT:
nebula-action-macros = { path = "macros" }
nebula-core = { path = "../core" }
nebula-error = { workspace = true }
serde, serde_json, thiserror, tokio-util, chrono, tokio, tracing
http, bytes, url  # webhook types
hmac, sha2, hex, base64, subtle  # webhook crypto
parking_lot
```

`nebula-credential` and `nebula-resource` removed — accessor traits come from
`nebula-core`, typed extension traits (`HasResourcesExt`, `HasCredentialsExt`)
come from those crates but are used by action authors via trait imports, not
crate dependencies at the nebula-action level.

**Note:** Action authors' crates still depend on `nebula-resource` and
`nebula-credential` for `Resource`, `Credential` types. But `nebula-action`
itself only needs core accessor traits.

## 3. File changes

| Action | File | Detail |
|---|---|---|
| **Rewrite** | `context.rs` | Delete Context trait + ActionContext/TriggerContext structs. Define ActionContext/TriggerContext umbrella traits + HasNodeIdentity + HasTriggerScheduling |
| **Rewrite** | `capability.rs` | Delete ResourceAccessor + ActionLogger + ActionLogLevel (moved to core). Keep TriggerScheduler + ExecutionEmitter + TriggerHealth. Remove async_trait. |
| **Delete** | `dependency.rs` | Replaced by `nebula_core::DeclaresDependencies` |
| **Update** | `action.rs` | `Action: ActionDependencies` → `Action: DeclaresDependencies` |
| **Update** | `error.rs` | `Arc<anyhow::Error>` → `Arc<dyn Error + Send + Sync>` |
| **Update** | `handler.rs` | Remove AgentHandler, async_trait. Pin<Box<dyn Future>> on handlers. |
| **Update** | `stateless.rs` | `ctx: &impl Context` → `ctx: &(impl ActionContext + ?Sized)`. Remove async_trait from StatelessHandler. |
| **Update** | `stateful.rs` | Same ctx change + handler async_trait removal |
| **Update** | `trigger.rs` | Same ctx → TriggerContext + handler async_trait removal |
| **Update** | `resource.rs` | Same ctx change + handler async_trait removal |
| **Update** | `control.rs` | Same ctx change |
| **Update** | `webhook.rs` | TriggerContext trait |
| **Update** | `poll.rs` | TriggerContext trait |
| **Update** | `metadata.rs` | InterfaceVersion defined here. ParameterCollection → Schema. |
| **Update** | `testing.rs` | Spy types adapt to new traits |
| **Update** | `prelude.rs` | Updated re-exports |
| **Update** | `lib.rs` | Updated re-exports, removed dead re-exports |
| **Update** | `macros.rs` | No change |
| **Update** | `output.rs` | No change |
| **Update** | `result.rs` | No change |
| **Update** | `port.rs` | No change |
| **Update** | `validation.rs` | Uses new trait names |

## 4. Diagnostic attributes (Rust 1.94)

```rust
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as an Action",
    label = "this type does not implement the Action trait",
    note = "derive it: #[derive(Action)]"
)]
pub trait Action: DeclaresDependencies + Send + Sync + 'static { ... }

#[diagnostic::on_unimplemented(
    message = "`{Self}` is missing capabilities for action execution",
    note = "ActionContext requires: Context + HasResources + HasCredentials + HasLogger + HasMetrics + HasEventBus + HasNodeIdentity"
)]
pub trait ActionContext: ... {}

#[diagnostic::on_unimplemented(
    message = "`{Self}` is missing capabilities for trigger execution",
    note = "TriggerContext requires: Context + HasResources + HasCredentials + HasLogger + HasMetrics + HasEventBus + HasTriggerScheduling"
)]
pub trait TriggerContext: ... {}
```

## 5. ActionResult::Retry status

`ActionResult::Retry` variant **stays**. Canon §11.2 status:

| Surface | Status |
|---|---|
| Variant in ActionResult enum | `implemented` (type exists) |
| Engine consuming Retry → re-enqueue with persisted attempts | `planned` (spec 09 R2 layer) |
| Spec 09 implementation PR wires it end-to-end | **prerequisite before using in docs/examples** |

Until spec 09 PR: variant exists but no doc examples show it as current
capability. ActionError::Retryable + nebula-resilience pipeline is the
canonical retry surface.

## 6. Testing criteria

- ActionContext trait: blanket impl works — any type implementing all
  capabilities satisfies ActionContext
- TriggerContext trait: same
- StatelessAction: `execute(&self, input, &impl ActionContext)` compiles
  with both concrete types and trait objects
- ActionDependencies deleted: `Action: DeclaresDependencies` compiles
- Handler traits: Pin<Box<dyn Future>> works for dyn dispatch
- ActionError: constructors accept `impl Error + Send + Sync + 'static`
- AgentHandler removed: ActionHandler enum has 4 variants (not 5)
- `#[diagnostic::on_unimplemented]`: better error messages verified
- No `async_trait` or `anyhow` in compiled output
- All existing tests pass with trait-based context

## 7. Migration path

### PR 1: anyhow + AgentHandler removal

1. Replace `Arc<anyhow::Error>` → `Arc<dyn Error + Send + Sync>` in error.rs
2. Remove AgentHandler + ActionHandler::Agent variant
3. Remove `anyhow` from Cargo.toml
4. All tests green

### PR 2: async_trait removal

1. Replace `#[async_trait]` with `Pin<Box<dyn Future>>` on all handler traits
2. Replace `#[async_trait]` on TriggerScheduler, ExecutionEmitter
3. Remove `async-trait` from Cargo.toml
4. Add `#[diagnostic::on_unimplemented]` on key traits

### PR 3: Context + capability migration

1. Delete `Context` trait from context.rs
2. Define `ActionContext` umbrella trait + blanket impl
3. Define `TriggerContext` umbrella trait + blanket impl
4. Define `HasNodeIdentity` + `HasTriggerScheduling`
5. Delete local ResourceAccessor, ActionLogger, ActionLogLevel from capability.rs
6. Update all action trait signatures: `&impl Context` → `&(impl ActionContext + ?Sized)`
7. Update all trigger signatures: `&TriggerContext` → `&(impl TriggerContext + ?Sized)`
8. Remove `nebula-credential` and `nebula-resource` from Cargo.toml

### PR 4: Dependencies + metadata

1. Delete dependency.rs (ActionDependencies)
2. `Action: DeclaresDependencies` supertrait
3. Update `#[derive(Action)]` macro for DeclaresDependencies generation
4. InterfaceVersion defined locally
5. ParameterCollection → Schema in ActionMetadata
6. Remove `nebula-parameter`, add `nebula-schema` in Cargo.toml

## 8. Open questions

### 8.1 `CredentialGuard` re-export

Currently: `pub use nebula_credential::CredentialGuard;` in lib.rs.
After removing nebula-credential dep — action authors import
`CredentialGuard` from `nebula-credential` directly, or via
`HasCredentialsExt::credential()` return type.

### 8.2 Testing module

`testing.rs` has `TestContextBuilder` that builds `ActionContext` struct.
After ActionContext becomes trait — `TestContextBuilder` builds
`TestActionContext` (concrete struct in this crate for testing, or moves
to `nebula-testing` per spec 20). Deferred to spec 20 implementation.

### 8.3 Handler trait ctx type

Handler traits (dyn-safe) need concrete ctx type:

```rust
pub trait StatelessHandler: Send + Sync {
    fn execute(&self, input: Value, ctx: &dyn ActionContext)
        -> Pin<Box<dyn Future<...> + Send + '_>>;
}
```

`&dyn ActionContext` works because ActionContext is a supertrait-bound
trait (all methods from Context + HasResources + ... are available via
vtable). Verify dyn-safety of all capability traits.

---

## 9. Action type evolution (Q&A additions)

Additions from expert Q&A on action type family. These are NEW types and
redesigns beyond the structural spec 23/24 alignment in §§1-8 above.

### 9.0 Crash recovery + idempotency (Q&A Q1)

**Type-aware recovery:** StatelessAction re-executes from start (idempotency
key guards side effects). StatefulAction resumes from last checkpoint.
TriggerAction restarts (start() called again). Max 3 consecutive orphans →
permanent Orphaned status (spec 17).

**Engine-managed iteration counter** for StatefulAction:
- Engine auto-tracks `iteration: u32` (0, 1, 2, ...)
- Full key: `{exe_id}:{node_key}:{iteration}:{attempt_id}`
- Engine checks + commits automatically on each step
- Engine validates `hash(state_before) != hash(state_after)` on Continue
  → catches stuck state / infinite loops

**Optional business idempotency key** on StatefulAction:

```rust
pub trait StatefulAction: Action {
    // ...
    /// Optional business key for external dedup (payments, invoices).
    /// Engine builds: {exe}:{node}:{iteration}:{THIS}:{attempt}
    /// Default: None — engine uses iteration counter alone.
    fn idempotency_key(&self, _state: &Self::State) -> Option<String> {
        None
    }
}
```

### 9.1 Per-type result enums (replaces unified ActionResult)

`ActionResult<T>` (11 variants) splits into per-type results. Authors see
only variants relevant to their action type. Engine-internal `FlowDirective`
unifies for dispatch.

```rust
/// StatelessAction returns this (not ActionResult):
pub enum StatelessResult<T> {
    Success(ActionOutput<T>),
    Skip { reason: String },
    Drop { reason: Option<String> },
}

/// StatefulAction returns this:
pub enum StatefulResult<T> {
    Continue { output: ActionOutput<T>, progress: Option<f64>, delay: Option<Duration> },
    Done { output: ActionOutput<T> },
    Abort { output: Option<ActionOutput<T>>, reason: String },
}

/// ControlAction: ControlOutcome (already exists, no change)
```

**StatefulResult change:** `Break { reason: BreakReason }` → split into
`Done` (successful completion) + `Abort` (intentional early stop). Clearer
semantics for pagination (Done = last page) vs search (Abort = found target).

`ActionResult<T>` retained as deprecated alias or engine-internal
`FlowDirective` for unified dispatch.

### 9.2 EventTrigger — event subscriber (Kafka, Redis Pub/Sub, AMQP, NATS)

DX over TriggerAction for event-driven triggers. Author writes `on_event()`.
Engine handles: resource acquisition, reconnect with backoff, emit, cancellation.

```rust
pub trait EventTrigger: Action {
    type Source: Resource;
    type Event: Serialize + DeserializeOwned + Send + Sync;

    fn on_event(
        &self,
        source: &<Self::Source as Resource>::Lease,
        ctx: &(impl TriggerContext + ?Sized),
    ) -> impl Future<Output = Result<EventOutcome<Self::Event>, ActionError>> + Send;

    fn on_error(&self, _error: ActionError) -> EventErrorAction {
        EventErrorAction::Reconnect
    }

    fn ack_strategy(&self) -> AckStrategy { AckStrategy::AfterEmit }
}

pub enum EventOutcome<E> {
    /// Emit event → start workflow execution.
    Emit(E),
    /// Skip message (ack — processed, just filtered).
    Skip,
    /// Reject message (nack — redelivery or dead-letter).
    Reject { reason: String },
}

pub enum EventErrorAction { Reconnect, Stop, Skip }

pub enum AckStrategy {
    /// Commit after successful emit (at-least-once).
    AfterEmit,
    /// Commit immediately (at-most-once).
    Immediate,
    /// Manual — author calls ctx.ack().
    Manual,
}
```

### 9.3 AgentAction — autonomous AI agent

LLM-driven think → act → observe loop. Agent chooses tools dynamically.

```rust
pub trait AgentAction: Action {
    type Input: Send + Sync;
    type Output: Send + Sync;

    fn think(
        &self,
        input: &Self::Input,
        observations: &[Observation],
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<AgentStep<Self::Output>, ActionError>> + Send;

    fn max_iterations(&self) -> u32 { 10 }
}

pub enum AgentStep<O> {
    /// Call one or more tools (parallel execution).
    ToolCalls(Vec<ToolCall>),
    /// Final answer.
    Answer(O),
    /// Delegate to another agent.
    Delegate { agent_key: ActionKey, input: serde_json::Value },
}

pub struct ToolCall {
    pub tool_name: String,
    pub tool_input: serde_json::Value,
}

pub struct Observation {
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub tool_output: serde_json::Value,
    pub duration: Duration,
}
```

### 9.4 AgentTool — standalone tool (MCP-style JSON protocol)

```rust
pub trait AgentTool: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Schema;

    fn execute(
        &self,
        input: serde_json::Value,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<serde_json::Value, ActionError>> + Send;
}
```

JSON in, JSON out. Agent sees name + description + schema. Like MCP tools.

### 9.5 ToolProvider on Resource — resource-bundled tools

Resource declares tools that work with its Lease. User connects resource
to agent → agent automatically sees resource's tools. Spec 25 addition.

```rust
/// Optional: Resource provides tools for AI agent use.
pub trait ToolProvider: Resource {
    fn tool_defs() -> Vec<ToolDef> where Self: Sized;

    fn call_tool(
        name: &str,
        input: serde_json::Value,
        lease: &Self::Lease,
    ) -> impl Future<Output = Result<serde_json::Value, ActionError>> + Send
    where Self: Sized;
}

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Schema,
}
```

`ResourceMetadata` gains `tools: Vec<ToolDef>` — populated from
`ToolProvider::tool_defs()` at registration. UI shows available tools.

Engine flow: agent calls tool by name → engine acquires resource lease →
`ToolProvider::call_tool(name, input, &lease)` → typed, safe.

Two tool sources for agents:
- `AgentTool` — standalone tools (web search, calculator)
- `ToolProvider` — resource-bundled (sql_query, send_message)

### 9.6 Streaming pipeline — StreamSource / StreamStage / StreamSink

Composable streaming inside a StreamNode (composite node in editor).

```rust
pub trait StreamSource: Action {
    type Out: Send + 'static;
    fn next(&self, ctx: &(impl ActionContext + ?Sized))
        -> impl Future<Output = Result<Option<Self::Out>, ActionError>> + Send;
}

pub trait StreamStage: Action {
    type In: Send + 'static;
    type Out: Send + 'static;

    /// Transform one item into zero or more outputs (filter/map/fan-out).
    fn transform(&self, item: Self::In, ctx: &(impl ActionContext + ?Sized))
        -> impl Future<Output = Result<Vec<Self::Out>, ActionError>> + Send;

    fn open(&self, _ctx: &(impl ActionContext + ?Sized))
        -> impl Future<Output = Result<(), ActionError>> + Send { async { Ok(()) } }
    fn close(&self, _ctx: &(impl ActionContext + ?Sized))
        -> impl Future<Output = Result<(), ActionError>> + Send { async { Ok(()) } }
}

pub trait StreamSink: Action {
    type In: Send + 'static;

    fn consume(&self, item: Self::In, ctx: &(impl ActionContext + ?Sized))
        -> impl Future<Output = Result<(), ActionError>> + Send;
    fn consume_batch(&self, items: Vec<Self::In>, ctx: &(impl ActionContext + ?Sized))
        -> impl Future<Output = Result<(), ActionError>> + Send {
        async { for item in items { self.consume(item, ctx).await?; } Ok(()) }
    }
    fn flush(&self, _ctx: &(impl ActionContext + ?Sized))
        -> impl Future<Output = Result<(), ActionError>> + Send { async { Ok(()) } }
}

pub struct StreamPipeline {
    pub backpressure: BackpressureStrategy,
    pub buffer_size: usize,
    pub max_items: u64,
    pub timeout: Option<Duration>,
}

pub enum BackpressureStrategy { Wait, DropOldest, DropNewest, Adaptive { target_latency: Duration } }
```

Stages connected via SupportPort auxiliary connections in editor.
StreamNode = composite node, internally runs Source → Stage* → Sink.

### 9.7 AwaitAction — universal "do and wait" (replaces InteractiveAction)

Universal pattern for: sub-workflow execution, human approval, webhook callback,
external job, payment confirmation — anything "initiate work, suspend, resume
when result arrives."

Replaces the narrower `InteractiveAction` (human-only) AND eliminates the need
for a separate `SubWorkflowAction` trait. Engine persists suspension state,
frees resources, resumes node on signal. Crash-safe: `Request` serialized
in DB, recovery re-registers wait condition.

```rust
pub trait AwaitAction: Action {
    type Input: Send + Sync;
    type Output: Send + Sync;
    /// What to persist during wait (child exe_id, approval form, callback_id).
    type Request: Serialize + DeserializeOwned + Send + Sync;
    /// What comes back (child output, approval decision, callback payload).
    type Response: DeserializeOwned + Send + Sync;

    /// Initiate work and declare what we're waiting for.
    /// Engine persists Request + condition, then suspends node.
    fn initiate(
        &self,
        input: Self::Input,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<Suspension<Self::Request>, ActionError>> + Send;

    /// Process response when signal arrives. May return final output
    /// or another suspension (multi-step: approval chains, retries).
    fn resume(
        &self,
        request: Self::Request,
        response: Self::Response,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<ResumeOutcome<Self::Output, Self::Request>, ActionError>> + Send;

    fn timeout(&self) -> Duration { Duration::from_secs(86400) }

    fn on_timeout(&self, _request: Self::Request) -> Result<Self::Output, ActionError> {
        Err(ActionError::fatal("await timed out"))
    }
}

pub struct Suspension<R> {
    pub request: R,
    pub condition: WaitCondition,
    pub display: Option<serde_json::Value>,
}

pub enum ResumeOutcome<O, R> {
    Done(ActionOutput<O>),
    Suspend(Suspension<R>),
}
```

**Use cases implemented via AwaitAction:**

| Use case | Request type | Response type | Wait condition |
|---|---|---|---|
| Sub-workflow | `ExecutionId` | `Value` (child output) | `Signal("execution_completed:{id}")` |
| Approval | `ApprovalForm` | `ApprovalDecision` | `Signal("approval:{node_key}")` |
| Webhook callback | `CallbackId` | `Value` (payload) | `Signal("callback:{id}")` |
| External ML job | `JobId` | `MLResult` | `Signal("job_completed:{id}")` |
| Payment confirmation | `PaymentRef` | `PaymentStatus` | `SignalOrTimer("payment:{id}", 30min)` |

All built-in implementations ship as actions in `nebula-plugin-core`, not
as framework traits. `AwaitAction` is the only trait in the framework.
```

### 9.8 TransactionalAction — saga/compensation

```rust
pub trait TransactionalAction: Action {
    type Input: Send + Sync;
    type Output: Send + Sync;
    type CompensationData: Serialize + DeserializeOwned + Send + Sync;

    fn execute(&self, input: Self::Input, ctx: &(impl ActionContext + ?Sized))
        -> impl Future<Output = Result<TransactionResult<Self::Output, Self::CompensationData>, ActionError>> + Send;

    fn compensate(&self, data: Self::CompensationData, ctx: &(impl ActionContext + ?Sized))
        -> impl Future<Output = Result<(), ActionError>> + Send;
}
```

### 9.9 Updated ActionHandler enum

```rust
pub enum ActionHandler {
    Stateless(Arc<dyn StatelessHandler>),
    Stateful(Arc<dyn StatefulHandler>),
    Trigger(Arc<dyn TriggerHandler>),
    Resource(Arc<dyn ResourceHandler>),
    Control(Arc<dyn StatelessHandler>),   // ControlAction adapts to StatelessHandler
    Agent(Arc<dyn AgentHandler>),          // NEW (was removed as stub, now real)
    Transactional(Arc<dyn TransactionalHandler>),  // NEW
    Stream(Arc<dyn StreamHandler>),        // NEW
    Await(Arc<dyn AwaitHandler>),                    // NEW (replaces Interactive)
}
```

### 9.10 Summary — final action type family

**Core traits (7):**

| Trait | Result | Use case |
|---|---|---|
| `StatelessAction` | `StatelessResult<O>` | Pure function, API calls |
| `StatefulAction` | `StatefulResult<O>` | Pagination, batch, loops |
| `TriggerAction` | start/stop | Workflow starters |
| `ResourceAction` | configure/cleanup | Scoped DI |
| `ControlAction` | `ControlOutcome` | If/Switch/Router/Filter |
| `AgentAction` | `AgentStep<O>` | AI agent, tool loop |
| `TransactionalAction` | execute/compensate | Saga, payments |

**DX wrappers (5):**

| Trait | Over | DX |
|---|---|---|
| `PaginatedAction` | StatefulAction | `fetch_page(cursor)` |
| `BatchAction` | StatefulAction | `process_batch(chunk)` |
| `WebhookAction` | TriggerAction | HTTP verification |
| `PollAction` | TriggerAction | Interval + cursor |
| `EventTrigger` | TriggerAction | Event subscriber + ack |

**Streaming (3):**

| Trait | Role |
|---|---|
| `StreamSource` | Produces items |
| `StreamStage` | Transform (filter/map/fan-out) |
| `StreamSink` | Consume + batch + flush |

**Await (1 — universal "do and wait"):**

| Trait | Role |
|---|---|
| `AwaitAction` | Sub-workflow, approval, webhook callback, external jobs, payments |

**Agent tools (2):**

| Trait | Role |
|---|---|
| `AgentTool` | Standalone tool (JSON protocol) |
| `ToolProvider` | Resource-bundled tools (typed lease access) |
