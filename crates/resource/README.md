---
name: nebula-resource
role: Bulkhead Pool (Release It! ch "Stability Patterns — Bulkhead"; resource lifecycle acquire / health / release)
status: frontier
last-reviewed: 2026-04-17
canon-invariants: [L2-11.4, L2-13.3]
related: [nebula-core, nebula-schema, nebula-error, nebula-resilience, nebula-credential, nebula-action]
---

# nebula-resource

## Purpose

External connections — database pools, HTTP clients, message brokers — are a primary failure surface in workflow engines. When an action creates its own client on demand and never releases it, pool exhaustion and orphaned handles accumulate silently. `nebula-resource` solves this by making the engine the owner of the resource lifecycle: acquire, health-check, hot-reload, and scope-bounded release are engine concerns, not per-action boilerplate. Actions receive a `ResourceGuard` that derefs to the lease type and releases on drop; the engine ensures the backing runtime is healthy before granting the guard.

## Role

**Bulkhead Pool** (Release It! ch "Stability Patterns — Bulkhead"). Isolates resource exhaustion per topology so one depleted pool cannot cascade to unrelated paths. Seven named topologies cover the full integration space: `Pooled`, `Resident`, `Service`, `Transport`, `Exclusive`, `EventSource`, `Daemon`. The `Resource` trait declares five associated types and five core methods; topology traits add pool-specific recycle and broken-instance decisions.

## Public API

- `Resource` — core trait: 5 associated types (`Config`, `Runtime`, `Lease`, `Error`, `Credential`), 5 core methods (`create`, `check`, `shutdown`, `destroy`, `schema()`).
- `ResourceGuard` — RAII lease guard with `Owned`/`Guarded`/`Shared` modes; deref to lease type, release on drop.
- `Manager`, `ManagerConfig`, `RegisterOptions` — central registry with acquire dispatch and shutdown coordination.
- `Registry`, `AnyManagedResource` — type-erased storage for registered resource instances.
- `ResourceMetadata` — static descriptor: key, name, description, tags.
- `ResourceMetadataBuilder` — fluent builder for `ResourceMetadata` via `.with_schema()`, `.with_version()`, `.build()`.
- `Cell` — lock-free `ArcSwap`-based cell for resident topologies.
- `ReleaseQueue` — background worker pool for async cleanup. Drain on crash is best-effort; see §11.4 canon note.
- `DrainTimeoutPolicy` — policy controlling drain operation timeout.
- `Error`, `ErrorKind`, `ErrorScope` — typed error with retry classification.
- `ResourceContext` — execution context with cancellation and capability traits (`HasResources`, `HasCredentials`).
- `ScopeLevel` — re-exported from `nebula_core::ScopeLevel`.
- `ResourcePhase`, `ResourceStatus` — lifecycle phase tracking for observability.
- `ResourceEvent` — lifecycle events (`Acquired`, `Released`, `HealthCheck`, `Recycled`, etc.).
- `ResourceOpsMetrics`, `ResourceOpsSnapshot` — registry-backed operation counters.
- `RecoveryGate`, `RecoveryGateConfig`, `WatchdogHandle`, `WatchdogConfig` — recovery patterns.
- `AcquireResilience`, `AcquireRetryConfig` — resilience configuration for acquire paths.
- `TopologyRuntime` — enum dispatching to the 7 topology runtime variants.
- Topology traits: `Pooled`, `Resident`, `Service`, `Transport`, `Exclusive`, `EventSource`, `Daemon`.
- Topology runtimes: `PoolRuntime`, `ResidentRuntime`, `ServiceRuntime`, `TransportRuntime`, `ExclusiveRuntime`, `EventSourceRuntime`, `DaemonRuntime`.
- `#[derive(Resource)]`, `#[derive(ClassifyError)]` — proc-macro derivations.
- `resource_key!` — macro for declaring resource keys.

## Contract

- **[L2-§11.4]** Resource lifecycle (acquire → use → release) is engine-owned. Async release is best-effort on crash; orphaned resources rely on the next process to drain via `ReleaseQueue`. Authors must not assume "release ran" without an explicit checkpoint. Seam: `crates/resource/src/release_queue.rs` — `ReleaseQueue`. Test: `crates/resource` unit tests.
- **[L2-§13.3]** Acquire → use → release for Resource-backed steps must be attributable in durable journal or an operator-visible trace. Not only ephemeral logs. Seam: `ResourceEvent` variants emitted through the engine observability path.
- **[L1-§11.4]** For long-lived exclusive/external resources (locks, leased cloud instances), deployments need an external TTL / dead-man strategy; Nebula v1 does not provide an external lease arbiter.
- **Bulkhead isolation** — `ErrorKind::Backpressure` signals pool exhaustion; callers decide retry policy. Pool depletion does not cascade across topology boundaries.

## Non-goals

- Not a connection driver — resource implementations supply the actual client (sqlx pool, reqwest client, etc.); this crate owns the lifecycle wrapper.
- Not a retry pipeline — retry around outbound calls inside `create`/`check` uses `nebula-resilience` directly. The `AcquireResilience` type configures the acquire-path retry only.
- Not a secret holder — `Credential` associated type is injected before `create()`; secrets are managed by `nebula-credential`.
- Not an expression evaluator — resource `Config` comes from `nebula-schema`-validated parameters; expression resolution is `nebula-expression`'s job.

## Maturity

See `docs/MATURITY.md` row for `nebula-resource`.

- API stability: `frontier` — 7 topologies, `Manager`, `ReleaseQueue`, and `ResourceGuard` are the authoritative lifecycle surface; topology runtime variants are actively evolving.
- `#![forbid(unsafe_code)]` enforced, `#![warn(missing_docs)]` active.
- Integration tests: 0 in `tests/`; lifecycle covered by unit tests per topology.

## Related

- Canon: `docs/PRODUCT_CANON.md` §11.4 (resource lifecycle contract — acquire/health/release; orphan drain), §13.3 (lifecycle visibility in journal/trace).
- Integration model: `docs/INTEGRATION_MODEL.md` §`nebula-resource`.
- Siblings: `nebula-core` (`ResourceKey`, `ExecutionId`), `nebula-credential` (injected before `create()`), `nebula-action` (`ResourceAction` trait, `ResourceAccessor` capability), `nebula-resilience` (acquire-path and outbound-call retry).

## Appendix

### Drain mechanism types (evicted from PRODUCT_CANON.md §11.4)

Orphaned resources are drained by the next process through:

- `DrainTimeoutPolicy` — policy controlling how long a drain operation waits.
- `ReleaseQueue` (`src/release_queue.rs`) — the queue of releases awaiting drain.

These types are L4 implementation detail — rename/refactor without canon revision. The L2 invariant ("async release is best-effort on crash; orphaned resources rely on next process") lives in canon §11.4.

### Topology reference

| Topology | Use case | Instance model |
|---|---|---|
| `Pooled` | Databases (Postgres, Redis) | N interchangeable instances with checkout/recycle |
| `Resident` | HTTP clients (`reqwest::Client`) | One shared instance, clone on acquire |
| `Service` | OAuth APIs, token-gated services | Long-lived runtime with short-lived tokens |
| `Transport` | WebSocket, gRPC channels | Shared connection with multiplexed sessions |
| `Exclusive` | File locks, hardware ports | One caller at a time via semaphore(1) |
| `EventSource` | Webhooks, SSE, change streams | Pull-based event subscription |
| `Daemon` | Background workers, watchers | Background run loop with restart policy |
