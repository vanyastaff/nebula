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

External connections — database pools, HTTP clients, message brokers — are a primary failure surface in workflow engines. When an action creates its own client on demand and never releases it, pool exhaustion and orphaned handles accumulate silently. `nebula-resource` solves this by making the engine the owner of the resource lifecycle: acquire, health-check, hot-reload, and scope-bounded release are engine concerns, not per-action boilerplate. Actions receive a `ResourceGuard` that derefs to `R::Runtime` and releases on drop; the engine ensures the backing runtime is healthy before granting the guard.

## Role

**Bulkhead Pool** (Release It! ch "Stability Patterns — Bulkhead"). Isolates resource exhaustion per topology so one depleted pool cannot cascade to unrelated paths. Three built-in topologies cover the integration space: `Pooled` (N interchangeable stateful instances), `Resident` (one shared instance, cloned on acquire), and `Bounded` (a runtime concurrency cap with no warm idle pool — capped / exclusive / unbounded). The `Provider` trait declares three associated types (`Config`, `Instance`, `Topology`) and lifecycle methods; per-topology hook traits (`PoolProvider` / `ResidentProvider` / `BoundedProvider`) add recycle / liveness / reset decisions. The framework owns the acquire loop and the credential-revoke fence; a custom `Topology<R>` impl can register through the same `Manager`. Long-running workers (`Daemon`) and pull-based subscriptions (`EventSource`) live in `nebula_engine::daemon` — canon §3.5 reserves "Resource" for pool/SDK clients.

## Public API (v4 — slot-binding pattern, 2026-04-29)

The v4 surface — singular `type Credential` is dropped in favor of typed credential **slot fields** declared via `#[credential(key = "…")]` field attributes on the resource struct. Each slot field is a lock-free `SlotCell<CredentialGuard<C>>` the framework populates and rotates through `&self`; the derive emits a `<field>_slot()` read accessor. Multi-credential resources are now natural; per-slot rotation lands via `Resource::on_credential_refresh(&self, slot_name, runtime)` with a companion `Resource::on_credential_revoke(&self, slot_name, runtime)`.

### `Resource` trait — 2 associated types, slot fields on Self

```rust
pub trait Resource: Send + Sync + 'static {
    type Config:  ResourceConfig;
    type Runtime: Send + Sync + 'static;

    fn key() -> ResourceKey;

    /// Slot cells are populated on `&self` BEFORE create runs; read the
    /// resolved guard via the derive-emitted `<field>_slot()` accessor.
    /// All lifecycle methods return the crate's typed `Error` — author
    /// error enums convert in via `#[derive(ClassifyError)]` + `?`.
    fn create(&self, config: &Self::Config, ctx: &ResourceContext)
        -> impl Future<Output = Result<Self::Runtime, Error>> + Send;

    /// Per-slot rotation: the engine swaps the rotated guard into the slot
    /// cell, then calls this with the slot name + live `Runtime`. `&self` —
    /// re-auth acts on `runtime`'s interior mutability. Default no-op.
    fn on_credential_refresh(&self, slot_name: &str, runtime: &Self::Runtime)
        -> impl Future<Output = Result<(), Error>> + Send { /* default no-op */ }

    /// Per-slot revocation: post-invocation the resource emits no further
    /// authenticated traffic on the revoked credential. Default no-op.
    fn on_credential_revoke(&self, slot_name: &str, runtime: &Self::Runtime)
        -> impl Future<Output = Result<(), Error>> + Send { /* default no-op */ }

    fn check    (&self, runtime: &Self::Runtime) -> impl Future<Output = Result<(), Error>> + Send;
    fn shutdown (&self, runtime: &Self::Runtime) -> impl Future<Output = Result<(), Error>> + Send;
    fn destroy  (&self, runtime: Self::Runtime)  -> impl Future<Output = Result<(), Error>> + Send;
}
```

The per-resource **credential epoch** (an order-sensitive fold over every
`#[credential]` slot's generation, used by the rotation reconcile) lives on a
separate `HasCredentialSlots` trait, emitted by `#[derive(ResourceSlots)]` —
never hand-maintained.

**`type Credential` was dropped.** There is no longer a singular credential associated type; resources declare credentials as slot fields. The opt-out alias `NoCredential` is no longer required — resources without credentials simply have no `#[credential]` fields.

### Slot-binding pattern — `#[derive(ResourceSlots)]` + hand-written `impl Resource`

The **two-derive pattern**: `#[derive(ResourceSlots)]` emits only the slot
plumbing; you supply a hand-written `impl Resource` with real `create` /
`check` / `shutdown` / `destroy` bodies. No container `#[resource(...)]`
attribute is needed or accepted.

```rust
use nebula_credential::CredentialGuard;
use nebula_resource::{Resource, ResourceSlots, SlotCell};

#[derive(ResourceSlots)]
struct Postgres {
    #[credential(key = "db_auth", purpose = "Main DB auth")]
    db_auth: SlotCell<CredentialGuard<DatabaseCredential>>,

    #[credential(key = "audit", purpose = "Audit log auth")]
    audit: SlotCell<CredentialGuard<AuditCredential>>,
}

impl Resource for Postgres {
    type Config  = PostgresConfig;
    type Runtime = PgPool;

    fn key() -> ResourceKey { resource_key!("postgres") }

    async fn create(&self, config: &PostgresConfig, _ctx: &ResourceContext)
        -> Result<PgPool, PgError>
    {
        // read resolved credentials through derive-emitted accessors
        let guard = self.db_auth_slot().expect("db_auth slot must be bound");
        // … build pool …
    }
    // check / shutdown / destroy …
}
```

`#[derive(ResourceSlots)]` emits:
- `impl DeclaresDependencies for Postgres` — enumerates credential slot fields so the engine resolves each before `create` runs.
- A read accessor per slot field: `pub fn <field>_slot(&self) -> Option<Arc<CredentialGuard<C>>>` returning the resolved guard, or `None` until the framework binds it. Implementations read the credential through this accessor — never off the raw cell field. A pure derive cannot add or rewrite struct fields and `ManagedResource` hands out `Arc<R>` (no `&mut R`), so the author declares the `SlotCell` cell and the framework populates / rotates it through `&self` via `SlotCell::store`.
- `impl HasCredentialSlots for Postgres` — order-sensitive epoch fold used by the engine's hot-reload path.

Topology traits (`Pooled`, `Resident`, etc.) are always hand-written — the derive does not emit them.

#### Field-type shape

The generated `<field>_slot()` accessor emits one fixed body, so a `#[credential]` slot field must currently be **exactly** `SlotCell<CredentialGuard<C>>` (required + eager). `Option<…>`- and `Lazy<…>`-wrapped slots are a hard compile error at the derive site until the accessor is generalized — declare an unconditional cell and treat the accessor's `None` (unbound) return as the optional/lazy case.

### `Manager` registration

Registration goes through **one funnel**: `Manager::register::<R>(spec: RegistrationSpec<R>)`. The per-topology `register_<topo>[_with]` shorthands and the 3-deep delegation chain were removed — `RegistrationSpec<R>` is a plain struct with public fields and no builder:

```rust
manager.register(RegistrationSpec {
    resource,                                  // fully-constructed R, all #[credential] slots resolved
    config,                                    // validated on register
    scope: ScopeLevel::Global,
    slot_identity: SlotIdentity::Unbound,      // structural anti-bleed identity (see below)
    topology: TopologyRuntime::Resident(resident_runtime),
    acquire: Manager::erased_acquire_resident_for::<R>(),
    recovery_gate: None,                       // Option<Arc<RecoveryGate>>
})?;
```

`slot_identity` is the **collision-free structural cross-tenant barrier** (`SlotIdentity::{Unbound, Structural(Arc<[(String, String)]>)}`): two registrations of the same resource type at the same `scope` whose resolved `(slot, credential)` bindings differ occupy **distinct** registry rows with **distinct** runtimes. Equality/hash is over the exact pair list — a distinct resolved credential is a distinct identity *by construction*, not a hash digest, so there is no collision space. `SlotIdentity::Unbound` preserves the historical single-row-per-`(key, scope)` dedup contract and carries no secret bytes. The JSON/`{{ … }}` engine-facing entry is `Manager::register_resolved::<R>(…)` (an internal positional ABI the engine registrar drives, not a general-purpose API).

The framework resolves declared `#[credential]` slots **before** invoking `Resource::create` — implementations read each resolved credential through the derive-emitted `self.<field>_slot()` accessor (`Option<Arc<CredentialGuard<C>>>`), handling the `None` (unbound) case explicitly.

### Other public API

- `ResourceGuard` — RAII runtime guard with `Owned`/`Guarded` modes; derefs to `R::Runtime`, releases on drop.
- `ResourceRef<R>` — lazy reference type holding a `ResourceId` string + `PhantomData<R>`. Resolves to a `ResourceGuard<R>` via `.resolve(ctx).await`.
- `RegistrationSpec` — the single registration param aggregate (see above).
- `SlotIdentity` — collision-free structural resolved-credential identity (`Unbound` / `Structural`); the cross-tenant barrier.
- `ManagerConfig`, `RegisterOptions` — configuration surface.
- `Registry`, `AnyManagedResource`, `LookupOutcome` — type-erased storage + lookup result for registered resource instances.
- `ResourceMetadata` — static descriptor: key, name, description, schema, version, tags.
- `ResourceConfig` — operational config trait (no secrets); supertype `HasSchema`.
- `SlotCell` — lock-free `ArcSwap`-based slot cell the framework populates/rotates (the public cell type; the internal `Cell` alias is no longer exported).
- `ReleaseQueue` — background worker pool for async cleanup. Drain on crash is best-effort; see §11.4 canon note.
- `DrainTimeoutPolicy` — policy controlling drain operation timeout.
- `ReloadOutcome` — result of `Manager::reload_config` (`NoChange` / `SwappedImmediately`).
- `Error`, `ErrorKind`, `ErrorScope` — typed error with retry classification.
- `ResourceContext` — execution context with cancellation and capability traits (`HasResources`, `HasCredentials`).
- `ScopeLevel` — re-exported from `nebula_core::ScopeLevel`.
- `ResourcePhase`, `ResourceStatus` — lifecycle phase tracking for observability.
- `ResourceEvent` — lifecycle events (`Registered`, `Removed`, `AcquireSuccess`, `AcquireFailed`, `Released`, `HealthChanged`, `ConfigReloaded`, `RetryAttempt`, `BackpressureDetected`, `RecoveryGateChanged`, `SlotRefreshed`, `SlotRevoked`, `SlotRefreshFailed`, `SlotRevokeFailed`).
- `ResourceOpsMetrics`, `ResourceOpsSnapshot` — registry-backed operation counters.
- `RecoveryGate`, `RecoveryGateConfig`, `RecoveryTicket`, `RecoveryWaiter`, `GateState` — thundering-herd recovery gate.
- Open `Topology<R>` trait + framework topology structs `Pooled<R>` / `Resident<R>` / `Bounded<R>` (reached monomorphically through `Provider::Topology`; no dispatch enum — the framework owns the acquire loop).
- Per-topology hook traits: `PoolProvider`, `ResidentProvider`, `BoundedProvider`.
- Topology configs / constructors: `PoolConfig`, `ResidentConfig`, `BoundedMode` (`Bounded::capped`/`exclusive`/`unbounded`).
- `CheckCost` — relative `check` probe cost driving the maintenance reaper's health-probe cadence.
- `#[derive(ResourceSlots)]`, `#[derive(ClassifyError)]` — proc-macro derivations.
- `resource_key!` — macro for declaring resource keys.

## Migration recipe (pre-v4 → v4)

The slot-binding break is hard. To migrate an existing `Resource` impl:

1. **Drop `type Credential`.** Move the credential dependency to a `#[credential(key = "…")]` slot field of type `SlotCell<CredentialGuard<C>>` on the struct, constructed with `SlotCell::empty()`. Change `Resource::Credential` references to read through the derive-emitted `self.<field>_slot()` accessor.
2. **Drop the `scheme: &<R::Credential as Credential>::Scheme` parameter** from `create`. The framework populates the slot cells before `create` runs; read the resolved guard via `self.<field>_slot()` (`Option<Arc<CredentialGuard<C>>>`) and handle the `None` (unbound) case explicitly.
3. **Replace `on_credential_refresh(scheme, ctx)` with `on_credential_refresh(&self, slot_name, runtime)`** and add an `on_credential_revoke(&self, slot_name, runtime)` override where the resource held revoke logic. The engine swaps the rotated guard into the slot cell before the call; `&self` is an immutable descriptor, so blue-green / re-auth acts on `runtime`'s interior mutability. Multi-credential resources can branch on `slot_name` to refresh only the affected sub-system.
4. **Drop `nebula_credential::NoCredential`.** Resources without credentials simply have no `#[credential]` fields. The `NoCredential` opt-out is no longer needed.
5. **Use the two-derive pattern**: annotate the struct with `#[derive(ResourceSlots)]`; write a hand-written `impl Resource` with real `create` / `check` / `shutdown` / `destroy` bodies. No `#[resource(...)]` container attribute. Topology traits (`Pooled`, `Resident`, etc.) are still hand-written.
6. **Update test code** — registration now goes through one funnel: `Manager::register::<R>(RegistrationSpec { resource, config, scope, slot_identity, topology, acquire, resilience, recovery_gate })`. The per-topology `register_<topo>[_with]` shorthands and the previous `acquire_*_default` shorthand were removed; acquire is the single `acquire_<topo>` / `acquire_<topo>_for_identity` family (or the erased `acquire_erased_for`).
7. **For credential slot identity**, pass `SlotIdentity::Unbound` for the historical single-row dedup, or build a `SlotIdentity::Structural` from the resolved `(slot, credential)` pairs for per-binding row separation. The old `u64` `slot_identity` digest was removed.

The trait-shape changes ship complete; the engine-side fan-out machinery for delivering slot-name rotation events lands in a follow-up.

## Runnable examples

- `cargo run -p nebula-examples --example resource_postgres_pool` — `Pooled` topology + `ResourceAction` for per-execution test schema (configure / cleanup ordering)
- `cargo run -p nebula-examples --example resource_resident_http` — `Resident` topology + OAuth-style credential refresh hook
- `cargo run -p nebula-examples --example resource_telegram_multi_workflow` — `Resident` topology + cross-workflow shared-resource dedupe (1 bot, 10 workflows, 1 `Resource::create`)

The headline patterns and topology selection guidance are distilled into `crates/resource/docs/topology-reference.md`.

## Contract

- **[L2-§11.4]** Resource lifecycle (acquire → use → release) is engine-owned. Async release is best-effort on crash; orphaned resources rely on the next process to drain via `ReleaseQueue`. Authors must not assume "release ran" without an explicit checkpoint. Seam: `crates/resource/src/release_queue.rs` — `ReleaseQueue`. Test: `crates/resource` unit tests.
- **[L2-§13.3]** Acquire → use → release for Resource-backed steps must be attributable in durable journal or an operator-visible trace. Not only ephemeral logs. Seam: `ResourceEvent` variants emitted through the engine observability path.
- **[L1-§11.4]** For long-lived exclusive/external resources (locks, leased cloud instances), deployments need an external TTL / dead-man strategy; Nebula v1 does not provide an external lease arbiter.
- **Bulkhead isolation** — `ErrorKind::Backpressure` signals pool exhaustion; callers decide retry policy. Pool depletion does not cascade across topology boundaries.

## Non-goals

- Not a connection driver — resource implementations supply the actual client (sqlx pool, reqwest client, etc.); this crate owns the lifecycle wrapper.
- Not a retry pipeline — retry composes one layer up (action handler / engine activity / caller-supplied `nebula-resilience` pipeline). The manager-side `AcquireResilience` wrapper was removed; peer Rust pools (sqlx, deadpool, bb8) ship acquire-timeout only, retry above. Retry around outbound calls inside `create`/`check` uses `nebula-resilience` directly at the resource impl.
- Not a secret holder — credentials are populated into slot fields by the framework; secret material is managed by `nebula-credential`.
- Not an expression evaluator — resource `Config` comes from `nebula-schema`-validated parameters; expression resolution is `nebula-expression`'s job. The engine-facing `Manager::register_resolved` orchestrates the resolve→validate→register pipeline but the evaluator itself stays out.

## Maturity

See `docs/MATURITY.md` row for `nebula-resource`.

- API stability: `frontier` — slot-binding pattern shipped; 2 topologies (`Pooled` / `Resident`), `Manager`, `ReleaseQueue`, and `ResourceGuard` are the authoritative lifecycle surface; topology runtime variants are actively evolving.
- `#![forbid(unsafe_code)]` enforced, `#![warn(missing_docs)]` active.
- Integration tests: shared-resource cross-workflow path is verified in `crates/engine/tests/resource_integration.rs::shared_resource::cross_workflow_resource_sharing`.
- Per-slot rotation fan-out: lands in a follow-up.

## Related

- Canon: workspace canon doc — resource lifecycle contract (acquire/health/release; orphan drain), lifecycle visibility in journal/trace.
- Integration model: workspace integration-model doc, `nebula-resource` section.
- Siblings: `nebula-core` (`ResourceKey`, `ExecutionId`, `Dependencies`), `nebula-credential` (`CredentialGuard` populated by framework), `nebula-action` (`ResourceAction` trait, `ResourceProduces<R>` marker), `nebula-resilience` (acquire-path and outbound-call retry).

## Appendix

### Drain mechanism types (evicted from PRODUCT_CANON.md §11.4)

Orphaned resources are drained by the next process through:

- `DrainTimeoutPolicy` — policy controlling how long a drain operation waits.
- `ReleaseQueue` (`src/release_queue.rs`) — the queue of releases awaiting drain.

These types are L4 implementation detail — rename/refactor without canon revision. The L2 invariant ("async release is best-effort on crash; orphaned resources rely on next process") lives in canon §11.4.

### Topology reference

| Topology   | Use case                        | Instance model                                    |
|------------|---------------------------------|---------------------------------------------------|
| `Pooled`   | Databases (Postgres, Redis)     | N interchangeable instances with checkout/recycle |
| `Resident` | HTTP clients (`reqwest::Client`) | One shared instance, clone on acquire             |
| `Bounded`  | License seats, serial device    | Concurrency cap, no warm pool (capped/exclusive/unbounded) |

Long-running workers (`Daemon`) and pull-based event subscriptions (`EventSource`) live in `nebula_engine::daemon`; this crate retains pool/SDK-client topologies only (canon §3.5).

#### Custom topologies (the open `Topology<R>` trait)

`Pooled` / `Resident` are not special — they are two `impl Topology<R>`s. An
author can supply a bespoke topology (a permit pool, an FFmpeg transcoder pool,
a sticky-session pool) by implementing the **slot-centric** `Topology<R>` trait
and pinning `type Topology = MyPool` on the resource. The contract is
**framework-driven and safe-by-construction**: the framework owns the acquire
loop — the fenced `InstanceStore::checkout`, the stale-slot destroy, the
cancel-safe guard wrap, and the on-release return-or-destroy. The topology
supplies only thin R-aware hooks (`create_slot`, `slot_instance`,
`into_instance`, `accept`, `prepare`, `on_release`, `pools`, `store_capacity`,
`dispatch_credential_hook`, …). A custom topology therefore writes **zero**
store / checkout / destroy / revoke-fence code — the credential-revoke fence is
framework-owned for every topology, built-in and custom alike. A non-pooling
topology that carries credential slots (a shared/multiplexed singleton) must
override `dispatch_credential_hook` for revoke teardown; the registrar emits a
loud warning when it does not.

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
    AcquireOptions, Manager, RegistrationSpec, ResidentConfig, ScopeLevel,
    dedup::SlotIdentity,
    runtime::{TopologyRuntime, resident::ResidentRuntime},
};

// One bot, registered once at organization scope through the single funnel.
let manager = Arc::new(Manager::new());
let bot = TelegramBot::new(/* construct from credentials */);
let resident_rt = ResidentRuntime::<TelegramBot>::new(ResidentConfig::default());
manager.register(RegistrationSpec {
    resource: bot,
    config: bot_config,
    scope: ScopeLevel::Organization(org_id),
    slot_identity: SlotIdentity::Unbound,
    topology: TopologyRuntime::Resident(resident_rt),
    acquire: Manager::erased_acquire_resident_for::<TelegramBot>(),
    resilience: None,
    recovery_gate: None,
})?;

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
guarantee applies to `Pooled` (one pool with N interchangeable instances).

Verification: see `crates/engine/tests/resource_integration.rs::shared_resource::cross_workflow_resource_sharing`
— 10 simulated workflows × 1 `TelegramBot` resource × Organization scope ⇒
exactly one `create` invocation, all 10 leases share the same `Arc`.

#### Invalidation triggers

- **Fingerprint change in `ResourceConfig`**. Calling `Manager::reload_config::<R>(new_config, &scope)` validates the new config, swaps it in, bumps the resource's `generation`, and emits `ResourceEvent::ConfigReloaded`. For `Pooled` topologies the pool's fingerprint atomic is updated so idle entries with the stale fingerprint are evicted on next acquire or release. `Resident` topologies keep the existing runtime alive until liveness fails (the rebuild then picks up the new config). No-op reloads (same fingerprint) short-circuit to `ReloadOutcome::NoChange` without bumping the generation.
- **Different `R::key()`**. Two distinct `Resource` impls — even configured identically — register under separate registry rows. `acquire_resident::<TelegramBot>` and `acquire_resident::<AlternateBot>` produce independent runtimes and can be replaced or shut down independently.
- **Different `ScopeLevel`**. The same `Resource` impl registered at `Organization(A)` and `Organization(B)` produces two independent instances; the registry's scope-aware `find_by_scope` does an exact match first and falls back to `Global` only when no exact match exists. Per-scope reloads / shutdowns affect only the matching scope.
- **Manager shutdown**. `Manager::shutdown()` cancels the shared token; in-flight acquires drain via `graceful_shutdown` per canon §11.4. After shutdown, every acquire returns `ErrorKind::Cancelled` — no leases are minted from a torn-down registry.
