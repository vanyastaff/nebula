---
name: nebula-resource
role: Bulkhead Pool (Release It! ch "Stability Patterns — Bulkhead"; resource lifecycle acquire / health / release)
status: frontier
last-reviewed: 2026-07-09
canon-invariants: [L2-11.4, L2-13.3]
related: [nebula-core, nebula-schema, nebula-error, nebula-resilience, nebula-credential, nebula-action]
---

# nebula-resource

## Purpose

External connections — database pools, HTTP clients, message brokers — are a primary failure surface in workflow engines. When an action creates its own client on demand and never releases it, pool exhaustion and orphaned handles accumulate silently. `nebula-resource` solves this by making the engine the owner of the resource lifecycle: acquire, health-check, hot-reload, and scope-bounded release are engine concerns, not per-action boilerplate. Actions receive a `ResourceGuard` that derefs to `R::Instance` and releases on drop; the engine ensures the backing instance is healthy before granting the guard.

## Role

**Bulkhead Pool** (Release It! ch "Stability Patterns — Bulkhead"). Isolates resource exhaustion per topology so one depleted pool cannot cascade to unrelated paths. Three built-in topologies provide common policies: `Pooled` (N interchangeable stateful instances), `Resident` (one retained master shared through owning Arc entries), and `Bounded` (a runtime concurrency cap with no warm idle pool — capped / exclusive / unbounded). The `Provider` trait declares three associated types (`Config`, `Instance`, `Topology`) and lifecycle methods; per-topology hook traits (`PoolProvider` / `ResidentProvider` / `BoundedProvider`) add recycle / liveness / reset decisions. The framework owns the acquire loop and the credential-revoke fence; a custom `Topology<R>` impl can register through the same `Manager`. Long-running workers (`Daemon`) and pull-based subscriptions (`EventSource`) live in `nebula_engine::daemon` — canon §3.5 reserves "Resource" for pool/SDK clients.

## Public API (v4 — slot-binding pattern, 2026-04-29)

The v4 surface — singular `type Credential` is dropped in favor of typed credential **slot fields** declared via `#[credential(key = "…")]` field attributes on the resource struct. Each slot field is a lock-free `SlotCell<CredentialGuard<C>>` the framework populates and rotates through `&self`; the derive emits a `<field>_slot()` read accessor. Multi-credential resources are now natural; per-slot rotation lands via `Provider::on_credential_refresh(&self, slot_name, instance)` with a companion `Provider::on_credential_revoke(&self, slot_name, instance)`.

### `Provider` trait — 3 associated types, slot fields on Self

Only `Config` / `Instance` / `Topology`, `key()`, and `create()` are required;
every other method has a default.

```rust
pub trait Provider: HasCredentialSlots + Send + Sync + Sized + 'static {
    type Config:   ResourceConfig;
    type Instance: Send + Sync + 'static;
    // Pooled<Self> | Resident<Self> | Bounded<Self> | custom Topology<R>
    type Topology: Topology<Self>;

    fn key() -> ResourceKey;

    /// Slot cells are populated on `&self` BEFORE create runs; read the
    /// resolved guard via the derive-emitted `<field>_slot()` accessor.
    /// All lifecycle methods return the crate's typed `Error` — author
    /// error enums convert in via `#[derive(ClassifyError)]` + `?`.
    async fn create(&self, config: &Self::Config, ctx: &ResourceContext)
        -> Result<Self::Instance, Error>;

    /// Per-slot rotation: the engine swaps the rotated guard into the slot
    /// cell, then calls this with the slot name + live `Instance`. `&self` —
    /// re-auth acts on `instance`'s interior mutability. Default no-op.
    async fn on_credential_refresh(&self, slot_name: &str, instance: &Self::Instance)
        -> Result<(), Error> { Ok(()) }

    /// Per-slot revocation: post-invocation the resource emits no further
    /// authenticated traffic on the revoked credential. Default no-op.
    async fn on_credential_revoke(&self, slot_name: &str, instance: &Self::Instance)
        -> Result<(), Error> { Ok(()) }

    /// Health probe. Default `Ok(())`; `check_cost` sets the maintenance
    /// reaper's probe cadence (`Cheap` / `Moderate` / `Expensive`).
    async fn check(&self, instance: &Self::Instance) -> Result<(), Error> { Ok(()) }
    fn check_cost(&self) -> CheckCost { CheckCost::Cheap }

    /// Single consuming terminal hook: flush, stop, close, and join here.
    /// The default only drops `instance`; async cleanup requires an override.
    async fn destroy(&self, instance: Self::Instance, cx: TeardownCx)
        -> Result<(), Error> { Ok(()) }

    /// Wall-clock budget `TeardownCx` turns into `cx.deadline`.
    fn teardown_budget(&self) -> Duration { Duration::from_secs(30) }

    /// `None` disables the hold-deadline leak watchdog (diagnostic only —
    /// it never force-releases a lease).
    fn max_hold_duration() -> Option<Duration> { None }

    /// Schema-free author intent. The factory derives and binds `Config`'s
    /// canonical schema exactly once.
    fn metadata() -> ResourceMetadataDraft;
}
```

`HasSchema::schema()` and `schema_of::<T>()` admit definitions through
`Result<ValidSchema, ValidationReport>`. `ResourceFactory::metadata()` performs
the only resource metadata admission, caches the resulting immutable
`ResourceMetadata`, and returns it by reference. Schema failures and a draft key
that differs from `Provider::key()` propagate as `MetadataBuildError`; registry
insertion fails before mutation. Catalog enumeration must not drop a failing entry.
`ResourceFactory` is sealed: plugins can store and invoke the erased contract but
cannot implement it. They must use a typed `KindActivator`; derive-emitted
`<Name>Factory` wrappers yield that crate-issued capability through
`into_contribution()`. The metadata, `TypeId`, validation, and registration projections
therefore share one concrete `Provider` type instead of being caller attestations.
Metadata names are checked
`MetadataName` values (`metadata_name!("HTTP client")` for static definitions),
and invariant-bearing base fields are read through accessors.
`Provider::metadata` is required. Replace removed `ResourceMetadataDraft::from_key`
calls with `new(key, metadata_name!("Display name"), description)` or checked
`try_new(key, name, description)`; names are never inferred from keys. Typed
categories and links use `with_categories` and `add_link`; the documentation URL
convenience authors the Overview link. Tags are trimmed, sorted, and deduplicated.

`ResourceFactory::validate` treats JSON strictly as data, consumes validation
against the admitted schema and `resolve_data()`, then decodes the resulting
`ResolvedValues`. `ResourceFactory::register` accepts a `ResourceConfigInput`
that makes ingress explicit: `data` normalizes persisted or transport JSON and
completes it without executing template-looking strings or `$expr`-shaped
objects, while `authored` accepts a typed `AuthoredValue` whose deliberate
expression nodes are admitted against that same cached schema and evaluated.
Normalization is retained exactly once; expression results never become new
programs. Both paths reject undeclared fields and protected secret leaves,
including nested results. Secrets must enter through credential slots, not
resource config.

Admitted `ResourceMetadata` has private fields, getters, and `Serialize` only.
Persisted catalog bytes deserialize as `RecordedResourceMetadata`; callers must
explicitly call `readmit_against` with a freshly admitted factory definition.
The catalog record nests shared fields under `base` with required
`metadata_wire_version: 2`. Flat/unversioned legacy records are rejected. Use
bounded `from_slice`/`from_reader` for raw input; direct generic serde validates
structure but does not bound parser allocation. Same-version category, link, or
typed-notice changes require fresh evidence. See
[catalog migration and limits](../../docs/INTEGRATION_MODEL.md#catalog-construction-and-wire-migration).

Root schemas follow the configuration's serde wire shape: `()` and derived unit
structs use scalar `null`, empty-braced records use `{}`, and primitives declare
their scalar kinds. Supplied objects are never converted to `null`. A custom
newtype configuration must expose the schema matching its serialized root.
`#[derive(ResourceConfig)]` supplies `HasSchema` only for unit and empty-braced
configs. Nonempty and tuple configs must declare `#[config(schema = external)]`
and provide a real schema, usually with `#[derive(Schema)]` for named fields.
The external option also permits an intentional custom override for empty configs.

The per-resource **credential epoch** (an order-sensitive fold over every
`#[credential]` slot's generation, used by the rotation reconcile) lives on a
separate `HasCredentialSlots` trait, emitted by `#[derive(Resource)]` —
never hand-maintained.

**`type Credential` was dropped.** There is no longer a singular credential associated type; resources declare credentials as slot fields. The opt-out alias `NoCredential` is no longer required — resources without credentials simply have no `#[credential]` fields.

### Slot-binding pattern — `#[derive(Resource)]` + hand-written `impl Provider`

The **two-derive pattern**: `#[derive(Resource)]` emits only the slot
plumbing; you supply a hand-written `impl Provider` with real `create` /
`check` / `destroy` bodies. No container `#[resource(...)]`
attribute is needed or accepted.

```rust
use nebula_credential::CredentialGuard;
use nebula_resource::{Provider, Resource, SlotCell};

#[derive(Resource)]
struct Postgres {
    #[credential(key = "db_auth", purpose = "Main DB auth")]
    db_auth: SlotCell<CredentialGuard<DatabaseCredential>>,

    #[credential(key = "audit", purpose = "Audit log auth")]
    audit: SlotCell<CredentialGuard<AuditCredential>>,
}

impl Provider for Postgres {
    type Config    = PostgresConfig;
    type Instance  = PgPool;
    type Topology  = Pooled<Self>;

    fn key() -> ResourceKey { resource_key!("postgres") }
    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), metadata_name!("Postgres"), "Database pool")
    }

    async fn create(&self, config: &PostgresConfig, _ctx: &ResourceContext)
        -> Result<PgPool, Error>
    {
        // read resolved credentials through derive-emitted accessors
        let guard = self.db_auth_slot().expect("db_auth slot must be bound");
        // … build pool …
    }
    // check / destroy …
}
```

`#[derive(Resource)]` emits:
- `impl DeclaresDependencies for Postgres` — enumerates credential slot fields so the engine resolves each before `create` runs.
- A read accessor per slot field: `pub fn <field>_slot(&self) -> Option<Arc<CredentialGuard<C>>>` returning the resolved guard, or `None` until the framework binds it. Implementations read the credential through this accessor — never off the raw cell field. A pure derive cannot add or rewrite struct fields and `ManagedResource` hands out `Arc<R>` (no `&mut R`), so the author declares the `SlotCell` cell and the framework populates / rotates it through `&self` via `SlotCell::store`.
- `impl HasCredentialSlots for Postgres` — order-sensitive epoch fold used by the engine's hot-reload path.

The per-topology hook trait (`PoolProvider` / `ResidentProvider` / `BoundedProvider`) is always hand-written — the derive does not emit it. Every hook has a default, so an empty `impl PoolProvider for Postgres {}` is enough to opt into pool topology.

#### Field-type shape

The generated `<field>_slot()` accessor emits one fixed body, so a `#[credential]` slot field must currently be **exactly** `SlotCell<CredentialGuard<C>>` (required + eager). `Option<…>`- and `Lazy<…>`-wrapped slots are a hard compile error at the derive site until the accessor is generalized — declare an unconditional cell and treat the accessor's `None` (unbound) return as the optional/lazy case.

### `Manager` registration

Registration goes through **one funnel**: `Manager::register::<R>(spec: RegistrationSpec<R>)`. The per-topology `register_<topo>[_with]` shorthands and the 3-deep delegation chain were removed — `RegistrationSpec<R>` is a plain struct with public fields and no builder:

```rust
manager.register(RegistrationSpec {
    resource,                                  // fully-constructed R, all #[credential] slots resolved
    config,                                    // validated on register
    scope: ScopeLevel::Global,
    slot_identity: SlotIdentity::Unbound,      // structural dedup/resolution key (see below)
    topology: Resident::new(ResidentConfig::default()),
    recovery_gate: None,                       // Option<Arc<RecoveryGate>>
})?;
```

`slot_identity` is a collision-free structural resolved-credential identity (`SlotIdentity::{Unbound, Structural(Arc<[(String, String)]>)}`). Registrations with different exact `(slot, credential)` bindings occupy distinct rows, but this identity is **not tenant authorization**: equal credentials or unbound registrations do not prove equal tenants. Hosts must admit the correct scope and caller authority. JSON/expression registration enters through `ResourceFactory::register`; its manager helper is crate-private and consumes the factory-admitted schema.

The framework resolves declared `#[credential]` slots **before** invoking `Provider::create` — implementations read each resolved credential through the derive-emitted `self.<field>_slot()` accessor (`Option<Arc<CredentialGuard<C>>>`), handling the `None` (unbound) case explicitly.

### Lifecycle ownership and breaking migration

`Provider::shutdown(&Instance)` has been removed: it was never dispatched by
the framework. This is a source-breaking change for implementations that defined
that method. Move all flush, drain, stop, close, and worker-join work into
`destroy(Instance, TeardownCx)` and remove the old method. There is one consuming
terminal hook, invoked on final ownership release; errors consume the instance
and never trigger terminal retries. The internal crate stays on the workspace's
lockstep version rather than introducing a separate release line.

The default `destroy` only runs synchronous Drop, appropriate for RAII-only
instances. An override must tolerate the manager's cancellation token already
being cancelled. Its future can be dropped at any await, so owned tasks and
handles need a synchronous drop fallback such as abort-on-drop task ownership.
Neither async cleanup nor Drop promises release after a process crash.

Use the supplied `cx.reason` and bound graceful work with
`tokio::time::timeout_at(cx.deadline.into(), …)`. `TeardownCx` has public mutable
fields; the framework separately captures its deadline, so local mutation cannot
extend the timeout. This bound is cooperative and cannot preempt a blocking
future poll or destructor.

`Provider::Instance` need not be `Clone`, including for Resident. Guards own the
actual topology entry and expose shared references; `Sync` is still required.
Exclusive mutable access and `!Sync` instances are not provided by this API.
Aliases intentionally exposed by a provider's instance remain the author's responsibility.

- Replace fabricated `ResourceGuard::owned` / `guarded` / `guarded_with_permit`
  and `detach` with manager acquisition and explicit `release().await`.
- Custom `Topology::create_entry` receives a borrowed `RetainedStore<Entry>`
  after the resource context and returns only the fresh `CreatedEntry<Entry>`.
  A topology that keeps a shared root publishes it into that store and keeps
  only its opaque `RetainedId`; hiding strong entries outside the store violates
  the mandatory lifecycle contract and escapes framework accounting.
- Retained state is manipulated only through the borrowed store. The store is
  not cloneable and exposes no public remove-and-take capability; closing and
  destruction remain framework-owned on the supported path. `InstanceStore`,
  however, exposes ownership-transferring `drain_all` to trusted in-process
  topology code. The type system cannot prevent a custom plugin from draining,
  dropping, or aliasing an entry outside framework submission, and framework
  abandonment metrics cannot observe that loss. `Topology::quiesce` may stop
  policy-owned background work, but must not invalidate or await issued guards.
  Physical shutdown belongs to final-owner `Provider::destroy`.
- Retained lease fences are per generation, not global to the resource row.
  Incremental cleanup can therefore extract every ready sibling while another
  generation still has a live lease or fail-closed poisoned accounting;
  terminal close also tears down healthy siblings before reporting poison. A
  newly ready retirement wakes cleanup without waiting for an unrelated lease
  transition.
- `CreatedEntry` exposes `new`, `entry`, and `into_entry`; displaced retained
  owners are retired inside `RetainedStore`, not returned beside a new lease.
  `into_owned_instance` returns `Option<Instance>`: shared ownership yields an
  instance only on final release.
- Drain counters settle after queued cleanup completes or is abandoned.
  `Released` is emitted only after cleanup returns; rejected or abandoned
  cleanup never emits a success event. Await release or graceful shutdown for
  a completion checkpoint.
- Nested release on the same queue uses bounded cooperative execution. A release
  targeting another queue, or exceeding that nested capacity, returns
  `ReleaseOutcome::Deferred` after acceptance instead of creating a wait cycle.
  Cleanup remains queue-owned; never retry that release. Separately spawned
  provider tasks do not inherit the current cleanup context.
- Registration and removal after shutdown admission closes are rejected. Replacement/removal fences the old
  row and schedules owned retirement; normal graceful shutdown awaits idle/master teardown.
- Force with outstanding leases returns an incomplete snapshot and retains cleanup
  workers while the manager lives. Manager Drop closes the queue; later releases
  may be rejected and counted. Dropping an open manager cannot await provider
  teardown: every remaining row emits a `ResourceTeardownFailed` event with
  `RetirementOrigin::ManagerDrop`. Call `graceful_shutdown` for a teardown
  checkpoint. This process-local queue cannot recover work after a crash.
- `ShutdownError::ResourceTeardownFailed` preserves the typed provider error through
  its source chain. Consequently, `ShutdownError` no longer implements `UnwindSafe`
  or `RefUnwindSafe`; callers crossing unwind boundaries must account for that source.

### Other public API

- `ResourceGuard` — manager-owned topology entry; borrows `R::Instance` through `Deref`, queues release on Drop, or awaits that same queued job through `release()`. No fabricated guards or detachable entries.
- `ResourceRef<R>` — lazy reference type holding a `ResourceId` string + `PhantomData<R>`. Resolves to a `ResourceGuard<R>` via `.resolve(ctx).await`.
- `RegistrationSpec` — the single registration param aggregate (see above).
- `AcquireOptions` — per-call acquire knobs (`deadline`, `acquire_slow_threshold`); every `acquire_*` takes one.
- `SlotIdentity`, `DedupKey` — structural resolved-credential identity (`Unbound` / `Structural`) and the `(key, scope, slot_identity)` registry dedup key; neither is tenant authority.
- `ManagerConfig`, `RegisterOptions` — configuration surface.
- `Registry`, `LookupOutcome`, `ManagedResourceView` — type-erased storage with read-only diagnostic lookup; lifecycle authority remains inside `Manager`.
- `ResourceMetadataDraft` — schema-free resource author intent returned by `Provider::metadata`.
- `ResourceMetadata` — immutable factory-admitted descriptor with getter-only key, name, description, schema, version, and tags; serializable but not deserializable.
- `RecordedResourceMetadata` — deserialized catalog evidence requiring explicit readmission against a fresh factory definition.
- `ResourceConfig` — operational config trait (no secrets); supertype `HasSchema`.
- `SlotCell` — lock-free `ArcSwap`-based credential slot cell the framework populates/rotates.
- `TeardownCx`, `TeardownReason` — deadline + cause handed to `Provider::destroy` (ADR-0093 teardown contract).
- `ReleaseQueue` — background worker pool for async cleanup. Drain on crash is best-effort; see §11.4 canon note.
- `DrainTimeoutPolicy`, `ShutdownConfig`, `ShutdownReport`, `ShutdownError` — `Manager::graceful_shutdown` drain policy, outcome, and typed failure.
- `ReloadOutcome` — result of `Manager::reload_config` (`NoChange` / `SwappedImmediately`).
- `Error`, `ErrorKind` — typed error with retry classification.
- `ResourceContext` — execution context with cancellation and capability traits (`HasResources`, `HasCredentials`).
- `ScopeLevel` — re-exported from `nebula_core::ScopeLevel`.
- `ResourcePhase`, `ResourceStatus` — lifecycle phase tracking for observability.
- `ResourceEvent` — lifecycle events (`Registered`, registry-unpublish `Removed`, `AcquireSuccess`, `AcquireFailed`, `Released`, `HealthChanged`, `ConfigReloaded`, `RetryAttempt`, `BackpressureDetected`, `RecoveryGateChanged`, `SlotRefreshed`, `SlotRevoked`, `SlotRefreshFailed`, `SlotRevokeFailed`, `RetiredCleanupFailed`, `ResourceTeardownFailed`, `MaintenanceEvicted`, `HoldDeadlineExceeded`). `ResourceTeardownFailed` reports every failing row-retirement stage with a fixed secret-free message; shutdown still returns only the first typed failure as its aggregate result.
- `ResourceOpsMetrics`, `ResourceOpsSnapshot` — registry-backed operation counters.
- `RecoveryGate`, `RecoveryGateConfig`, `RecoveryTicket`, `RecoveryWaiter`, `GateState` — thundering-herd recovery gate.
- Open `Topology<R>` trait + framework topology structs `Pooled<R>` / `Resident<R>` / `Bounded<R>` (reached monomorphically through `Provider::Topology`; no dispatch enum — the framework owns the acquire loop).
- Per-topology hook traits: `PoolProvider`, `ResidentProvider`, `BoundedProvider`.
- Topology configs / constructors: `PoolConfig`, `ResidentConfig`, `BoundedMode` (`Bounded::capped`/`exclusive`/`unbounded`).
- `PoolStats` — point-in-time pool snapshot (`idle`, `capacity`, `available_permits`, `in_use`) via `Manager::pool_stats`.
- `TopologyTag` — the runtime topology discriminant (`Pool` / `Resident` / `Bounded` / custom) carried on a `ResourceGuard`; read via `guard.topology_tag()`.
- Custom-topology surface: framework-owned `InstanceStore` for idle entries and
  non-cloneable `RetainedStore` + opaque `RetainedId` for topology-retained
  roots, plus `Checkout`, `CheckedOut`, `ReturnOutcome`, `Ticket`, `Unavailable`,
  `Load`, `MaintenanceSchedule`, `AdmissionPhase`, `AdmissionStatus`,
  `PoolStrategy`, and `NoTopology`.
- `HasCredentialSlots` — per-resource credential epoch fold; emitted by `#[derive(Resource)]`, or by `no_credential_slots!(R)` for a slot-less resource.
- `HasResourcesExt` — the `ctx.resource::<R>().await?` access surface for action code.
- Sealed `ResourceFactory`, typed `KindActivator`, `ResourceActivatorRegistry`, `ResourceConfigInput`, `RegisterRequest`, `RegistrarError`, `ResourceRegistrationOutcome`, `SlotBinding`, `BoxFut` — the crate-issued erased plugin-registration bridge.
- `CheckCost` — relative `check` probe cost driving the maintenance reaper's health-probe cadence.
- Re-exports so consumers need no direct sibling dep: `Subscriber` (`nebula-eventbus`), `Credential` / `CredentialContext` / `CredentialId` (`nebula-credential`), `HasSchema` / `Schema` / `ValidSchema` / `impl_empty_has_schema!` (`nebula-schema`).
- Feature `rotation`: `ResourceFanoutDriver`, `ResourceFanoutIndex`, `Bind`, `RotationOutcome`.
- `#[derive(Resource)]`, `#[derive(ResourceConfig)]`, `#[derive(ClassifyError)]` — proc-macro derivations.
- `resource_key!` — re-exported from `nebula_core` for declaring resource keys at compile time.
- `prelude` — `use nebula_resource::prelude::*` for internal/direct crate consumers. Integration
  authors use the curated `nebula_sdk::prelude`, which includes the topology hook traits and omits
  engine-owned managers, registries, release queues, and rotation fan-out. If a required authoring
  contract is missing from the SDK, treat it as an SDK gap; there is no supported
  `nebula_sdk::nebula_resource` escape hatch. The SDK re-exports resource derives;
  generated paths resolve through its hidden macro namespace when no leaf dependency
  is declared. External compile contracts cover representative derives and manual
  `Provider` terminal hooks using the curated `TeardownCx` / `TeardownReason` exports
  plus the general-purpose `async-trait` crate.

## Migration recipe (pre-v4 → v4)

The slot-binding break is hard. To migrate an existing `Resource` impl:

1. **Drop `type Credential`.** Move the credential dependency to a `#[credential(key = "…")]` slot field of type `SlotCell<CredentialGuard<C>>` on the struct, constructed with `SlotCell::empty()`. Change `Provider::Credential` references to read through the derive-emitted `self.<field>_slot()` accessor.
2. **Drop the `scheme: &<R::Credential as Credential>::Scheme` parameter** from `create`. The framework populates the slot cells before `create` runs; read the resolved guard via `self.<field>_slot()` (`Option<Arc<CredentialGuard<C>>>`) and handle the `None` (unbound) case explicitly.
3. **Replace `on_credential_refresh(scheme, ctx)` with `on_credential_refresh(&self, slot_name, instance)`** and add an `on_credential_revoke(&self, slot_name, instance)` override where the resource held revoke logic. The engine swaps the rotated guard into the slot cell before the call; `&self` is an immutable descriptor, so blue-green / re-auth acts on `instance`'s interior mutability. Multi-credential resources can branch on `slot_name` to refresh only the affected sub-system.
4. **Drop `nebula_credential::NoCredential`.** Resources without credentials simply have no `#[credential]` fields. The `NoCredential` opt-out is no longer needed.
5. **Use the two-derive pattern**: annotate the struct with `#[derive(Resource)]` (emits slot plumbing only); write a hand-written `impl Provider` with real `create` / `check` / `destroy` bodies. No `#[resource(...)]` container attribute. The per-topology hook trait (`PoolProvider` / `ResidentProvider` / `BoundedProvider`) is still hand-written.
6. **Update test code** — registration now goes through one funnel: `Manager::register::<R>(RegistrationSpec { resource, config, scope, slot_identity, topology, recovery_gate })`. The per-topology `register_<topo>[_with]` shorthands and the previous `acquire_*_default` shorthand were removed; acquire is the single `acquire_<topo>` / `acquire_<topo>_for_identity` family (or the type-erased `acquire_any`).
7. **For credential slot identity**, pass `SlotIdentity::Unbound` for the historical single-row dedup, or build a `SlotIdentity::Structural` from the resolved `(slot, credential)` pairs for per-binding row separation. The old `u64` `slot_identity` digest was removed.

The trait-shape changes ship complete; the per-slot rotation fan-out
(`credential_fanout`, gated behind the `rotation` feature) has also landed
in this crate — see [`credential-rotation.md`](docs/credential-rotation.md)
for the full sequence.

## Runnable examples

- `cargo run -p nebula-examples --example resource_pooled_http_prelude` — the smallest end-to-end `Pooled` registration, driven from `nebula_resource::prelude::*` (plus the `PoolProvider` hook trait, which the prelude does not re-export)
- `cargo run -p nebula-examples --example resource_postgres_pool` — `Pooled` topology + `ResourceAction` for per-execution test schema (configure / cleanup ordering)
- `cargo run -p nebula-examples --example resource_resident_http` — `Resident` topology + OAuth-style credential refresh hook
- `cargo run -p nebula-examples --example resource_telegram_multi_workflow` — `Resident` topology + cross-workflow shared-resource dedupe (1 bot, 10 workflows, 1 `Provider::create`)

The headline patterns and topology selection guidance are distilled into
`crates/resource/docs/topology-reference.md`; the credential rotate → slot
swap → refresh/revoke sequence behind the second example is diagrammed in
`crates/resource/docs/credential-rotation.md`.

## Contract

`Manager::graceful_shutdown` rejects acquires and snapshots/fences registered rows
before its first await. One manager-owned publisher submits that snapshot to the
bounded retirement supervisor while guard drain runs. Retirement joins tracked
maintenance and relinquishes idle/master ownership. A retained Resident parent or
idle Pooled parent can therefore release same-manager child guards during normal
graceful shutdown. Shared instances stay usable until their last owning lease
releases; relinquishing a root is not physical destruction. Replaced/removed rows
are also included in cleanup. Once guards drain, shutdown clears the registry index,
settles retirement, closes the release queue and joins its workers.
`Force` with outstanding leases returns an incomplete report and leaves cleanup
workers open while the manager lives; late leases can still release.
An `Abort` drain timeout or cancellation during drain preserves the diagnostic
registry, the single snapshot and its publisher while the manager lives. Retried
calls keep the first caller's policy and start time. Publication is bounded by the
original drain-plus-cleanup envelope; terminal finalization gets at most the cleanup
budget and cannot extend that envelope. A late zero guard count can advance after
the drain deadline. Publication failure returns a terminal error; with outstanding
guards it retains release workers for late cleanup. Immediate `shutdown()` and cancellation of the
manager token also leave cleanup available; dropping the manager closes it.

This is process-local ownership within one manager. Engine wiring of activation-bound
nested resource accessors, tenant authorization and durable lifecycle fan-out remain
separate contracts; the manager does not infer or schedule a runtime dependency graph.

The release worker budget is cooperative: expiry aborts all workers and awaits
termination, including disposal of buffered tasks. Tokio cannot preempt a blocking
future poll or destructor. Cancellation during worker waiting requests abort but
cannot await acknowledgement only when the owning bounded worker-wait future is
dropped. Cancelling a `Manager::graceful_shutdown` caller leaves that future in
the manager-owned terminal task, which still joins or aborts and joins workers.
Standalone `ReleaseQueue::shutdown` retains its best-effort behavior: cancelling
its wait leaves workers running.

`ShutdownReport::release_queue_drained` reports worker completion, and
`dropped_release_tasks` is a cumulative queue-lifetime snapshot of futures that
did not complete, including unstarted batch members. Guards dropped after `Force` may add
losses later. Neither a drained queue nor zero observed losses proves that every
provider teardown succeeded. Framework jobs return typed results; completed
provider errors are observed separately from abandoned jobs. `ResourceGuard::release().await` is
the explicit per-guard error checkpoint.

Queue messages, nested execution slots, and saturation rescue admission are bounded.
Rescue and nested dispatchers belong to the shutdown handle and are aborted/joined
with ordinary workers. A destroy batch occupies one queue message, owns its entries,
and, when built with `panic = "unwind"`, applies a separate panic/timeout boundary to
each member while continuing siblings. With `panic = "abort"`, a trusted in-process
plugin panic terminates the process before framework recovery or settlement can run.
The queue-message bound does not bound the size of a provider instance or an owned
batch, and the queue is not a durable cleanup log.

- **[L2-§11.4]** Resource lifecycle (acquire → use → release) is engine-owned. Async release is best-effort on crash; process-local queued jobs cannot be recovered by the next process. Authors must not assume "release ran" without an explicit checkpoint. External orphan recovery requires a separate durable/TTL strategy.
- **[L2-§13.3]** Acquire → use → release for Resource-backed steps must be attributable in durable journal or an operator-visible trace. Not only ephemeral logs. Seam: `ResourceEvent` variants emitted through the engine observability path.
- **[L1-§11.4]** For long-lived exclusive/external resources (locks, leased cloud instances), deployments need an external TTL / dead-man strategy; Nebula v1 does not provide an external lease arbiter.
- **Bulkhead isolation** — `ErrorKind::Backpressure` signals pool exhaustion; callers decide retry policy. Pool depletion does not cascade across topology boundaries.

## Non-goals

- Not a connection driver — resource implementations supply the actual client (sqlx pool, reqwest client, etc.); this crate owns the lifecycle wrapper.
- Not a retry pipeline — retry composes one layer up (action handler / engine activity / caller-supplied `nebula-resilience` pipeline). The manager-side `AcquireResilience` wrapper was removed; peer Rust pools (sqlx, deadpool, bb8) ship acquire-timeout only, retry above. Retry around outbound calls inside `create`/`check` uses `nebula-resilience` directly at the resource impl.
- Not a secret holder — credentials are populated into slot fields by the framework; secret material is managed by `nebula-credential`.
- Not an expression evaluator — resource `Config` comes from `nebula-schema`-validated parameters; expression resolution is `nebula-expression`'s job. `ResourceFactory::register` orchestrates the resolve→validate→register pipeline through its crate-private manager helper, but the evaluator itself stays out.

## Positioning

`nebula-resource` is `publish = false` — an internal workspace crate, not
published to crates.io. This section frames it honestly for the workspace
audience deciding whether to reach for it over a general-purpose pool crate
inside this repo; publishing it standalone is a separate, unmade owner
decision.

### Why this instead of deadpool / bb8 / sqlx's pool?

The workspace already depends on `tokio` throughout, so runtime-agnosticism
is not a design goal here — what those general-purpose pools don't give a
multi-tenant, credential-rotating workflow engine is:

| Capability | `nebula-resource` | deadpool | bb8 | sqlx (built-in pool) |
|------------|--------------------|----------|-----|------------------------|
| Credential-rotation fan-out + revoke fence (no TOCTOU window) | Yes — built-in, see [`credential-rotation.md`](docs/credential-rotation.md) | No | No | No |
| Hold-deadline leak watchdog (HikariCP `leakDetectionThreshold` lineage) | Yes — `Provider::max_hold_duration` + `HoldDeadlineExceeded` event | No | No | No |
| Jittered anti-thundering-herd recovery gate | Yes — `RecoveryGate`, equal-jitter backoff | No | No | Partial (connection-level retry only) |
| Topology choice (Pooled / Resident / Bounded) + an open `Topology` trait for a fourth | Yes | Pool only | Pool only | Pool only |
| Typed, retryable `ErrorKind` taxonomy | Yes — see the `error` module's caller-action table | Partial (typed but not retry-classified) | Partial | Partial |
| Documented + tested cancel safety on every acquire path | Yes — see "Guarantees" in the crate-root rustdoc | Not documented as a contract | Not documented as a contract | Not documented as a contract |
| Scope-aware registry: create-once dedupe + hot config reload | Yes — `(key, scope, slot_identity)` dedupe, `reload_config` | No (one pool per `Pool` value you manage) | No | No |
| Runtime-agnostic (no forced `tokio` dependency) | No — `tokio`-only | Yes | Yes | Partial |
| Zero background tasks | No — maintenance reaper + release-queue workers | Yes | Yes | Partial |
| Published, versioned crate with an existing adapter ecosystem | No — `publish = false`, in-tree only | Yes | Yes | Yes |

### When NOT to use this crate

- **A standalone binary with no credential rotation and no multi-tenant
  scoping.** `deadpool`/`bb8` are lighter, published, and runtime-agnostic —
  reach for one of those instead of pulling in the engine's registry and
  event-bus machinery for a single always-on connection pool.
- **You need a published, externally-consumable crate today.** This crate is
  `publish = false` and its API still moves between minor releases
  (`frontier` maturity) — publishing it is an unmade decision, not a
  roadmap item you can currently depend on.
- **A pure DB-driver integration with no cross-cutting lifecycle needs.**
  `sqlx`'s built-in pool already handles connection-level retry and
  driver-specific health checks tightly coupled to its own query path;
  wrapping it in another pooling layer buys nothing unless you specifically
  need credential rotation, hold-deadline detection, or scope-aware dedupe.

## Maturity

See the `nebula-resource` row in the workspace [`docs/MATURITY.md`](../../docs/MATURITY.md).

- API stability: `frontier` — slot-binding pattern shipped; 3 topologies (`Pooled` / `Resident` / `Bounded`), `Manager`, `ReleaseQueue`, and `ResourceGuard` are the authoritative lifecycle surface; topology runtime variants are actively evolving.
- `#![forbid(unsafe_code)]` enforced, `#![deny(missing_docs)]` +
  `#![warn(missing_debug_implementations)]` active.
- Integration tests: shared-resource cross-workflow path is verified in `crates/engine/tests/resource_integration.rs::shared_resource::cross_workflow_resource_sharing`.
- Per-slot rotation fan-out: landed in this crate (`credential_fanout`, feature `rotation`) — see [`credential-rotation.md`](docs/credential-rotation.md).

## Related

- Canon: workspace canon doc — resource lifecycle contract (acquire/health/release; orphan drain), lifecycle visibility in journal/trace.
- Integration model: workspace integration-model doc, `nebula-resource` section.
- Siblings: `nebula-core` (`ResourceKey`, `ExecutionId`, `Dependencies`), `nebula-credential` (`CredentialGuard` populated by framework), `nebula-action` (`ResourceAction` trait, `ResourceProduces<R>` marker), `nebula-resilience` (acquire-path and outbound-call retry).

## Appendix

### Drain mechanism types (evicted from PRODUCT_CANON.md §11.4)

Cooperative process-local drain uses:

- `DrainTimeoutPolicy` — policy controlling how long a drain operation waits.
- `ReleaseQueue` (`src/release_queue/mod.rs`) — the queue of releases awaiting drain.

These types are L4 implementation detail — rename/refactor without canon revision.
The queue is not persisted and cannot drain a crashed process's work after restart.
External orphan recovery requires a durable recovery mechanism or a provider TTL.

### Topology reference

| Topology   | Use case                        | Instance model                                    |
|------------|---------------------------------|---------------------------------------------------|
| `Pooled`   | Databases (Postgres, Redis)     | N interchangeable instances with checkout/recycle |
| `Resident` | HTTP clients (`reqwest::Client`) | One retained instance, shared owning lease entries |
| `Bounded`  | License seats, serial device    | Concurrency cap, no warm pool (capped/exclusive/unbounded) |

Long-running workers (`Daemon`) and pull-based event subscriptions (`EventSource`) live in `nebula_engine::daemon`; this crate retains pool/SDK-client topologies only (canon §3.5).

#### Custom topologies (the open `Topology<R>` trait)

`Pooled` / `Resident` are not special — they are two `impl Topology<R>`s. An
author can supply a bespoke topology (a permit pool, an FFmpeg transcoder pool,
a sticky-session pool) by implementing the **entry-centric** `Topology<R>` trait
and pinning `type Topology = MyPool` on the resource. The contract is
**framework-driven, with a mandatory trusted-plugin lifecycle contract**: the framework owns the acquire
loop — the fenced `InstanceStore::checkout`, the stale-entry destroy, the
cancel-safe guard wrap, and the on-release return-or-destroy. The topology
supplies only thin R-aware hooks (`create_entry`, `entry_instance`,
`into_owned_instance`, `quiesce`, `accept`, `prepare`, `on_release`, `pools`,
`store_capacity`, `dispatch_credential_hook`, …). Hooks receive borrowed
framework stores: `InstanceStore` for idle entries and `RetainedStore` for
long-lived roots. A custom topology may publish or retire retained roots only
through the latter and must keep opaque `RetainedId`s rather than hidden strong owners.
Every retained generation has its own lease fence, so a live lease blocks only
that generation; ready retired siblings remain independently drainable.
Only owners published into framework stores are accounted for. Cloning strong
aliases out of `RetainedLease`, or forgetting aliases/leases, violates the
contract; trusted in-process plugins are not isolated by the type system.
Built-in Resident follows the contract structurally. The store itself cannot
be cloned or publicly drained, and the topology writes **zero** terminal
destroy or revoke-fence code — those remain framework-owned for every topology,
built-in and custom alike. The async hooks
are plain `async fn` in trait (RPITIT) — do **not** annotate your
`impl Topology<R>` block with `#[async_trait]` (`Provider` still needs it;
`Topology` does not). A non-pooling
topology that carries credential slots must declare `handles_own_revoke` and
provide the corresponding revoke policy, such as `dispatch_credential_hook`
for a retained shared instance. Typed and resolved registration reject an
unsupported policy with a permanent error, including declared-but-unbound slots.

### Shared resource pattern

When multiple workflows acquire the same `Resource` impl at the same scope,
the manager deduplicates by `(R::key(), ScopeLevel, SlotIdentity)`. Config
`fingerprint()` participates in freshness/reload policy, not registry row
identity. Within one Resident row, concurrent leases share its current retained
runtime; initial creation is serialized, while recreation may create a successor.

This is the foundation of the "one bot, ten workflows" headline: a single
Telegram bot client serving many concurrent workflow nodes without
re-authenticating, re-warming connections, or contending for rate limits
across duplicate clients.

#### Telegram bot example

`rust,ignore`: `TelegramBot`, `bot_config`, and `build_workflow_resource_ctx`
are illustrative stand-ins, not real types in this crate — for a fully
compiling register→acquire example, see the doctest on `Manager::register`.

```rust,ignore
use std::sync::Arc;

use nebula_resource::{
    AcquireOptions, Manager, RegistrationSpec, Resident, ResidentConfig, ScopeLevel,
    dedup::SlotIdentity,
};

// One bot, registered once at organization scope through the single funnel.
let manager = Arc::new(Manager::new());
let bot = TelegramBot::new(/* construct from credentials */);
manager.register(RegistrationSpec {
    resource: bot,
    config: bot_config,
    scope: ScopeLevel::Organization(org_id),
    slot_identity: SlotIdentity::Unbound,
    topology: Resident::new(ResidentConfig::default()),
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

// `Provider::create` was invoked exactly once; every acquirer holds a
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
- **Manager shutdown**. `Manager::shutdown()` cancels the manager token; in-flight acquires drain via `graceful_shutdown` per canon §11.4. Cleanup stays open during handle drain. After shutdown, every acquire returns `ErrorKind::Cancelled`.
