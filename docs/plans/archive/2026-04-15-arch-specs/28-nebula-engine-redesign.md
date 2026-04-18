# 28 — `nebula-engine` redesign (absorbs `nebula-runtime`)

> **Status:** DRAFT
> **Authority:** subordinate to `docs/PRODUCT_CANON.md`. Canon wins on conflict.
> **Parent:** [`./README.md`](./README.md)
> **Scope:** Redesign execution layer as two crates: `nebula-execution` (pure
> types, unchanged) + `nebula-engine` (absorbs `nebula-runtime`). Integrate
> specs 08/09/14/17/23 decisions. Port-driven routing, crash recovery,
> expression resolution, event system, trigger management.
> **Depends on:** 08 (cancellation), 09 (retry), 10 (timeouts), 14 (stateful),
> 17 (multi-process), 23 (context), 24 (core), 25 (resource), 26 (credential),
> 27 (action types)
> **Consumers:** `nebula-api`, `nebula-testing`

## 1. Problem

Execution layer is split across three crates with unclear boundaries:

- **`nebula-execution`** (13 files) — pure data types: ExecutionStatus, State,
  Plan, Journal, IdempotencyKey. No async deps. Used by storage. **Well-designed.**
- **`nebula-runtime`** (9 files) — action dispatch, sandbox bridge, registry,
  checkpoint, blob. Depends on nebula-action, nebula-sandbox.
- **`nebula-engine`** (9 files) — DAG orchestration, frontier-based execution.
  Depends on runtime + 10 other crates.

**Engine depends on runtime** (one-way). They share responsibility "execute
workflows" at different granularity. Splitting them creates artificial boundary:
- `ActionRuntimeContext` (spec 23) bridges both
- Checkpoint management spans both
- Retry accounting spans both

### 1.1 Additional issues

- **EdgeCondition + FlowKind::Error** — duplicate error routing mechanisms
  (accidental, not intentional design)
- **No crash recovery contract** — what happens to running nodes on process death
- **Expression resolution** — unclear who resolves params (engine vs action)
- **Trigger management** — trigger lifecycle mixed with execution concerns
- **Event system** — ad-hoc mpsc channel, should use nebula-eventbus

## 2. Decision

### 2.1 Two crates (first principles)

**`nebula-execution`** — STAYS SEPARATE. Pure types, no async. Used by storage.

**`nebula-engine`** — ABSORBS `nebula-runtime`. One crate for all operational
execution concerns: orchestration + dispatch + context + durability + coordination.

**`nebula-runtime`** — DELETED. Files move into engine.

**`nebula-sandbox`** — STAYS SEPARATE. Isolation = orthogonal concern.

### 2.2 Port-driven routing (replaces EdgeCondition)

**Delete `EdgeCondition`** from nebula-workflow. Error routing via `OutputPort::Error`
port + explicit ControlAction nodes. Zero invisible logic on edges.

```
// Before (edge conditions — invisible):
[B] ──OnSuccess──→ [C]
[B] ──OnError{Transient}──→ [RetryHandler]

// After (port-driven — visual):
[B] ──out──→ [C]
[B] ──error──→ [ErrorRouter] ──transient──→ [RetryHandler]
                              ──permanent──→ [AlertHandler]
```

Edges = simple wires from port to port. No conditions. All routing via
explicit nodes (If, Switch, Router, ErrorRouter, Filter).

### 2.3 Engine-resolved expressions

Engine resolves ALL expressions in node params before dispatch. Actions
receive pure `serde_json::Value` — zero expression awareness.

```
1. All predecessors completed → outputs in storage
2. Engine builds EvalContext { nodes: outputs, execution, vars, env }
3. Engine evaluates all ${...} expressions in node params
4. Resolved Value passed as action input
5. Action: pure business logic
```

## 3. Target module structure

```
nebula-engine/src/
├── lib.rs
│
├── orchestrator/              ← DAG execution (from engine)
│   ├── mod.rs                 (WorkflowEngine — frontier-based scheduler)
│   ├── frontier.rs            (ready-node detection, edge activation)
│   └── resolver.rs            (expression/param resolution via nebula-expression)
│
├── dispatch/                  ← Action execution (from runtime)
│   ├── mod.rs                 (ActionDispatcher — sandbox bridge, timeout)
│   ├── registry.rs            (ActionRegistry — handler lookup)
│   ├── sandbox.rs             (SandboxRunner integration)
│   └── data_policy.rs         (DataPassingPolicy — output size enforcement)
│
├── context/                   ← Spec 23 runtime contexts
│   ├── mod.rs
│   ├── action.rs              (ActionRuntimeContext: impl ActionContext)
│   ├── trigger.rs             (TriggerRuntimeContext: impl TriggerContext)
│   └── accessor.rs            (EngineResourceAccessor, EngineCredentialAccessor)
│
├── durability/                ← Persistence + idempotency
│   ├── checkpoint.rs          (write-behind buffer — spec 14 pattern)
│   ├── idempotency.rs         (engine-managed counter + optional business key)
│   └── output_buffer.rs       (node output write-behind — flush triggers)
│
├── trigger.rs                 (TriggerManager — generic lifecycle, one file)
│
├── event.rs                   (ExecutionEvent via nebula-eventbus)
├── error.rs                   (EngineError — unified, absorbs RuntimeError)
├── blob.rs                    (BlobStorage for large data — from runtime)
└── stream.rs                  (BoundedStreamBuffer — from runtime)
```

~20 files. Clear internal boundaries. One crate, one compile unit.

## 4. Crash recovery (Q1)

### 4.1 Type-aware recovery

Engine detects orphaned nodes (claimed_until expired, node still "running"):

| Action type | Recovery |
|---|---|
| **StatelessAction** | Re-execute from start. New attempt_id. Idempotency key guards side effects. |
| **StatefulAction** | Resume from last checkpoint. New attempt_id. Checkpoint has iteration + state. |
| **TriggerAction** | Restart trigger (start() called again). |
| **ResourceAction** | Re-run configure(). Scoped resource recreated. |
| **AwaitAction** | Check suspension state in DB. If waiting → re-register wait condition. If signal arrived during crash → resume(). |

### 4.2 Idempotency — engine-managed

StatefulAction idempotency is automatic:

```
Engine tracks: iteration counter (0, 1, 2, ...)
Full key: {exe_id}:{node_key}:{iteration}:{attempt_id}
Optional: developer provides business segment via idempotency_key(state)
Full key with business: {exe_id}:{node_key}:{iteration}:{biz_key}:{attempt_id}
```

Engine on each StatefulAction step:
1. Build key from iteration counter (+ optional business key)
2. Check if committed → skip if yes
3. Execute action
4. Validate `hash(state_before) != hash(state_after)` → catch stuck loops
5. Commit key + persist state
6. Increment iteration

StatelessAction: key = `{exe_id}:{node_key}:{attempt_id}`. Automatic, zero developer input.

### 4.3 Bounded retries on orphan

Spec 17: "Orphaned after 3 crashes." Engine tracks consecutive orphan count
per node. Third orphan → permanent `Orphaned` status, no more retries.

## 5. Cancellation (Q4 — spec 08)

Two-tier model:

**In-process (trusted actions):**
1. `CancellationToken` fires
2. Grace period (spec 08: 30s default per node)
3. Grace exceeded → `JoinHandle::abort()` — Future dropped at next `.await`
4. Rust Drop runs destructors (connections closed, guards released)
5. Node status = `Terminated(GraceExceeded)`

**Process sandbox (untrusted):**
1. IPC cancel message
2. Grace period
3. SIGTERM
4. SIGTERM grace
5. SIGKILL

**Cancel vs Terminate (spec 08 — two RBAC'd actions):**
- Cancel = full grace period
- Terminate = zero grace → immediate abort/SIGKILL

## 6. Event system (Q8)

Engine emits `ExecutionEvent` via `nebula-eventbus`:

```rust
pub enum ExecutionEvent {
    ExecutionStarted { exe_id, workflow_id, version_id },
    ExecutionCompleted { exe_id, status, duration },
    NodeDispatched { exe_id, node_key, attempt_id },
    NodeCompleted { exe_id, node_key, attempt_id, duration },
    NodeFailed { exe_id, node_key, attempt_id, error },
    NodeOrphaned { exe_id, node_key, reason },
    NodeSuspended { exe_id, node_key, condition },
    NodeResumed { exe_id, node_key },
    CheckpointFlushed { exe_id, nodes_count },
    TriggerStarted { trigger_id, workflow_id },
    TriggerStopped { trigger_id },
    TriggerFired { trigger_id, exe_id },
}
```

Spec 18 subscribers: storage writer, metrics collector, websocket
broadcaster, audit writer. Each gets own `broadcast::Receiver`.
Slow subscriber → backpressure policy, never blocks engine.

## 7. Trigger management (Q9)

One generic `TriggerManager` — doesn't know cron/webhook/poll specifics:

```rust
pub struct TriggerManager {
    running: HashMap<TriggerId, RunningTrigger>,
    lifecycle: LayerLifecycle,
}

impl TriggerManager {
    pub async fn register(&mut self, id: TriggerId, handler: Arc<dyn TriggerHandler>) { ... }
    pub async fn deregister(&mut self, id: &TriggerId) { ... }
    pub async fn shutdown_all(&mut self) { ... }
}
```

Trigger-specific logic (cron scheduling, webhook endpoints, poll loops,
event subscriptions) lives in action impls (plugin crates), not engine.

## 8. Workflow version loading (Q10)

V1: load `workflow_version` from DB at execution start. No cache.
V2: add cache with publish-event invalidation.

Trigger firing: reads latest Published version at claim time (spec 13).

## 9. Port-driven routing details

### 9.1 Edge model (replaces EdgeCondition)

```rust
/// A connection between two ports in the workflow graph.
/// NO conditions — routing via explicit nodes.
pub struct Connection {
    pub from_node: NodeKey,
    pub from_port: PortKey,   // "out", "error", "rule_0", etc.
    pub to_node: NodeKey,
    pub to_port: PortKey,     // "in"
}
```

### 9.2 Engine dispatch per port kind

```
Node completes:
  Success → data sent through Main port connections
  Error   → error data sent through Error port connections
  
  If no Error port connections → error propagates to execution level
  (same as current behavior when no OnError edge exists)
```

### 9.3 ErrorRouter — built-in ControlAction

```rust
// nebula-plugin-core
struct ErrorRouter;

impl ControlAction for ErrorRouter {
    async fn evaluate(&self, input: ControlInput, _ctx: &ActionContext)
        -> Result<ControlOutcome>
    {
        let error = input.get::<ActionError>()?;
        let port = match error.classify().category() {
            ErrorCategory::External => "transient",
            ErrorCategory::Internal => "permanent",
            ErrorCategory::Cancelled => "cancelled",
            _ => "other",
        };
        Ok(ControlOutcome::Route { port: port.into(), output: input.into_value() })
    }
}
```

Explicit node in DAG. Visible in editor. Debuggable in journal.

### 9.4 Migration from EdgeCondition

| EdgeCondition | Port-driven equivalent |
|---|---|
| `Always` | Default — wire from Main port |
| `OnSuccess` | Wire from Main port (same as Always when no Error port) |
| `OnError { Any }` | Wire from Error port |
| `OnError { matcher }` | Error port → ErrorRouter node → typed output ports |
| `When { expression }` | If/Switch/Filter ControlAction node |

## 10. Files absorbed from nebula-runtime

| Runtime file | Engine destination | Changes |
|---|---|---|
| `runtime.rs` | `dispatch/mod.rs` | ActionRuntime → ActionDispatcher |
| `registry.rs` | `dispatch/registry.rs` | ActionRegistry unchanged |
| `sandbox.rs` | `dispatch/sandbox.rs` | SandboxRunner, InProcessSandbox unchanged |
| `data_policy.rs` | `dispatch/data_policy.rs` | DataPassingPolicy unchanged |
| `blob.rs` | `blob.rs` | BlobStorage unchanged |
| `stream_backpressure.rs` | `stream.rs` | BoundedStreamBuffer unchanged |
| `queue.rs` | Absorbed into orchestrator | MemoryQueue → internal |
| `error.rs` | Merged into `error.rs` | RuntimeError variants → EngineError |

## 11. Cargo.toml (target)

```toml
[package]
name = "nebula-engine"
description = "Workflow execution engine for Nebula"

[dependencies]
# Nebula ecosystem
nebula-core = { path = "../core" }
nebula-error = { workspace = true }
nebula-action = { path = "../action" }
nebula-expression = { path = "../expression" }
nebula-plugin = { path = "../plugin" }
nebula-workflow = { path = "../workflow" }
nebula-execution = { path = "../execution" }
nebula-credential = { path = "../credential" }
nebula-resource = { path = "../resource" }
nebula-resilience = { path = "../resilience" }
nebula-sandbox = { path = "../sandbox" }
nebula-eventbus = { path = "../eventbus" }
nebula-metrics = { path = "../metrics" }
nebula-telemetry = { path = "../telemetry" }

# Async
tokio = { workspace = true, features = ["rt", "sync", "time", "macros"] }
tokio-util = { workspace = true }

# Core
dashmap = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
chrono = { workspace = true }

# REMOVED: nebula-runtime (absorbed)
```

## 12. Spec 23 context integration

Engine owns concrete context implementations:

```rust
// context/action.rs
pub struct ActionRuntimeContext {
    base: BaseContext,
    resources: Arc<EngineResourceAccessor>,    // scoped + global resolution
    credentials: Arc<EngineCredentialAccessor>,
    logger: Arc<dyn Logger>,
    metrics: Arc<dyn MetricsEmitter>,
    eventbus: Arc<dyn EventEmitter>,
    node_key: NodeKey,
    attempt_id: AttemptId,
}

impl nebula_core::Context for ActionRuntimeContext { /* delegate base */ }
impl HasResources for ActionRuntimeContext { /* return resources */ }
impl HasCredentials for ActionRuntimeContext { /* return credentials */ }
impl HasLogger for ActionRuntimeContext { /* return logger */ }
impl HasMetrics for ActionRuntimeContext { /* return metrics */ }
impl HasEventBus for ActionRuntimeContext { /* return eventbus */ }
impl HasNodeIdentity for ActionRuntimeContext {
    fn node_key(&self) -> &NodeKey { &self.node_key }
    fn attempt_id(&self) -> &AttemptId { &self.attempt_id }
}
// → blanket impl ActionContext satisfied ✓
```

Same pattern for `TriggerRuntimeContext` (HasTriggerScheduling instead of
HasNodeIdentity).

## 13. Testing criteria

- Frontier-based scheduler: nodes dispatch when ALL predecessors resolved
- Port-driven: Main port → success path, Error port → error path
- No EdgeCondition in any test
- ErrorRouter: routes Transient/Permanent/Cancelled correctly
- Expression resolution: engine resolves before dispatch, action sees pure Value
- Crash recovery: StatelessAction re-executes, StatefulAction resumes checkpoint
- Idempotency: engine-managed counter prevents duplicate execution
- Stuck detection: state hash unchanged after Continue → error
- Cancel: token → grace → abort. Terminate: immediate abort
- TriggerManager: register/deregister/shutdown generic lifecycle
- EventBus: ExecutionEvent emitted, subscribers receive independently
- ActionRuntimeContext: satisfies ActionContext blanket impl
- All absorbed runtime tests pass in new location

## 14. Migration path

### PR 1: Absorb runtime into engine

1. Move runtime files → engine (dispatch/, blob.rs, stream.rs)
2. Merge RuntimeError into EngineError
3. Update engine Cargo.toml (add runtime deps, remove nebula-runtime)
4. Delete nebula-runtime crate from workspace
5. Fix all workspace imports
6. All tests green

### PR 2: Port-driven routing

1. Remove EdgeCondition from nebula-workflow
2. Update Connection struct (no condition field)
3. Update engine frontier/edge evaluation — port-based dispatch
4. Add ErrorRouter built-in ControlAction
5. Migrate existing workflow tests

### PR 3: Context + event system

1. Add context/ module (ActionRuntimeContext, TriggerRuntimeContext)
2. Implement all HasX capability traits
3. Switch from mpsc to nebula-eventbus for ExecutionEvent
4. Add trigger.rs (TriggerManager)

### PR 4: Crash recovery + idempotency

1. Add durability/ module (checkpoint, idempotency, output_buffer)
2. Engine-managed iteration counter for StatefulAction
3. State hash validation (stuck detection)
4. Orphan detection + type-aware recovery
5. Integration tests: crash → recovery → correct state

## 15. Open questions

### 15.1 EngineResourceAccessor — scoped resolution

Scoped resources (plan 10) + global resources → EngineResourceAccessor
resolves: scoped first → global fallback → not found. Implementation
detail deferred to PR.

### 15.2 Retry R2 wiring

Spec 09 R2 (engine-level retry with persisted attempts) needs
integration with dispatch + durability modules. Separate spec 09
implementation PR.

### 15.3 Multi-process coordination

Spec 17 claim/lease/heartbeat management. Implementation lives in
engine/coordination/ but details deferred to spec 17 implementation PR.

### 15.4 nebula-workflow EdgeCondition removal

Removing EdgeCondition from nebula-workflow is a breaking change to that
crate. Coordinate PR 2 with workflow crate maintainer (= you). Connection
struct simplification + all workflow tests updated.
