# nebula-resource

v2 rewrite — topology-agnostic resource management with 7 topology patterns.

## Architecture (v2)
- Phase 1 complete: primitives layer (Error, Ctx, Resource trait, Handle, Cell, ReleaseQueue, State, Options)
- Resource trait uses RPITIT (no async_trait) with 5 associated types + 4 lifecycle methods
- `Ctx` trait is dyn-compatible; typed extensions via `ctx_ext::<T>()` free function (not generic method)
- `ResourceHandle` has 3 variants: Owned, Guarded (exclusive + pool return), Shared (Arc-wrapped)
- `Cell<T>` wraps `ArcSwapOption` for lock-free resident topology
- `ReleaseQueue` distributes cleanup across N workers + 1 fallback

## Invariants
- `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]` on the crate
- `Resource::key()` is a method, not `const KEY` (domain-key macro not const-compatible)
- `ErrorKind` determines retry policy: Transient/Exhausted = retryable, rest = not
- Backward-compat: `Context`/`Scope` in `compat` module (deprecated, for webhook/engine migration)
- `AnyResource` trait = dyn-safe marker for heterogeneous resource registration
- `Manager` is a placeholder struct pending Phase 5

## Traps
- `Ctx::ext_raw()` returns raw `dyn Any` — always use `ctx_ext::<T>()` free function for type safety
- `ResourceHandle::detach()` on Shared returns None (can't take Arc with other holders)
- `ReleaseQueue::submit` drops tasks silently if both primary and fallback channels are full

## Relations
- Depends on: nebula-core (ResourceKey, ExecutionId, WorkflowId)
- Depended on by: nebula-action, nebula-plugin, nebula-engine, nebula-webhook

<!-- reviewed: 2026-03-25 -->
