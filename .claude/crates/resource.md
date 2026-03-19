# nebula-resource
Resource lifecycle management — pooling, health checks, autoscaling, hooks, and RAII guards.

## Invariants
- Resources are always accessed through `Guard` (RAII). On `Guard` drop, the resource is automatically returned to the pool.
- Never hold a raw resource instance outside a guard. Pool integrity depends on RAII.
- Never import nebula-credential directly. Use `EventBus<CredentialRotatedEvent>` for credential rotation signals.

## Key Decisions
- `acquire_typed::<R>(&ctx)` uses `TypeId` — type-safe acquisition without string keys. Prefer over string-based `acquire("name", &ctx)`.
- `Manager` is the central registry. `ManagerBuilder` for construction. `TypedPool` for per-type pools.
- `ResourceProvider` trait = DI for actions (mirrors `CredentialProvider` pattern). Injected via `ResourceAccessor` in `Context`.
- `QuarantineManager` isolates unhealthy resources. `HealthChecker` tracks health state transitions.
- `AutoScaler` dynamically adjusts pool size based on `AutoScalePolicy`.
- `HookRegistry` + `ResourceHook` for lifecycle events (acquire, release, health-change, slow-acquire).

## Traps
- Circular dep with nebula-credential (see above — EventBus only).
- `PoisonGuard` wraps a resource that panicked during use — check `is_poisoned()` before returning to pool.
- `ResourceEvent` bus is per-manager, not global — subscribe to the specific manager's bus.

## Relations
- Depends on nebula-core, nebula-telemetry (re-exports CallRecord types), nebula-eventbus. Peer with nebula-credential.

<!-- reviewed: 2026-03-19 -->
<!-- fmt-only change: stress.rs import order and line width reformatted by cargo fmt -->
