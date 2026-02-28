# nebula-action

Core action system. Defines all traits that workflow nodes implement.

## Trait Hierarchy

```
Action                  (base: metadata only)
├── StatelessAction     (Input → Output, pure function)
├── StatefulAction      (persistent state between calls)
│   ├── TriggerAction   (workflow starter, lives outside the DAG)
│   ├── InteractiveAction   [DX] human-in-the-loop
│   └── TransactionalAction [DX] Saga / compensation
└── ResourceAction      (injects a resource into downstream nodes)
    └── (TriggerAction extends StatefulAction)
        ├── WebhookAction   [DX] inbound HTTP webhook
        └── PollAction      [DX] cursor-based polling
```

The engine works with the four **core types**. The DX types are convenience wrappers — an
experienced developer can always implement the same behaviour manually via the core traits.

## Core Types

### `Action` — base trait

Metadata only. Does not define execution behaviour.

```rust
pub trait Action: Send + Sync {
    fn metadata(&self) -> &ActionMetadata;
}
```

---

### `StatelessAction` — pure function

No state. Can execute in parallel without coordination. The most common type.

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

**`ActionResult` variants:** `Success`, `Skip`, `Branch`, `Route`, `MultiOutput`, `Retry`.

**Use when:** data transforms, API calls, validation, content generation.

---

### `StatefulAction` — iterative processing

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

    async fn initialize_state(&self, input: &Self::Input, ctx: &ActionContext) -> Result<Self::State> {
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

**Additional `ActionResult` variants:** `Continue { delay }`, `Break { reason }`, `Wait { condition }`.

`Wait` suspends execution until an external event — human approval, HTTP callback — without
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

### `TriggerAction` — workflow starter

Extends `StatefulAction`. Lives outside the execution DAG and *spawns* executions.
Managed by `TriggerManager` independently of `WorkflowEngine`.

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

`TriggerContext` is not bound to a specific execution — the trigger lives at the workflow
level.

**Use when:** any external event source that initiates a workflow.

---

### `ResourceAction` — dependency injection via the graph

Provides a capability (resource, service, tool) to its downstream nodes. The engine
manages the lifecycle explicitly:

1. Calls `ResourceAction::configure()` **before** downstream nodes execute.
2. Creates a `Resource::Instance` via `nebula-resource` (with pooling and health checks).
3. Makes it available downstream via `ctx.resource()`.
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

**Why it's a core type (not DX):**
- **Execution order** — the engine has explicit topological knowledge that `ResourceAction`
  must precede its downstream nodes.
- **Scoped lifecycle** — the resource lives only while the downstream branch executes, then
  `cleanup` is called. Not global.
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
`ctx.resource()` = global access to the resource registry in `nebula-resource`.

**Use when:** providing a DB connection pool, injecting AI tools into an agent node,
configuring a credentialed HTTP client for a specific branch.

## DX Types

Convenience wrappers. The engine uses the underlying core trait — the DX layer just removes
boilerplate. Any DX type can be implemented manually via its corresponding core trait.

```
StatefulAction
├── InteractiveAction    — Wait{Approval/Webhook} + declarative UI API
└── TransactionalAction  — Saga pattern + compensation boilerplate

TriggerAction
├── WebhookAction        — endpoint registration + signature verification
└── PollAction           — cursor management + interval scheduling
```

### `InteractiveAction`

Human-in-the-loop pattern. Declarative API over `ActionResult::Wait`.

Two patterns of human participation:

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

### `TransactionalAction`

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
|------|-----------|
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

### `WebhookAction`

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

### `PollAction`

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

## Resource Consumption

All action types consume resources via `ActionContext` — orthogonal to the trait hierarchy:

```rust
// Available in any action type
let db    = ctx.resource::<DatabaseResource>().await?;
let cache = ctx.resource::<CacheResource>().await?;
```

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
│
└── no → StatelessAction
```

## Summary Table

| Type | Core/DX | State | Extends | Purpose |
|------|---------|-------|---------|---------|
| `StatelessAction` | Core | ❌ | `Action` | Input → Output, pure function |
| `StatefulAction` | Core | ✅ | `Action` | Iterative processing with state |
| `TriggerAction` | Core | ✅ | `StatefulAction` | Workflow starter |
| `ResourceAction` | Core | ❌ | `Action` | Injects resource into downstream nodes |
| `InteractiveAction` | DX | ✅ | `StatefulAction` | Human-in-the-loop, approvals |
| `TransactionalAction` | DX | ✅ | `StatefulAction` | Saga / compensation |
| `WebhookAction` | DX | ✅ | `TriggerAction` | Inbound HTTP webhook |
| `PollAction` | DX | ✅ | `TriggerAction` | Cursor-based source polling |
