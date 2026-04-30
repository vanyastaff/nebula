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

**Bulkhead Pool** (Release It! ch "Stability Patterns — Bulkhead"). Isolates resource exhaustion per topology so one depleted pool cannot cascade to unrelated paths. Five named topologies cover the full integration space: `Pooled`, `Resident`, `Service`, `Transport`, `Exclusive`. The `Resource` trait declares five associated types and seven core methods; topology traits add pool-specific recycle and broken-instance decisions. Long-running workers (`Daemon`) and pull-based subscriptions (`EventSource`) live in `nebula_engine::daemon` per ADR-0037 — canon §3.5 reserves "Resource" for pool/SDK clients.

## Public API

- `Resource` — core trait: 5 associated types (`Config`, `Runtime`, `Lease`, `Error`, `Credential`), 7 core methods (`create`, `check`, `shutdown`, `destroy`, `schema()`, `on_credential_refresh`, `on_credential_revoke`). The trait binds to credentials via `type Credential: Credential` per [ADR-0036](../../docs/adr/0036-resource-credential-adoption-auth-retirement.md); resources without an authenticated binding write `type Credential = NoCredential;` (re-exported as `nebula_resource::NoCredential`). The runtime projects `<Self::Credential as Credential>::Scheme` and threads it into `create` and rotation hooks.
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
- `TopologyRuntime` — enum dispatching to the 5 topology runtime variants.
- Topology traits: `Pooled`, `Resident`, `Service`, `Transport`, `Exclusive`.
- Topology runtimes: `PoolRuntime`, `ResidentRuntime`, `ServiceRuntime`, `TransportRuntime`, `ExclusiveRuntime`.
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

- API stability: `frontier` — 5 topologies, `Manager`, `ReleaseQueue`, and `ResourceGuard` are the authoritative lifecycle surface; topology runtime variants are actively evolving.
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

Long-running workers (`Daemon`) and pull-based event subscriptions (`EventSource`) live in `nebula_engine::daemon` per ADR-0037; this crate retains pool/SDK-client topologies only (canon §3.5).

### Shared resource pattern

When multiple workflows acquire the same `Resource` impl at the same scope,
the manager deduplicates by `(R::key(), ScopeLevel)` and — for topologies
backed by a single shared runtime — by the config `fingerprint()`. Exactly
one `Resource::create` invocation runs, and every acquirer receives a lease
that points at the same backing runtime.

This is the foundation of the "one bot, ten workflows" headline: a single
Telegram bot client serving many concurrent workflow nodes without
re-authenticating, re-warming connections, or contending for rate limits
across duplicate clients.

#### Telegram bot example

```rust,ignore
use std::sync::Arc;

use nebula_resource::{
    AcquireOptions, Manager, ResidentConfig, ScopeLevel,
    runtime::{TopologyRuntime, resident::ResidentRuntime},
};

// One bot, registered once at organization scope.
let manager = Arc::new(Manager::new());
let bot = TelegramBot::new(/* construct from credentials */);
let resident_rt = ResidentRuntime::<TelegramBot>::new(ResidentConfig::default());
manager.register(
    bot,
    bot_config,
    ScopeLevel::Organization(org_id),
    TopologyRuntime::Resident(resident_rt),
    None, None,
)?;

// 10 workflows, each acquiring concurrently, all share the one client.
let mut handles = Vec::new();
for _ in 0..10 {
    let mgr = Arc::clone(&manager);
    handles.push(tokio::spawn(async move {
        let ctx = build_workflow_resource_ctx(org_id);
        mgr.acquire_resident::<TelegramBot>(&ctx, &AcquireOptions::default()).await
    }));
}

// `Resource::create` was invoked exactly once; every acquirer holds a
// lease whose underlying `Arc` is pointer-equal to every other acquirer's.
```

Resident is the natural topology for a shared bot client; the same dedupe
guarantee applies to `Pooled` (one pool with N interchangeable instances),
`Service` (long-lived runtime with refreshable tokens), and `Transport`
(shared connection with multiplexed sessions). `Exclusive` deliberately
opts out — its semantics are "one caller at a time," not "one shared
instance."

Verification: see `crates/engine/tests/resource_integration.rs::shared_resource::cross_workflow_resource_sharing`
— 10 simulated workflows × 1 `TelegramBot` resource × Organization scope ⇒
exactly one `create` invocation, all 10 leases share the same `Arc`.

#### Invalidation triggers

- **Fingerprint change in `ResourceConfig`**. Calling `Manager::reload_config::<R>(new_config, &scope)` validates the new config, swaps it in, bumps the resource's `generation`, and emits `ResourceEvent::ConfigReloaded`. For `Pooled` topologies the pool's fingerprint atomic is updated so idle entries with the stale fingerprint are evicted on next acquire or release. `Resident` topologies keep the existing runtime alive until liveness fails (the rebuild then picks up the new config). No-op reloads (same fingerprint) short-circuit to `ReloadOutcome::NoChange` without bumping the generation.
- **Different `R::key()`**. Two distinct `Resource` impls — even configured identically — register under separate registry rows. `acquire_resident::<TelegramBot>` and `acquire_resident::<AlternateBot>` produce independent runtimes and can be replaced or shut down independently.
- **Different `ScopeLevel`**. The same `Resource` impl registered at `Organization(A)` and `Organization(B)` produces two independent instances; the registry's scope-aware `find_by_scope` does an exact match first and falls back to `Global` only when no exact match exists. Per-scope reloads / shutdowns affect only the matching scope.
- **Manager shutdown**. `Manager::shutdown()` cancels the shared token; in-flight acquires drain via `graceful_shutdown` per canon §11.4. After shutdown, every acquire returns `ErrorKind::Cancelled` — no leases are minted from a torn-down registry.
