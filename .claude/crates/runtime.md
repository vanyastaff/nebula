# nebula-runtime
Action execution layer — ActionRegistry, data policies, and MemoryQueue.

## Invariants
- Actions run in-process inside `SandboxedContext`. The engine calls `ActionRuntime`, which calls the sandbox, which calls the action handler.
- **Sandbox types re-exported from `nebula-sandbox`** for backward compatibility. New code should depend on `nebula-sandbox` directly.

## Key Decisions
- `ActionRegistry` maps `ActionKey → InternalHandler` with version index. `get()` returns latest; `get_versioned()` returns specific version.
- `DataPassingPolicy` / `LargeDataStrategy` enforce output size limits — oversized outputs can be redirected to blob storage.
- `MemoryQueue` / `TaskQueue` for async task dispatch. `BoundedStreamBuffer` / `PushOutcome` for streaming backpressure.
- `SandboxRunner` trait, `InProcessSandbox`, `SandboxedContext` — **moved to `nebula-sandbox`**, re-exported here.
- `ActionRuntime::new(registry, sandbox, data_policy, metrics)` — no EventBus. Runtime records metrics only.

## Traps
- `sandbox.rs` is now a re-export module. Actual implementation lives in `nebula-sandbox`.
- `ActionExecutor` type alias also from `nebula-sandbox`.

## Relations
- Depends on nebula-action, nebula-core, **nebula-sandbox**. Used by nebula-engine.

<!-- reviewed: 2026-04-08 — sandbox types moved to nebula-sandbox crate, runtime re-exports for compat -->
