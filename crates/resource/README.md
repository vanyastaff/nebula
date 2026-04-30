---
name: nebula-resource
role: Bulkhead Pool (Release It! ch "Stability Patterns — Bulkhead"; resource lifecycle acquire / health / release)
status: frontier
last-reviewed: 2026-04-29
canon-invariants: [L2-11.4, L2-13.3]
related: [nebula-core, nebula-schema, nebula-error, nebula-resilience, nebula-credential, nebula-action]
---

# nebula-resource

## Purpose

External connections — database pools, HTTP clients, message brokers — are a primary failure surface in workflow engines. When an action creates its own client on demand and never releases it, pool exhaustion and orphaned handles accumulate silently. `nebula-resource` solves this by making the engine the owner of the resource lifecycle: acquire, health-check, hot-reload, and scope-bounded release are engine concerns, not per-action boilerplate. Actions receive a `ResourceGuard` that derefs to the lease type and releases on drop; the engine ensures the backing runtime is healthy before granting the guard.

## Role

**Bulkhead Pool** (Release It! ch "Stability Patterns — Bulkhead"). Isolates resource exhaustion per topology so one depleted pool cannot cascade to unrelated paths. Five named topologies cover the full integration space: `Pooled`, `Resident`, `Service`, `Transport`, `Exclusive`. The `Resource` trait declares four associated types and lifecycle methods; topology traits add pool-specific recycle and broken-instance decisions. Long-running workers (`Daemon`) and pull-based subscriptions (`EventSource`) live in `nebula_engine::daemon` per ADR-0037 — canon §3.5 reserves "Resource" for pool/SDK clients.

## Public API (v4 — M6 / dependency redesign, 2026-04-29)

The v4 surface lands per ADR-0044 (supersedes ADR-0036) — singular `type Credential` is dropped in favor of typed credential **slot fields** declared via `#[credential(key = "…")]` field attributes on the resource struct. Multi-credential resources are now natural; per-slot rotation hooks land via `Resource::on_credential_refresh(&mut self, slot_name)`.

### `Resource` trait — 4 associated types, slot fields on Self

```rust
pub trait Resource: Send + Sync + 'static {
    type Config:  ResourceConfig;
    type Runtime: Send + Sync + 'static;
    type Lease:   Send + Sync + 'static;
    type Error:   std::error::Error + Send + Sync + Into<crate::Error> + 'static;

    fn key() -> ResourceKey;

    /// Slot fields are populated on `&self` BEFORE create runs.
    fn create(&self, config: &Self::Config, ctx: &ResourceContext)
        -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    /// Per-slot rotation: receive the slot name that rotated.
    fn on_credential_refresh(&mut self, slot_name: &str)
        -> impl Future<Output = Result<(), Self::Error>> + Send { /* default no-op */ }

    fn check    (&self, runtime: &Self::Runtime) -> impl Future<Output = Result<(), Self::Error>> + Send;
    fn shutdown (&self, runtime: &Self::Runtime) -> impl Future<Output = Result<(), Self::Error>> + Send;
    fn destroy  (&self, runtime: Self::Runtime)  -> impl Future<Output = Result<(), Self::Error>> + Send;
}
```

**`type Credential` was dropped** per ADR-0044. There is no longer a singular credential associated type; resources declare credentials as slot fields. The opt-out alias `NoCredential` is no longer required — resources without credentials simply have no `#[credential]` fields.

### Slot-binding pattern — `#[derive(Resource)]` + `#[credential]` field attrs

```rust
use nebula_credential::{Credential, CredentialGuard};
use nebula_resource::Resource;

#[derive(Resource)]
#[resource(
    key      = "postgres",
    topology = "pool",
    config   = PostgresConfig,
    runtime  = PgPool,
    lease    = PgConnection,
    error    = PgError,
)]
struct Postgres {
    #[credential(key = "db_auth", purpose = "Main DB auth")]
    db_auth: CredentialGuard<<DatabaseCredential as Credential>::Scheme>,

    #[credential(key = "audit", purpose = "Audit log auth")]
    audit: Option<CredentialGuard<<AuditCredential as Credential>::Scheme>>,
}
```

The derive emits:
- `impl Resource for Postgres { type Config = …; type Runtime = …; … fn key() … }` with a `todo!()` `create` body — the implementor supplies it.
- `impl DeclaresDependencies for Postgres` enumerating the credential slot fields so the engine can resolve each before `create` runs.
- A topology marker (`Pooled` / `Resident` / `Service` / `Transport` / `Exclusive`) is **not** auto-derived — the topology attribute is informational; the implementor still writes `impl Pooled for Postgres { … }` (or the chosen topology trait) by hand.

#### Field-type matrix (mirrors `#[derive(Action)]`)

| Field type | Semantics |
|---|---|
| `CredentialGuard<C::Scheme>` | required + eager |
| `Option<CredentialGuard<C::Scheme>>` | optional + eager |
| `Lazy<CredentialGuard<C::Scheme>>` | required + lazy |
| `Option<Lazy<CredentialGuard<C::Scheme>>>` | optional + lazy |

### `Manager` registration

| API | Use |
|---|---|
| `Manager::register::<R>(...)` | Typed registration with a fully-resolved `R` value. Zero overhead. |
| `Manager::register_pooled::<R>(...)` / `_resident_with` / `_service_with` / `_transport_with` / `_exclusive_with` | Topology-specific shortcuts. |
| `Manager::register_from_value::<R>(json, expr_engine, …)` | JSON-driven registration: resolves `{{ … }}` templates → validates against `<R::Config as HasSchema>::schema()` → deserializes Config → constructs `R` with slot fields → registers. Phase 9 cross-crate path. |

The framework resolves declared `#[credential]` slots **before** invoking `Resource::create` — implementations read credentials directly off `&self`.

### Other public API

- `ResourceGuard` — RAII lease guard with `Owned`/`Guarded`/`Shared` modes; deref to lease type, release on drop.
- `ResourceRef<R>` — lazy reference type holding a `ResourceId` string + `PhantomData<R>`. Resolves to a `ResourceGuard<R>` via `.resolve(ctx).await`. New in Phase 1.
- `ManagerConfig`, `RegisterOptions` — configuration surface.
- `Registry`, `AnyManagedResource` — type-erased storage for registered resource instances.
- `ResourceMetadata`, `ResourceMetadataBuilder` — static descriptor: key, name, description, schema, version, tags.
- `ResourceConfig` — operational config trait (no secrets); supertype `HasSchema`.
- `Cell` — lock-free `ArcSwap`-based cell for resident topologies.
- `ReleaseQueue` — background worker pool for async cleanup. Drain on crash is best-effort; see §11.4 canon note.
- `DrainTimeoutPolicy` — policy controlling drain operation timeout.
- `Error`, `ErrorKind`, `ErrorScope` — typed error with retry classification.
- `ResourceContext` — execution context with cancellation and capability traits (`HasResources`, `HasCredentials`).
- `ScopeLevel` — re-exported from `nebula_core::ScopeLevel`.
- `ResourcePhase`, `ResourceStatus` — lifecycle phase tracking for observability.
- `ResourceEvent` — lifecycle events (`Acquired`, `Released`, `HealthCheck`, `Recycled`, `ConfigReloaded`, …).
- `ResourceOpsMetrics`, `ResourceOpsSnapshot` — registry-backed operation counters.
- `RecoveryGate`, `RecoveryGateConfig`, `WatchdogHandle`, `WatchdogConfig` — recovery patterns.
- `AcquireResilience`, `AcquireRetryConfig` — resilience configuration for acquire paths.
- `TopologyRuntime` — enum dispatching to the 5 topology runtime variants.
- Topology traits: `Pooled`, `Resident`, `Service`, `Transport`, `Exclusive`.
- Topology runtimes: `PoolRuntime`, `ResidentRuntime`, `ServiceRuntime`, `TransportRuntime`, `ExclusiveRuntime`.
- `#[derive(Resource)]`, `#[derive(ClassifyError)]` — proc-macro derivations.
- `resource_key!` — macro for declaring resource keys.

## Migration recipe (pre-v4 → v4)

The Phase 4 / ADR-0044 break is hard. To migrate an existing `Resource` impl:

1. **Drop `type Credential`.** Move the credential dependency to a `#[credential(key = "…")]` slot field on the struct. Change `Resource::Credential` references in your code to read off the slot field directly.
2. **Drop the `scheme: &<R::Credential as Credential>::Scheme` parameter** from `create`. The framework populates slot fields before `create` runs; read the credential off `&self.<slot_field>` instead.
3. **Replace `on_credential_refresh(scheme, ctx)` with `on_credential_refresh(&mut self, slot_name)`.** The new hook receives the slot name that rotated; the new credential is already in the slot field on `&mut self`. Multi-credential resources can branch on `slot_name` to refresh only the affected sub-system.
4. **Drop `nebula_credential::NoCredential`.** Resources without credentials simply have no `#[credential]` fields. The `NoCredential` opt-out is no longer needed.
5. **For `#[derive(Resource)]`** (new), parse `#[resource(key, topology, config, runtime, lease, error)]` struct attribute; the derive emits `Resource` trait shape (with `todo!()` `create` body — you supply it) and a `DeclaresDependencies` impl enumerating slot fields. Topology traits (`Pooled`, `Resident`, etc.) are still hand-written.
6. **Update test code** — `register*<R>` API now takes a fully-resolved `R` value; the previous `acquire_*_default` shorthand is folded into the single `acquire_*` family.
7. **For JSON-driven registration**, use `Manager::register_from_value::<R>(json, expr_engine, …)` — this is the Phase 9 cross-crate path through `schema → validator → expression`.

The deferred per-slot rotation reverse-index + fan-out subsystem is documented at `.ai-factory/PHASE4_BLOCKED.md` and tracked in ROADMAP §M11.5. The trait-shape changes ship complete; the engine-side fan-out machinery for delivering slot-name rotation events is a separate milestone.

## Runnable examples

- `cargo run -p nebula-examples --example m6_postgres_pool` — `Pooled` topology + `ResourceAction` for per-execution test schema (configure / cleanup ordering)
- `cargo run -p nebula-examples --example m6_resident_http` — `Resident` topology + OAuth-style credential refresh hook
- `cargo run -p nebula-examples --example m6_telegram_multi_workflow` — `Resident` topology + cross-workflow shared-resource dedupe (1 bot, 10 workflows, 1 `Resource::create`)

The headline patterns and topology selection guidance are distilled into `crates/resource/docs/topology-reference.md`.

## Contract

- **[L2-§11.4]** Resource lifecycle (acquire → use → release) is engine-owned. Async release is best-effort on crash; orphaned resources rely on the next process to drain via `ReleaseQueue`. Authors must not assume "release ran" without an explicit checkpoint. Seam: `crates/resource/src/release_queue.rs` — `ReleaseQueue`. Test: `crates/resource` unit tests.
- **[L2-§13.3]** Acquire → use → release for Resource-backed steps must be attributable in durable journal or an operator-visible trace. Not only ephemeral logs. Seam: `ResourceEvent` variants emitted through the engine observability path.
- **[L1-§11.4]** For long-lived exclusive/external resources (locks, leased cloud instances), deployments need an external TTL / dead-man strategy; Nebula v1 does not provide an external lease arbiter.
- **Bulkhead isolation** — `ErrorKind::Backpressure` signals pool exhaustion; callers decide retry policy. Pool depletion does not cascade across topology boundaries.

## Non-goals

- Not a connection driver — resource implementations supply the actual client (sqlx pool, reqwest client, etc.); this crate owns the lifecycle wrapper.
- Not a retry pipeline — retry around outbound calls inside `create`/`check` uses `nebula-resilience` directly. The `AcquireResilience` type configures the acquire-path retry only.
- Not a secret holder — credentials are populated into slot fields by the framework; secret material is managed by `nebula-credential`.
- Not an expression evaluator — resource `Config` comes from `nebula-schema`-validated parameters; expression resolution is `nebula-expression`'s job. `Manager::register_from_value` orchestrates the pipeline but the evaluator itself stays out.

## Maturity

See `docs/MATURITY.md` row for `nebula-resource`.

- API stability: `frontier` — slot-binding pattern (ADR-0044) shipped; 5 topologies, `Manager`, `ReleaseQueue`, and `ResourceGuard` are the authoritative lifecycle surface; topology runtime variants are actively evolving.
- `#![forbid(unsafe_code)]` enforced, `#![warn(missing_docs)]` active.
- Integration tests: shared-resource cross-workflow path is verified in `crates/engine/tests/resource_integration.rs::shared_resource::cross_workflow_resource_sharing`.
- Per-slot rotation fan-out: deferred — see `.ai-factory/PHASE4_BLOCKED.md` and ROADMAP §M11.5.

## Related

- Canon: `docs/PRODUCT_CANON.md` §11.4 (resource lifecycle contract — acquire/health/release; orphan drain), §13.3 (lifecycle visibility in journal/trace).
- ADRs: `docs/adr/0044-supersede-0036-resource-credential-singular.md` (supersedes ADR-0036), `docs/adr/0042-node-binding-mechanism.md`, `docs/adr/0043-dependency-declaration-dx.md`.
- Integration model: `docs/INTEGRATION_MODEL.md` §`nebula-resource`.
- Siblings: `nebula-core` (`ResourceKey`, `ExecutionId`, `Dependencies`), `nebula-credential` (`CredentialGuard` populated by framework), `nebula-action` (`ResourceAction` trait, `ResourceProduces<R>` marker), `nebula-resilience` (acquire-path and outbound-call retry).

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
