# nebula-runtime

Action execution layer — `ActionRegistry`, `ActionRuntime`, data policies, `MemoryQueue`.

## Invariants

- `ActionRegistry` is the single source of truth. DashMap-backed, `&self` API. Six `register_*` methods (`stateless`, `stateful`, `trigger`, `webhook`, `poll`, `resource`). Lookup returns owned `(ActionMetadata, ActionHandler)`; `ActionHandler` wraps `Arc` internally.
- `ActionRuntime::run_handler` dispatches on the `ActionHandler` enum:
  - `Stateless` → `IsolationLevel::None` direct; `CapabilityGated`/`Isolated` through `self.sandbox: Arc<dyn SandboxRunner>`. Unknown variants fail-closed.
  - `Stateful` → iteration loop racing `handler.execute(..)` against `context.cancellation.cancelled()` via `tokio::select!` (#304 fix). Hard cap `MAX_ITERATIONS = 10_000`. Optional `StatefulCheckpointSink` hook persists `(iteration, state)` at every iteration boundary and resumes on the next dispatch (#308). Non-`None` isolation still fail-closed — needs Phase 1 broker.
  - `Trigger` / `Resource` / `Agent` → typed `RuntimeError::*NotExecutable` / `AgentNotSupportedYet`. Only the `NEBULA_ACTION_DISPATCH_REJECTED_TOTAL` counter fires (with a `reason` label); duration histogram and executions/failures counters are NOT touched on these paths (#305 fix).
- `DataPassingPolicy` / `LargeDataStrategy` enforce output size limits. `enforce_data_limit` walks **every** downstream-visible slot (`MultiOutput.outputs`, `Branch.alternatives`, `Wait.partial_output`, …) — a fan-out port cannot hide a large payload behind a small main.
- `SandboxRunner` + `InProcessSandbox` + `SandboxedContext` + `ActionExecutor` live in `nebula-sandbox`, re-exported from `runtime::sandbox`. New code should import `nebula-sandbox` directly.
- `ActionRuntime::new(registry, sandbox, data_policy, metrics)` — no `EventBus`. Runtime only records metrics.

## Traps

- `execute_action` runs only Stateless and Stateful. Trigger/Resource/Agent return typed errors — separate lifecycles.
- Stateful state persists across dispatches ONLY when the caller wires a `StatefulCheckpointSink` (engine does this via `ExecutionRepo::{save,load,delete}_stateful_checkpoint`). Without a sink the loop is still stack-local. Sink semantics are asymmetric by design: `load` failures WARN+fallback to `init_state`, `save` failures propagate as `ActionError`, and terminal `clear` failures WARN+ignore.
- **Stateless sandbox dispatch is live** (Phase 0): `CapabilityGated` / `Isolated` go through `self.sandbox`. In Phase 0 the engine passes an echo-style `ActionExecutor`, so non-None actions silently echo input rather than invoking the registered handler. Acceptable because **no production actions declare non-None isolation yet**. Phase 1 replaces this with `PluginSupervisor` + gRPC.
- **Stateful sandbox dispatch still fail-closes** for non-None. Unblocks when Phase 1 broker iteration loop lands.
- `RuntimeError::InvalidActionKey { key, reason }` distinguishes parse failures from `ActionNotFound` — use for user-facing CLI/API errors.
- Stateful cancellation races the handler future against `cancellation.cancelled()` inside every iteration (#304). Dropping the pinned future aborts mid-`await` — handlers whose mid-`await` state cannot safely be dropped must document that and guard critical sections internally. See `StatefulHandler::execute` doc.
- `MemoryQueue::nack` is part of the at-least-once contract: never remove from `in_flight` until requeue succeeds. Using `try_send` after removal can silently lose tasks when the queue is full.

## Relations

Depends on `nebula-action`, `nebula-core`, `nebula-sandbox`. Used by `nebula-engine`.

<!-- reviewed: 2026-04-14 (batch5B: #304 #305 #308 + PR389 review fixes) -->
