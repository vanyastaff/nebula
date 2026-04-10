# nebula-runtime
Action execution layer — ActionRegistry, data policies, and MemoryQueue.

## Invariants
- Actions run in-process inside `SandboxedContext`. The engine calls `ActionRuntime`, which calls the sandbox, which calls the action handler.
- **Sandbox types re-exported from `nebula-sandbox`** for backward compatibility. New code should depend on `nebula-sandbox` directly.

## Key Decisions
- **Phase 7.5 (2026-04-09):** `ActionRegistry` is now the single source of truth — moved from `nebula-action` to `nebula-runtime`. DashMap-backed, `&self` API. Has all 6 register_* convenience methods: `register_stateless`, `register_stateful`, `register_trigger`, `register_webhook`, `register_poll`, `register_resource`. Lookup returns owned `(ActionMetadata, ActionHandler)` — `ActionHandler` is `Arc` inside, cloning is cheap.
- **`ActionRuntime::run_handler` dispatches on `ActionHandler` enum** (5 variants):
  - `Stateless` → direct execution (sandbox dispatch deferred to Phase 7.6)
  - `Stateful` → iteration loop with in-memory state checkpoint, hard cap MAX_ITERATIONS=10_000, cooperative cancellation between iterations
  - `Trigger` → `Err(RuntimeError::TriggerNotExecutable)` permanently — triggers run via dedicated trigger runtime (post-v1)
  - `Resource` → `Err(RuntimeError::ResourceNotExecutable)` permanently — resources scoped via resource graph (post-v1)
  - `Agent` → `Err(RuntimeError::AgentNotSupportedYet)` (Phase 9)
- Non-executable variants (Trigger/Resource/Agent) are counted as failed executions in metrics — they were valid lookups attempted via the wrong path.
- `DataPassingPolicy` / `LargeDataStrategy` enforce output size limits — oversized outputs can be redirected to blob storage.
- `MemoryQueue` / `TaskQueue` for async task dispatch. `BoundedStreamBuffer` / `PushOutcome` for streaming backpressure.
- `SandboxRunner` trait, `InProcessSandbox`, `SandboxedContext` — **moved to `nebula-sandbox`**, re-exported here.
- `ActionRuntime::new(registry, sandbox, data_policy, metrics)` — no EventBus. Runtime records metrics only.

## Traps
- `sandbox.rs` is now a re-export module. Actual implementation lives in `nebula-sandbox`.
- `ActionExecutor` type alias also from `nebula-sandbox`.
- `ActionRuntime::execute_action` only runs Stateless and Stateful actions. Triggers/Resources/Agents return typed errors — they have separate lifecycles that don't fit the one-shot execute model.
- Stateful state is in-memory only — does not survive process restart. Persistence requires nebula-storage integration.
- Sandboxed Stateful execution returns `ActionError::Fatal` for non-`None` isolation. Phase 7.6 work.
- Sandboxed Stateless execution currently bypasses the sandbox dispatch path — calls handler directly. Phase 7.6 work.
- Cooperative cancellation in stateful loop checks between iterations only. A poorly-written `execute()` that hangs forever inside one iteration cannot be cancelled.

## Relations
- Depends on nebula-action, nebula-core, **nebula-sandbox**. Used by nebula-engine.

<!-- reviewed: 2026-04-09 — Phase 7.5 ActionRegistry unification, ActionHandler enum dispatch, InternalHandler deleted -->
