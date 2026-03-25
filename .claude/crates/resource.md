# nebula-resource

v2 rewrite — topology-agnostic resource management.

## Architecture (v2)
- Phase 1: primitives (Error, Ctx, Resource trait, Handle, Cell, ReleaseQueue, State, Options)
- Phase 2: 7 topology traits in `topology/`
- Phase 3: recovery layer (`recovery/`) + integration config (`integration/`)
- Resource trait uses RPITIT; `Ctx` is dyn-compatible with `ctx_ext::<T>()` free fn
- `Manager` is a placeholder pending Phase 5

## Invariants
- `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`
- `Resource::key()` is a method, not `const KEY`
- `ErrorKind` determines retry: Transient/Exhausted = retryable
- `AnyResource` = dyn-safe marker for heterogeneous registration
- Topology traits extend `Resource` with RPITIT + default impls
- `RecoveryGate` uses CAS via `ArcSwap`; ticket drop auto-fails with backoff
- Deprecated `Context`/`Scope` in `compat` (for migration)

## Traps
- `Ctx::ext_raw()` → use `ctx_ext::<T>()` instead
- `ResourceHandle::detach()` on Shared returns None
- `ReleaseQueue::submit` drops silently if all channels full

## Relations
- Depends on: nebula-core
- Depended on by: nebula-action, nebula-plugin, nebula-engine, nebula-webhook

<!-- reviewed: 2026-03-25 -->
