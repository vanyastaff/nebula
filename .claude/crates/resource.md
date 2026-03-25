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

## Key API changes (2026-03-22)
- `fn cleanup` renamed to `fn destroy` across the trait, all impls, pool internals (`destroy_with_hooks`), and `HookEvent::Cleanup` → `HookEvent::Destroy`
- `fn is_broken` now returns `BrokenCheck` enum (`Healthy`/`Broken`), not `bool`
- `fn is_reusable` and `fn recycle` now take `_meta: &InstanceMetadata` as second argument — the old `is_reusable_with_meta` / `recycle_with_meta` methods are gone
- `BrokenCheck` is `pub` in `nebula_resource::resource`; `InstanceMetadata` is `pub` in `nebula_resource::pool`
- `Scope` enum is now `#[non_exhaustive]`

<!-- reviewed: 2026-03-25 -->
<!-- reviewed: 2026-03-25 -->
