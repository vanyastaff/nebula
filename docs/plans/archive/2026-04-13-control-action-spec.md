# ControlAction — Design Specification

> DX family for flow-control nodes (If, Switch, Router, Filter, NoOp, Stop, Fail).
> Adapter pattern over `StatelessHandler`, mirroring `PollTriggerAdapter` / `WebhookTriggerAdapter`.

**Date:** 2026-04-13
**Status:** Draft
**Crate:** `nebula-action` (trait surface). Concrete nodes live in a separate downstream crate — exact name/location TBD, not part of this spec.
**Depends on:** prerequisites in `nebula-action` and `nebula-execution` (see §4)
**Depended by:** the downstream crate hosting concrete nodes, community plugin crates, workflow editor (UI)
**Related decision:** `.project/context/decisions.md` — "ControlAction — adapter pattern, not blanket impl, not macro"

---

## 1. Goals & Philosophy

### What ControlAction IS

- **A public DX trait** for flow-control nodes that make synchronous decisions on a single input — branching, filtering, terminating. Implemented by both Nebula-provided core plugins and third-party community crates.
- **Adapter-erased to `StatelessHandler`.** A `ControlActionAdapter<A>` wraps a typed `ControlAction` and implements `StatelessHandler`. Engine, runtime, and `ActionHandler` enum are untouched — adapter is the bridge between the author-facing DX trait and the dyn-compat handler contract.
- **Type-safe public contract via `ControlOutcome`.** The enum exposes exactly the return shapes that make sense for flow control (`Branch`, `Route`, `Pass`, `Drop`, `Terminate`) and hides the broader `ActionResult` surface (`Wait`, `Retry`, `Continue`, `Break`) which is meaningless for stateless control nodes.
- **Trait surface only in `nebula-action`.** Concrete 7 nodes (If, Switch, Router, Filter, NoOp, Stop, Fail) live in a separate downstream crate (name and location TBD — packaging decision, not a trait-contract decision). `nebula-action` stays pure contract.

### What ControlAction is NOT

- **Not a new core trait.** It does not add a sixth variant to `ActionHandler` enum. Engine dispatch is unchanged — control nodes are executed through the same `StatelessHandler::execute` path as any other stateless action.
- **Not a macro.** A declarative macro would prevent community crates from building new control primitives on the abstraction. Community extensibility is a requirement, not a nice-to-have.
- **Not sealed.** The trait is public and openly implementable. Sealing would reproduce the macro's limitation (closed set of implementors).
- **Not a home for stateful control.** `Delay`, `ForEach`, `LoopUntil`, `WaitFor` all need `StatefulAction` (they have cursor or timer state between engine dispatches). They will become `DelayAction` / `LoopAction` / `ScheduleUntilAction` DX families over `StatefulAction` in a separate spec.
- **Not a home for scheduler concerns.** `Merge`, `Parallel`, `Matrix`, `Subflow`, `Aggregate`, `Capability-gate` are engine, trigger-rule, or deployment-config problems. They will not be modeled as action types in any form. See §11.

### Design Principles

1. **Adapter pattern over blanket impl.** Mirrors `PollAction → PollTriggerAdapter → TriggerHandler` and `WebhookAction → WebhookTriggerAdapter → TriggerHandler`. Coherence lives on the adapter type (`ControlActionAdapter<A>`), not on `StatelessAction`. Any number of DX families can coexist without burning a coherence budget.
2. **Native `async fn` for authors, `#[async_trait]` for handlers.** `ControlAction::evaluate` uses RPITIT (`fn evaluate(...) -> impl Future<Output = ...> + Send`), same as current `StatelessAction::execute`. The adapter's `StatelessHandler::execute` uses `#[async_trait]` because the handler is dyn-compat and stored as `Arc<dyn StatelessHandler>` in the registry. The adapter bridges the two without boxing futures on the hot path.
3. **Type-safe outcome, not subset convention.** `ControlOutcome` is a distinct `#[non_exhaustive]` enum. Author cannot accidentally return `ActionResult::Wait` from a control node — the compiler refuses, not a runtime lint.
4. **Community extensibility by default.** The trait is public and non-sealed. Any external crate can `impl ControlAction` and wrap in `ControlActionAdapter`. The trait lives in `nebula-action` precisely so community doesn't need to fork or monkeypatch.
5. **Ports through standard `ActionMetadata`.** No separate `control_ports()` method. Authors declare inputs and outputs the same way as every other DX family. `RouterAction` uses `OutputPort::Dynamic` from the existing port model.

---

## 2. Research Context

This spec is grounded in a two-pass inventory of 23 workflow/automation/orchestration platforms conducted 2026-04-13:

**Pass 1 (consumer-focused):** n8n, Make.com, Zapier, Kestra, Apache Airflow, Argo Workflows, Prefect, Dagster, Node-RED, Inngest, Temporal, Windmill, Pipedream.

**Pass 2 (formal DSLs & enterprise iPaaS):** AWS Step Functions, Azure Logic Apps, GCP Workflows, Power Automate, Tines, LangGraph, GitHub Actions, Activepieces, Flyte, Metaflow.

**Findings:**

- **~80 concrete flow-control nodes** deduplicated into **34 semantic families**, spanning 11 categories (`branch`, `gate`, `iterate`, `aggregate`, `merge`, `delay`, `parallel`, `subflow`, `error`, `terminate`, `noop`) plus two candidate new categories (`dispatch`, `capability-gate`).
- **Only 7 families fit the synchronous-stateless-decision profile** that `ControlAction` addresses: `If`, `Switch`, `Router`, `Filter`, `NoOp`, `Stop`, `Fail`. The remaining 27 families either already have a home (`InteractiveAction`/`PaginatedAction`/`BatchAction`), or belong to scheduler/engine concerns, or require state and map to future DX over `StatefulAction`.
- **Kestra's Flowable Tasks category** is the closest prior art for a dedicated flow-control type distinction (confirmed: *"Flowable tasks control orchestration logic — running tasks or subflows in parallel, creating loops, and handling conditional branching"*), but its runtime model (YAML-nested child tasks) does not translate to a DAG engine.
- **Native backpressure** and **typed streaming channels** are unclaimed across all 23 platforms — but that territory belongs to a future `StreamAction` discussion, not `ControlAction`.

Full platform-by-platform tables and synthesis lived in the research conversation; decisions distilled into this spec + `decisions.md`.

---

## 3. Scope — The 7 Control Nodes

| Family | Semantic | ControlOutcome variant | Primary outputs |
|---|---|---|---|
| `IfAction` | Binary branch on predicate | `Branch { selected, output }` | 2 static ports: `true`, `false` |
| `SwitchAction` | N-way branch on value match (static port set) | `Branch { selected, output }` | N static ports + optional `default` |
| `RouterAction` | N-way routing with config-driven ports (first-match or all-match) | `Branch` (first-match) or `Route { ports }` (all-match) | `OutputPort::Dynamic` over rules array |
| `FilterAction` | Predicate gate — pass item or drop it | `Pass { output }` or `Drop { reason }` | 1 main output |
| `NoOpAction` | Pass-through / UI anchor / explicit placeholder | `Pass { output }` | 1 main output |
| `StopAction` | Terminate execution successfully | `Terminate { reason: Success }` | 0 outputs |
| `FailAction` | Terminate execution with typed error | `Terminate { reason: Failure { code, message } }` | 0 outputs |

Each of the 7 is a separate `struct` in `nebula-plugin-core`, each `impl ControlAction for ...`. Each is a 50–150 line file.

---

## 4. Prerequisites (Blocking Work)

Three correctness gaps in existing code block `ControlAction` implementation. They must land **before** Phase 1 (the trait itself) can correctly model Filter and Stop/Fail semantics. See plan for sequencing.

### 4.1 `ActionResult::Drop { reason }` — for Filter

**Problem:** current `ActionResult::Skip { reason }` is documented as "skip downstream dependents" — the entire subgraph reachable from this node is skipped. This matches n8n "skip branch" semantics but **not** Filter semantics: a filter drops one item while leaving the rest of the branch alive to process the next item.

Without `Drop`, `FilterAction::evaluate → ControlOutcome::Drop { reason }` has nowhere to desugar correctly. Desugaring to `Skip` would kill the downstream subgraph, which is the opposite of what a filter does in every platform surveyed (n8n, Node-RED `rbe`, Airflow `ShortCircuit`, Pipedream `Continue-if`, GitHub Actions `if`).

**Fix:** add `ActionResult::Drop { reason: Option<String> }`. Engine treats it as "this node produced no output on its main port — downstream dependents see no data from this branch, but the broader execution continues." Concretely, the engine's frontier logic should treat a dropped item the same as an upstream `Skip` on a single port, not on the whole subgraph.

### 4.2 `ActionResult::Terminate { reason }` + `ExecutionTerminationReason` — for Stop/Fail

**Problem:** `StopAction` desugared to `Skip` is silently wrong. In a parallel-branch execution, `Skip` on one branch leaves other branches running — but Stop semantically means "end this execution regardless of other branches." And audit logs have no way to distinguish a user-intended `Stop` from a crashed action.

**Fix:** add `ActionResult::Terminate { reason: TerminationReason }` where `TerminationReason` is:

```rust
#[non_exhaustive]
pub enum TerminationReason {
    Success { note: Option<String> },
    Failure { code: ErrorCode, message: String },
}
```

Engine recognises `Terminate` and transitions the whole `ExecutionState` (not just the node) to a terminal state. Companion change in `nebula-execution`:

```rust
#[non_exhaustive]
pub enum ExecutionTerminationReason {
    NaturalCompletion,
    ExplicitStop { by_node: NodeId, note: Option<String> },
    ExplicitFail { by_node: NodeId, code: ErrorCode, message: String },
    Cancelled,
    SystemError,
}
```

Audit log and UI can then correctly attribute "this workflow ended because node `stop_on_duplicate` decided to" vs. "a crash terminated the run."

**v1 limitation (shipped in Phase 0):** only the `ActionResult::Terminate` and `ExecutionTerminationReason` *types* have landed; the engine-side wiring is partial. `evaluate_edge` gates the local subgraph of a terminating node (same as `Skip`), but the scheduler does **not** yet cancel sibling branches in flight, and `determine_final_status` does not consume the `Terminate` signal. `ExecutionResult::termination_reason` therefore stays `None` today. Full scheduler integration is tracked as Phase 3 of this plan; until then, `Terminate` behaves as "stop my own subgraph," not "stop the whole execution."

### 4.3 `ActionCategory` in `ActionMetadata`

**Problem:** runtime doesn't need to distinguish control nodes from data nodes (the whole point of the adapter pattern). But the **UI editor, workflow validator, and audit log** do — palette grouping, icon shape, reachability validation, and log filtering all need to know whether a node is control-flow or data-flow. Without a discriminator in metadata, the editor has to hardcode type names, which breaks for community-contributed control nodes.

**Fix:** add `ActionCategory` enum to `metadata.rs`:

```rust
#[non_exhaustive]
pub enum ActionCategory {
    Data,        // StatelessAction / StatefulAction doing transformation
    Control,     // ControlAction family
    Trigger,     // TriggerAction
    Resource,    // ResourceAction
    Agent,       // AgentAction (future)
    Terminal,    // Subcategory: Stop, Fail — no downstream outputs
}
```

`ControlActionAdapter::metadata()` overrides the author's metadata to stamp `ActionCategory::Control` (or `::Terminal` for Stop/Fail) automatically, so authors cannot forget.

### 4.4 Documented `MultiOutput` join semantics

`RouterAction` in all-match mode desugars to `ActionResult::MultiOutput { outputs }`. The existing docstring does not pin down what "multi-output" means to downstream nodes — does a downstream with two upstream edges fire on any output or all outputs? This needs to be resolved in the `result.rs` docstring (not a code change, documentation only), because it affects `RouterAction`'s semantics.

**Decision for this doc update:** multi-output follows the same rule as multiple upstream edges with `all_success` trigger policy — downstream fires when **all** emitted output ports carry data, absent output ports imply "not emitted" and do not block downstream. Authors of routers who want first-match-only should use `Branch`, not `MultiOutput`.

---

## 5. Architecture

### 5.1 The adapter pattern, recap

```text
┌──────────────────────────────────────┐
│  nebula-action (public traits)       │
│                                      │
│   pub trait ControlAction            │
│       — non-sealed, community impl   │
│       — native async fn (RPITIT)     │
│       — returns ControlOutcome       │
│                                      │
│   pub struct ControlActionAdapter<A: ControlAction>
│       — wraps typed ControlAction    │
│       — impl StatelessHandler        │
│                                      │
│   #[async_trait]                     │
│   impl<A> StatelessHandler           │
│       for ControlActionAdapter<A>    │
│   where A: ControlAction + ...       │
│       — the single blanket           │
│       — coherence lives on adapter   │
└──────────────────────────────────────┘
              │
              ▼ type erasure
┌──────────────────────────────────────┐
│  Arc<dyn StatelessHandler>           │
│       stored in ActionRegistry       │
│       dispatched by existing         │
│       ActionHandler::Stateless path  │
└──────────────────────────────────────┘
              │
              ▼ registration
┌──────────────────────────────────────┐
│  nebula-plugin-core (concrete)       │
│                                      │
│   struct IfAction { ... }            │
│   impl ControlAction for IfAction    │
│                                      │
│   registry.register(                 │
│       ControlActionAdapter::new(     │
│           IfAction { ... }           │
│       )                              │
│   );                                 │
└──────────────────────────────────────┘
```

### 5.2 Trait shape

```rust
// crates/action/src/control.rs

use std::future::Future;
use serde_json::Value;

use crate::{ActionContext, ActionError, ActionMetadata};

/// DX trait for flow-control nodes — synchronous decisions on a single input.
///
/// Implementors are wrapped in [`ControlActionAdapter`] at registration time;
/// the adapter erases the typed trait to [`StatelessHandler`] and desugars
/// [`ControlOutcome`] to [`ActionResult`].
///
/// # When to implement this
///
/// Implement `ControlAction` for nodes that:
///
/// - Make a synchronous decision based on a single input value
/// - Route, filter, or terminate execution based on that decision
/// - Do not need iteration, delay, or external signals (those go to `StatefulAction`)
///
/// "Stateless" here means **no engine-persisted state between dispatches**:
/// no `State` associated type, no checkpointing, no serialization. In-memory
/// `&self` state for local concerns (rate-limit counters, simple caches,
/// metrics) is fine — it just does not survive process restarts. If you
/// need state that *does* survive restarts, you want `StatefulAction`,
/// not `ControlAction`.
///
/// # When NOT to implement this
///
/// - Need cursor or counter between calls → `StatefulAction` (via `PaginatedAction`/`BatchAction`)
/// - Wait for time or external signal → `StatefulAction` (via future `DelayAction` / `InteractiveAction`)
/// - Fork execution into parallel branches → not an action at all; it's a DAG topology concern
/// - Aggregate N inputs into one output → not an action; it's a `trigger_rule` on incoming edges
///
/// # Example
///
/// ```ignore
/// use nebula_action::{ControlAction, ControlInput, ControlOutcome, ActionContext, ActionError, ActionMetadata};
/// use serde_json::Value;
///
/// pub struct IfAction {
///     metadata: ActionMetadata,
/// }
///
/// impl ControlAction for IfAction {
///     fn metadata(&self) -> &ActionMetadata {
///         &self.metadata
///     }
///
///     async fn evaluate(
///         &self,
///         input: ControlInput,
///         _ctx: &ActionContext,
///     ) -> Result<ControlOutcome, ActionError> {
///         let condition = input.get_bool("/condition")?;
///         let selected = if condition { "true" } else { "false" };
///         Ok(ControlOutcome::Branch {
///             selected: selected.into(),
///             output: input.into_value(),
///         })
///     }
/// }
/// ```
pub trait ControlAction: Send + Sync + 'static {
    /// Static metadata describing this node (ports, parameters, category).
    ///
    /// The adapter may override `category` to stamp [`ActionCategory::Control`]
    /// (or `::Terminal` for nodes that return only [`ControlOutcome::Terminate`]).
    fn metadata(&self) -> &ActionMetadata;

    /// Evaluate the control decision for a single input.
    ///
    /// Returned [`ControlOutcome`] determines how execution proceeds:
    /// branch, route, pass, drop, or terminate. See variant docs.
    ///
    /// This method must not block on external resources or persist state
    /// between calls — those use cases belong to `StatefulAction`.
    fn evaluate(
        &self,
        input: ControlInput,
        ctx: &ActionContext,
    ) -> impl Future<Output = Result<ControlOutcome, ActionError>> + Send;
}
```

### 5.3 `ControlOutcome`

```rust
/// The decision returned by a [`ControlAction`].
///
/// Each variant corresponds to a flow-control semantic that cannot be
/// expressed safely through the broader `ActionResult` surface.
#[non_exhaustive]
pub enum ControlOutcome {
    /// Route the input to one selected output port.
    ///
    /// Used by `IfAction` (2-way), `SwitchAction` (N-way static), and
    /// `RouterAction` in first-match mode.
    ///
    /// `selected` must match a port key declared in [`ActionMetadata::outputs`];
    /// the adapter validates this in debug builds (release: skipped for perf).
    Branch {
        selected: PortKey,
        output: Value,
    },

    /// Route the input to multiple output ports in one call.
    ///
    /// Used by `RouterAction` in all-match mode. Desugars to
    /// `ActionResult::MultiOutput`. Downstream join semantics follow the
    /// documented `MultiOutput` contract (see `result.rs`).
    Route {
        ports: Vec<(PortKey, Value)>,
    },

    /// Pass the input through unchanged to the single main output.
    ///
    /// Used by `NoOpAction` and `FilterAction` in "match" case.
    Pass {
        output: Value,
    },

    /// Drop this item without emitting output on the main port.
    ///
    /// Used by `FilterAction` in "no-match" case. Desugars to
    /// `ActionResult::Drop`. Unlike `Skip`, the broader execution continues;
    /// only this item is silently removed from the flow.
    Drop {
        reason: Option<String>,
    },

    /// Terminate this branch and signal that execution should stop.
    ///
    /// Used by `StopAction` (success) and `FailAction` (error). Desugars
    /// to `ActionResult::Terminate`. **v1 scope**: the engine's
    /// `evaluate_edge` treats `Terminate` like `Skip` — downstream edges
    /// from this node do not fire. Full parallel-branch cancellation
    /// (reaching sibling branches in flight) and `ExecutionResult::
    /// termination_reason` population via `ExecutionTerminationReason::
    /// ExplicitStop` / `ExplicitFail` are deferred scheduler work (see
    /// §4.2 and the plan Phase 3). Until that ships, `Terminate` is
    /// best-effort local signalling, not execution-wide enforcement.
    Terminate {
        reason: TerminationReason,
    },
}
```

### 5.4 `TerminationReason` and `TerminationCode`

```rust
/// Why a `ControlAction` requested termination.
#[non_exhaustive]
pub enum TerminationReason {
    /// Successful early termination — `StopAction`.
    ///
    /// Maps (once scheduler wiring is complete) to
    /// `ExecutionStatus::Completed` with an audit-log note that the
    /// node `by_node` requested termination. In Phase 0 only the
    /// local downstream edges are gated; the execution status is
    /// whatever `determine_final_status` computes from the drained
    /// frontier.
    Success { note: Option<String> },

    /// Error termination — `FailAction`.
    ///
    /// Maps (once wired) to `ExecutionStatus::Failed` with a typed
    /// termination code and message. Companion
    /// `ExecutionTerminationReason::ExplicitFail` will be recorded in
    /// the audit log for distinguishing from crashes when the Phase 3
    /// scheduler wiring lands.
    Failure {
        code: TerminationCode,
        message: String,
    },
}

/// Opaque identifier for a termination error.
///
/// Public newtype (`#[serde(transparent)]`) over `Arc<str>`. Pinning
/// the wire format today lets the internal representation swap to a
/// structured `ErrorCode` in Phase 10 of the action-v2 roadmap without
/// breaking serialized shapes or the public API.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TerminationCode(Arc<str>);
```

### 5.5 `ControlInput`

```rust
/// Owned wrapper around the input `Value` passed to a control node.
///
/// Provides convenient typed accessors so control authors don't
/// reinvent the same `serde_json::Value::pointer(...).and_then(...)`
/// boilerplate for every If/Switch/Filter.
///
/// Owned (not borrowed) because the returned future carries a
/// `+ 'static` bound via `StatelessHandler`, and a borrowed wrapper
/// would force `async move` to copy anyway.
#[non_exhaustive]
pub struct ControlInput {
    value: Value,
}

impl ControlInput {
    /// Construct from a raw JSON value.
    pub fn from(value: Value) -> Self {
        Self { value }
    }

    /// Read a boolean at a JSON pointer.
    pub fn get_bool(&self, pointer: &str) -> Result<bool, ActionError> {
        // returns ActionError::validation(...) on missing / wrong type
        todo!()
    }

    /// Read a string slice at a JSON pointer.
    pub fn get_str(&self, pointer: &str) -> Result<&str, ActionError> { todo!() }

    /// Read an integer at a JSON pointer.
    pub fn get_i64(&self, pointer: &str) -> Result<i64, ActionError> { todo!() }

    /// Read an f64 at a JSON pointer.
    pub fn get_f64(&self, pointer: &str) -> Result<f64, ActionError> { todo!() }

    /// Read an arbitrary sub-value at a JSON pointer.
    pub fn get(&self, pointer: &str) -> Option<&Value> {
        self.value.pointer(pointer)
    }

    /// Consume the wrapper and return the underlying value.
    ///
    /// Used in passthrough cases — e.g. `ControlOutcome::Pass { output: input.into_value() }`.
    pub fn into_value(self) -> Value {
        self.value
    }
}
```

The accessor set is extensible because `ControlInput` is `#[non_exhaustive]`; new helpers (e.g. `get_datetime`, `get_path`) can land in post-v1 without breaking external authors.

### 5.6 `ControlActionAdapter`

```rust
use async_trait::async_trait;
use std::sync::Arc;

use crate::{ActionMetadata, StatelessHandler};

/// Adapter that wraps a typed [`ControlAction`] and erases it to
/// [`StatelessHandler`] for registration in the action registry.
///
/// This follows the same pattern as [`PollTriggerAdapter`] and
/// [`WebhookTriggerAdapter`]: the DX trait is native async + typed,
/// the adapter is `#[async_trait]` + dyn-compat.
pub struct ControlActionAdapter<A: ControlAction> {
    action: A,
    cached_metadata: Arc<ActionMetadata>,
}

impl<A: ControlAction> ControlActionAdapter<A> {
    /// Wrap a typed control action.
    ///
    /// The adapter caches a copy of the action's metadata with
    /// `category` field stamped to [`ActionCategory::Control`] (or
    /// `::Terminal` for Stop/Fail). Authors cannot forget to set
    /// the category.
    pub fn new(action: A) -> Self {
        let mut meta = action.metadata().clone();
        meta.category = derive_category(&meta, &action);
        Self {
            action,
            cached_metadata: Arc::new(meta),
        }
    }
}

#[async_trait]
impl<A> StatelessHandler for ControlActionAdapter<A>
where
    A: ControlAction + Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.cached_metadata
    }

    async fn execute(
        &self,
        input: Value,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let outcome = self.action.evaluate(ControlInput::from(input), ctx).await?;
        Ok(outcome.into_action_result())
    }
}

/// Category inference rule:
///
/// - Terminal if metadata has zero declared output ports (Stop, Fail)
/// - Control otherwise
fn derive_category<A: ControlAction>(meta: &ActionMetadata, _action: &A) -> ActionCategory {
    if meta.outputs.is_empty() {
        ActionCategory::Terminal
    } else {
        ActionCategory::Control
    }
}
```

### 5.7 Desugaring `ControlOutcome` → `ActionResult`

```rust
impl From<ControlOutcome> for ActionResult<Value> {
    fn from(outcome: ControlOutcome) -> Self {
        match outcome {
            ControlOutcome::Branch { selected, output } => ActionResult::Branch {
                selected,
                output: ActionOutput::Value(output),
                alternatives: HashMap::new(),
            },
            ControlOutcome::Route { ports } => {
                // `ports` is already a HashMap<PortKey, Value>; wrap values
                // into ActionOutput and hand off to MultiOutput. Duplicates
                // are unrepresentable because the caller built a HashMap.
                let outputs = ports
                    .into_iter()
                    .map(|(k, v)| (k, ActionOutput::Value(v)))
                    .collect();
                ActionResult::MultiOutput {
                    outputs,
                    main_output: None,
                }
            }
            ControlOutcome::Pass { output } => ActionResult::Success {
                output: ActionOutput::Value(output),
            },
            ControlOutcome::Drop { reason } => ActionResult::Drop { reason },
            ControlOutcome::Terminate { reason } => {
                ActionResult::Terminate { reason }
            }
        }
    }
}
```

`ActionResult::Drop` and `ActionResult::Terminate` are added in Prerequisites (§4.1, §4.2); this `From` impl is what validates their shape is right. Note: all payload-carrying variants of `ActionResult` take `output: ActionOutput<T>` (not bare `T`) because `ActionOutput` is the common envelope that supports inline values, binary blobs, references, and streaming — the control adapter always wraps its `serde_json::Value` payload via `ActionOutput::Value(_)`.

---

## 6. Community Extensibility

### 6.1 Extension point

A third-party crate (say, `acme-workflow-nodes`) can publish new control nodes without forking Nebula:

```rust
// Cargo.toml
[dependencies]
nebula-action = "..."
async-trait = "..."
serde_json = "..."

// lib.rs
use nebula_action::{
    ControlAction, ControlActionAdapter, ControlInput, ControlOutcome,
    ActionContext, ActionError, ActionMetadata, StatelessHandler,
};
use serde_json::Value;
use std::sync::Arc;

pub struct ThrottleAction {
    metadata: ActionMetadata,
    rate_limit_per_sec: u32,
    // In-memory rate-limit counter. Does not survive process restarts —
    // that is fine for best-effort local throttling, which is the whole
    // point of this example. "Stateless between invocations" in the
    // ControlAction contract means no engine-persisted state (no `State`
    // associated type, no checkpointing, no serialization), not "no
    // `&self` fields."
    state: parking_lot::Mutex<ThrottleState>,
}

impl ControlAction for ThrottleAction {
    fn metadata(&self) -> &ActionMetadata { &self.metadata }

    async fn evaluate(
        &self,
        input: ControlInput,
        _ctx: &ActionContext,
    ) -> Result<ControlOutcome, ActionError> {
        let mut st = self.state.lock();
        if st.should_drop(self.rate_limit_per_sec) {
            Ok(ControlOutcome::Drop { reason: Some("rate limit exceeded".into()) })
        } else {
            Ok(ControlOutcome::Pass { output: input.into_value() })
        }
    }
}

// User registers it in their application bootstrap:
registry.register(
    Arc::new(ControlActionAdapter::new(ThrottleAction::new(100)))
        as Arc<dyn StatelessHandler>
);
```

### 6.2 What community cannot do (on purpose)

- Add new `ControlOutcome` variants. The enum is `#[non_exhaustive]` but only `nebula-action` may add variants — community extending it would fragment the ecosystem on how downstream engines interpret the new variant.
- Add second blanket impl over `StatelessAction`. `nebula-action` documents the coherence rule: there is exactly one DX family blanket-impl'd to `StatelessHandler` via `ControlActionAdapter`. A second DX family targeting `StatelessHandler` must use its own adapter type (`FooAdapter<A: FooAction>: StatelessHandler`), which is a separate concrete type and poses no coherence risk.
- Bypass the adapter. `impl StatelessHandler for MyControlNode` directly is possible but unsupported — the author loses `ControlOutcome` type safety and the automatic `ActionCategory` stamp.

---

## 7. Relationship to Existing DX Families

| DX family | Author trait | Adapter | Core trait erased to | Lives in |
|---|---|---|---|---|
| `PollAction` | `PollAction` (native async) | `PollTriggerAdapter<A>` | `TriggerHandler` | `nebula-action::poll` |
| `WebhookAction` | `WebhookAction` (native async) | `WebhookTriggerAdapter<A>` | `TriggerHandler` | `nebula-action::webhook` |
| `ControlAction` (this spec) | `ControlAction` (native async) | `ControlActionAdapter<A>` | `StatelessHandler` | `nebula-action::control` |
| Future: `DelayAction` / `LoopAction` | TBD | TBD | `StatefulHandler` | `nebula-action::delay` / `nebula-action::loop_` |

`ControlAction` is the first DX family targeting `StatelessHandler`. Its adapter-pattern approach is a template for any future DX family needing this target.

---

## 8. Crate Placement

### 8.1 Trait and adapter in `nebula-action`

Everything described in §5 lives in `crates/action/src/control.rs`. That file contains only: trait, types, adapter, From impls, tests. Zero concrete node implementations. This keeps `nebula-action` at the "protocol, not runtime" design principle from action v2 spec §1.

### 8.2 Concrete 7 nodes live downstream

The 7 concrete control nodes (`IfAction`, `SwitchAction`, `RouterAction`, `FilterAction`, `NoOpAction`, `StopAction`, `FailAction`) do **not** live in `nebula-action`. They live in a separate downstream crate. The exact name and location of that crate is a packaging decision that does not affect the trait contract, and is explicitly **out of scope for this spec**.

Constraints the downstream crate must satisfy:

- Depends on `nebula-action` (for the `ControlAction` trait)
- Sits in the Business layer per `cargo deny` (`nebula-action` is Business; downstream stays Business or higher)
- No circular dependency on `nebula-action`

### 8.3 Why the separation matters regardless of name

- `nebula-action` stays pure contract. Concrete plugins are downstream of the contract crate, architecturally and in the dependency graph.
- Community plugin authors have a canonical reference implementation to copy from — whichever crate hosts it.
- Bundling concrete nodes into `nebula-action` would couple trait evolution to reference-impl evolution, defeating the adapter-pattern isolation.

---

## 9. Testing Strategy

### 9.1 Unit tests in `nebula-action`

- `ControlOutcome` → `ActionResult` conversion: one test per variant.
- `ControlActionAdapter::metadata()` stamps `ActionCategory::Control` for nodes with outputs.
- `ControlActionAdapter::metadata()` stamps `ActionCategory::Terminal` for nodes with zero outputs.
- `ControlActionAdapter::execute` wires `evaluate` → `into_action_result` correctly.
- One dummy `struct TestIf; impl ControlAction for TestIf { ... }` as a smoke test of the full path.

### 9.2 Contract tests

- Serialization round-trip for `ControlOutcome`, `TerminationReason` (even though `ControlOutcome` is not serialized over the wire, `TerminationReason` is, via `ActionResult::Terminate`).
- `non_exhaustive` match-arm compilation test — `match outcome { ... _ => unreachable!() }` compiles in external crate.

### 9.3 Integration tests in the downstream crate

Per concrete node: golden-path `evaluate` test + one error case. Shared test helper that builds a minimal `ActionContext` (leveraging `TestContextBuilder::minimal()` from action-v2 Phase 2b, or direct construction if Phase 2b hasn't landed yet). Lives in whichever crate ends up hosting the concrete nodes.

### 9.4 Engine integration test

One end-to-end test that constructs a 3-node workflow (`Source → If → Sink`) and verifies that the `true` branch fires on a truthy input, `false` branch on falsy, execution state transitions are correct, and `ActionCategory::Control` appears in the node metadata at runtime. Lives in `crates/engine/tests/control_action_smoke.rs` or similar (path TBD when engine integration tests exist).

---

## 10. Open Questions (Deferred to post-v1)

1. **Validation hook.** Should `ControlAction` have a `validate(&self, ctx: &ActionContext) -> Result<(), ActionError>` called once at registration time (like `PollAction::validate`)? Or should the adapter debug-assert in `execute` that `ControlOutcome::Branch::selected` matches a declared output port? Decision deferred until v1 is in use and real validation bugs surface. Note: the current `ControlActionAdapter::new` already has an opportunity to run validation because it inspects metadata — this is a natural hook if we add it later.

2. **`ControlInput` typed getters — extent.** The spec proposes `get_bool` / `get_str` / `get_i64` / `get_f64`. Should we add `get_datetime`, `get_uuid`, `get_path`? Decision: start with four basic getters, extend based on plugin author feedback.

3. **Error-as-artifact pattern (Metaflow `@catch(var=)`).** Interesting DX: upstream error becomes a variable readable by the next node, instead of jumping to a handler branch. Would need `ControlInput::previous_error()` and scheduler cooperation to pipe error data forward. Not in v1 scope; revisit when we have enough real-world signal.

4. **`ControlOutcome::Defer`** or **`ControlOutcome::Emit`** variants. Possible future additions:
   - `Defer` — node wants the engine to retry after some condition. But this is what `StatefulAction::Wait` is for, so probably unnecessary.
   - `Emit` — fire an event on the `EventBus` as a side effect of the control decision. Useful for audit/telemetry. Revisit if a concrete use case surfaces.

5. **Versioning of `ControlOutcome`.** `#[non_exhaustive]` handles additive evolution, but if we ever need to *remove* a variant or change a variant's shape, we need a migration story. No decision now; revisit when the first such case arises.

---

## 11. Non-Goals / Explicitly Out of Scope

These are **not** `ControlAction` problems. Each has its own home.

| Concern | Where it belongs |
|---|---|
| `Delay` / `Wait` / `Sleep` | Future `DelayAction` DX over `StatefulAction` (uses `ActionResult::Wait`) |
| `ForEach` / `Loop` / `LoopUntil` / `Until` | Future `LoopAction` DX over `StatefulAction` |
| `Split In Batches` / `ForEachItem` | Existing `BatchAction` / `PaginatedAction` plan |
| `Approval` / `Wait for webhook` | Existing `InteractiveAction` / `WebhookAction` plan |
| `Parallel` / `branchall` / static fan-out | DAG topology — not an action type |
| `Matrix` / Airflow `.expand()` | Plan-time DAG expansion — engine feature, not action |
| `Merge` / `Join` with trigger rules | `trigger_rule` on incoming edges — scheduler feature |
| `Subflow` / `Execute Workflow` / `ChildWorkflow` | Dedicated engine primitive (new type or `ctx` capability); see `decisions.md` for open discussion |
| `Aggregator` (array/numeric/text/table) | Business-logic `StatelessAction` plugins, not control |
| `Capability-gate` (concurrency, environment, reviewers) | Deployment/runtime config, not a node type |
| `Retry policy` | `ActionMetadata` field + per-node override in `NodeDefinition`, not an action trait |
| `Compensation / saga` | Explicitly rejected today — see `decisions.md` "No saga / transactional trait today" |
| `goto` / unconditional jump | Rejected — breaks DAG semantics, no platform except GCP Workflows offers it |
| `Stream` / internal multi-stage pipeline with backpressure | Future discussion, separate spec; orthogonal to `ControlAction` |
| `Agent tools` via sub-nodes | `AgentAction` Phase 9 territory; uses `SupportPort` which already exists |

---

## 12. Rejected Alternatives

Full rationale in `.project/context/decisions.md` — "ControlAction — adapter pattern, not blanket impl, not macro". Condensed here:

- **Declarative macro**: blocks community extensibility.
- **Seven separate sealed traits**: blocks community for the same reason.
- **Sixth core trait in `ActionHandler` enum**: unnecessary — adapter + existing `StatelessHandler` dispatch is enough.
- **Blanket `impl<T: ControlAction> StatelessAction for T`**: burns the sole coherence slot on `StatelessAction`; adapter pattern avoids the budget problem.

---

## 13. References

- `crates/action/src/poll.rs` — `PollTriggerAdapter` prior art for the adapter pattern
- `crates/action/src/webhook.rs` — `WebhookTriggerAdapter` prior art
- `crates/action/src/port.rs` — existing port model (`SupportPort`, `DynamicPort`, `ConnectionFilter`)
- `crates/action/src/result.rs` — `ActionResult` current shape (needs §4.1, §4.2 extensions)
- `crates/action/src/metadata.rs` — `ActionMetadata` (needs §4.3 `ActionCategory` field)
- `crates/execution/src/status.rs` — `ExecutionState` machine (needs §4.2 `ExecutionTerminationReason`)
- `docs/plans/2026-04-08-action-v2-spec.md` — action v2 philosophy, DX layer rules
- `.project/context/decisions.md` — "ControlAction — adapter pattern, not blanket impl, not macro"

Companion implementation plan: `docs/plans/2026-04-13-control-action-plan.md`.
