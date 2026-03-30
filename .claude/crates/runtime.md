# nebula-runtime
Action execution layer — ActionRegistry, InProcessSandbox, data policies, and MemoryQueue.

## Invariants
- **`InProcessSandbox` only** — Phase 2. No OS-process isolation, no WASM sandbox. Adding either is Phase 3 (ADR 008). Do not implement process/WASM isolation here.
- Actions run in-process inside `SandboxedContext`. The engine calls `ActionRuntime`, which calls `InProcessSandbox`, which calls the action handler.

## Key Decisions
- `ActionRegistry` maps `ActionKey → InternalHandler`. Actions must be registered before the engine can dispatch them.
- `DataPassingPolicy` / `LargeDataStrategy` enforce output size limits — oversized outputs can be redirected to blob storage.
- `MemoryQueue` / `TaskQueue` for async task dispatch. `BoundedStreamBuffer` / `PushOutcome` for streaming backpressure.
- `SandboxedContext` wraps `ActionContext` with the sandbox boundary — implements the `Context` trait.

## Traps
- Phase 3 will add OS-process sandbox. When that happens, `InProcessSandbox` → `SandboxRunner` trait split is planned. Don't tightly couple to `InProcessSandbox` concrete type.
- `ActionExecutor` is the trait; `InProcessSandbox` is the Phase 2 impl. Use the trait in test mocks.

## Relations
- Depends on nebula-action, nebula-core. Used by nebula-engine. Sits between engine (scheduling) and action (business logic).

<!-- reviewed: 2026-03-30 -->
