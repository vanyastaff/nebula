# nebula-runtime

Action execution layer — `ActionRegistry`, `ActionRuntime`, data policies, `MemoryQueue`.

## Invariants

- `ActionRegistry` is the single source of truth for registered actions. DashMap-backed, `&self` API. Has six `register_*` convenience methods (`stateless`, `stateful`, `trigger`, `webhook`, `poll`, `resource`). Lookup returns owned `(ActionMetadata, ActionHandler)`; `ActionHandler` wraps `Arc` internally so cloning is cheap.
- `ActionRuntime::run_handler` dispatches on the `ActionHandler` enum:
  - `Stateless` → direct execution for `IsolationLevel::None`; non-`None` returns `Fatal` (fail-closed until real sandbox dispatch lands).
  - `Stateful` → iteration loop with in-memory state checkpoint, hard cap `MAX_ITERATIONS = 10_000`, cooperative cancellation **between** iterations.
  - `Trigger` → `RuntimeError::TriggerNotExecutable` (triggers have their own runtime, post-v1).
  - `Resource` → `RuntimeError::ResourceNotExecutable` (resources scoped via the resource graph, post-v1).
  - `Agent` → `RuntimeError::AgentNotSupportedYet`.
- Non-executable variants (Trigger/Resource/Agent) are still counted as failed executions in metrics — they were valid registry lookups invoked via the wrong path.
- `DataPassingPolicy` / `LargeDataStrategy` enforce output size limits — oversized outputs can be redirected to blob storage.
- `SandboxRunner` trait, `InProcessSandbox`, `SandboxedContext`, `ActionExecutor` — implemented in `nebula-sandbox`, re-exported from `runtime::sandbox` for backward compatibility. New code should import from `nebula-sandbox` directly.
- `ActionRuntime::new(registry, sandbox, data_policy, metrics)` — no `EventBus`. Runtime only records metrics.

## Traps

- `ActionRuntime::execute_action` runs **only** Stateless and Stateful actions. Trigger/Resource/Agent return typed errors — they have separate lifecycles that don't fit the one-shot execute model.
- Stateful state is in-memory only — does not survive process restart. Persistence needs `nebula-storage` integration.
- Sandboxed Stateless/Stateful execution returns `Fatal` for any `IsolationLevel != None` — fail-closed to prevent silent capability bypass. Unblocks when real sandbox dispatch lands.
- `RuntimeError::InvalidActionKey { key, reason }` distinguishes parse failures from `ActionNotFound` — use it when reporting CLI/API errors to users.
- Cooperative cancellation in the stateful loop checks **between** iterations only. A hanging `execute()` body cannot be cancelled.

## Relations

Depends on `nebula-action`, `nebula-core`, `nebula-sandbox`. Used by `nebula-engine`.
