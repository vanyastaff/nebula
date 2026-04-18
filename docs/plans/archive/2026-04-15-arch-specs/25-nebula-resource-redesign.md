# 25 — `nebula-resource` redesign

> **Status:** DRAFT
> **Authority:** subordinate to `docs/PRODUCT_CANON.md`. Canon wins on conflict.
> **Parent:** [`./README.md`](./README.md)
> **Scope:** Align `nebula-resource` with specs 23 (cross-crate foundation) and
> 24 (nebula-core redesign). Rename ResourceHandle→ResourceGuard, replace Ctx
> with ResourceContext, integrate credential rotation, add typed access extension.
> **Depends on:** 23 (Context/Guard/Dependencies), 24 (nebula-core redesign),
> 22 (credential events)
> **Consumers:** `nebula-action`, `nebula-engine`, `nebula-testing`, plugin crates

## 1. Problem

`nebula-resource` has a mature topology/runtime/recovery system (38 files,
7 topology types) that works well. However it predates spec 23 decisions
and has these misalignments:

1. **`ResourceHandle<R>`** named inconsistently with `CredentialGuard<S>` —
   spec 23 unified naming convention says `*Guard` for RAII wrappers
2. **`Ctx` trait + `BasicCtx`** — local context with String-typed `ScopeLevel`
   and unused `Extensions` type-map (0 consumers outside ctx.rs)
3. **Local `ScopeLevel`** — uses `String` for Organization/Project variants,
   spec 24 `nebula-core::ScopeLevel` uses typed IDs
4. **No Guard/TypedGuard** trait implementations on ResourceHandle
5. **No `DeclaresDependencies`** — resource-to-resource and resource-to-credential
   dependencies are undeclared
6. **No credential rotation** — Manager has config hot-reload (`reload_config`)
   but no integration with `CredentialEvent` from spec 22
7. **No typed access** — actions use `manager.acquire::<R>(key, ctx, opts)`,
   no ergonomic `ctx.resource::<R>()` extension trait
8. **`compat.rs`** — deprecated, should be deleted
9. **`ReloadOutcome`** — designed in resource-hld.md plans but not implemented;
   current `reload_config` only handles Pool topology (fingerprint swap)

### 1.1 What stays unchanged

The following subsystems are well-designed and require only `&dyn Ctx` →
`&ResourceContext` signature updates:

- **7 topology traits** (Pooled, Resident, Service, Transport, Exclusive,
  EventSource, Daemon) — architecture validated by 4 prototypes (Postgres,
  Google Sheets, Telegram, SSH)
- **7 topology runtimes** + `TopologyRuntime<R>` dispatch enum
- **Manager** — registry, acquire dispatch, shutdown orchestration
- **Registry** — type-erased `AnyManagedResource` storage
- **Recovery** — gate, group, watchdog (thundering herd prevention)
- **ReleaseQueue** — async cleanup workers
- **Cell** — ArcSwap for Resident topology
- **Metrics** — ResourceOpsMetrics counters
- **Events** — ResourceEvent lifecycle events
- **State** — ResourcePhase/ResourceStatus
- **Error** — Error/ErrorKind/ErrorScope with Classify impl
- **Integration** — AcquireResilience retry/circuit-breaker config
- **ResourceConfig** trait, **ResourceMetadata**, **AnyResource** trait

## 2. Decision

Targeted changes to align with spec 23/24 without rewriting working
infrastructure. Six concrete changes:

1. **Rename** `ResourceHandle<R>` → `ResourceGuard<R>` + add Guard/TypedGuard impls
2. **Replace** `Ctx`/`BasicCtx`/`Extensions`/local `ScopeLevel` → `ResourceContext` struct
3. **Add** `HasResourcesExt` extension trait for typed `ctx.resource::<R>()` access
4. **Add** credential rotation path on Manager via `CredentialEvent` subscription
5. **Implement** per-topology `ReloadOutcome` dispatch (from resource-hld plans)
6. **Delete** `compat.rs`

## 3. Changes

### 3.1 ResourceHandle → ResourceGuard

File rename: `handle.rs` → `guard.rs`.
Type rename: `ResourceHandle<R>` → `ResourceGuard<R>`, `HandleInner` → `GuardInner`.

Add `acquired_at: Instant` field to Owned variant (was `Duration::ZERO`).

Add trait implementations:

```rust
impl<R: Resource> nebula_core::Guard for ResourceGuard<R> {
    fn guard_kind(&self) -> &'static str { "resource" }
    fn acquired_at(&self) -> Instant { self.acquired_at }
}

impl<R: Resource> nebula_core::TypedGuard for ResourceGuard<R> {
    type Inner = R::Lease;
    fn as_inner(&self) -> &Self::Inner { self } // delegates to Deref
}
```

Existing `Deref<Target = R::Lease>`, `Drop`, `Debug` (redacted), `#[must_use]`,
`taint()`, `detach()`, `hold_duration()` — unchanged except field rename.

### 3.2 Ctx → ResourceContext

**Delete entirely:** `ctx.rs` (Ctx trait, BasicCtx, Extensions, local ScopeLevel,
ctx_ext function — 235 lines).

**New file:** `context.rs`

```rust
use std::sync::Arc;
use nebula_core::{
    context::{Context, BaseContext},
    context::capability::{HasResources, HasCredentials},
    accessor::{ResourceAccessor, CredentialAccessor},
    scope::{Scope, Principal},
    obs::TraceId,
    Clock,
};
use tokio_util::sync::CancellationToken;

/// Domain context for resource lifecycle methods (create, check, shutdown, destroy).
///
/// Narrower than ActionContext — provides access to other resources and
/// credentials (for resource-to-resource deps and auth resolution) but no
/// node identity, trigger scheduling, or event emission.
pub struct ResourceContext {
    base: BaseContext,
    resources: Arc<dyn ResourceAccessor>,
    credentials: Arc<dyn CredentialAccessor>,
}

impl ResourceContext {
    pub fn new(
        base: BaseContext,
        resources: Arc<dyn ResourceAccessor>,
        credentials: Arc<dyn CredentialAccessor>,
    ) -> Self {
        Self { base, resources, credentials }
    }
}

impl Context for ResourceContext { /* delegate to base */ }
impl HasResources for ResourceContext {
    fn resources(&self) -> &dyn ResourceAccessor { &*self.resources }
}
impl HasCredentials for ResourceContext {
    fn credentials(&self) -> &dyn CredentialAccessor { &*self.credentials }
}
```

### 3.3 Resource trait update

```rust
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Runtime: Send + Sync + 'static;
    type Lease: Send + Sync + 'static;
    type Error: std::error::Error + Send + Sync + Into<crate::Error> + 'static;
    type Auth: AuthScheme;

    fn key() -> ResourceKey;

    fn create(
        &self,
        config: &Self::Config,
        auth: &Self::Auth,
        ctx: &ResourceContext,          // was: &dyn Ctx
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    fn check(
        &self, runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }

    fn shutdown(
        &self, runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }

    fn destroy(
        &self, runtime: Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }

    fn metadata() -> ResourceMetadata { ResourceMetadata::from_key(&Self::key()) }
}
```

`auth` parameter stays — validated by 4 prototypes (Postgres, Google Sheets,
Telegram, SSH). Credential pre-resolved by engine, passed typed.

### 3.4 Topology trait updates

All topology traits that accept `&dyn Ctx` switch to `&ResourceContext`:

```rust
pub trait Pooled: Resource {
    fn is_broken(&self, runtime: &Self::Runtime) -> BrokenCheck { BrokenCheck::Healthy }
    fn recycle(&self, runtime: &Self::Runtime, metrics: &InstanceMetrics)
        -> impl Future<Output = Result<RecycleDecision, Self::Error>> + Send;
    fn prepare(&self, runtime: &Self::Runtime, ctx: &ResourceContext)    // was &dyn Ctx
        -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }
}

pub trait Service: Resource {
    const TOKEN_MODE: TokenMode;
    fn acquire_token(&self, runtime: &Self::Runtime, ctx: &ResourceContext) // was &dyn Ctx
        -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;
    fn release_token(&self, _runtime: &Self::Runtime, _token: Self::Lease) 
        -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }
}

// Transport, Exclusive — same pattern: &dyn Ctx → &ResourceContext
// Resident — no ctx in methods (Clone-based)
// EventSource — no ctx (subscription-based)
// Daemon — uses CancellationToken directly, no ctx change needed
```

**`Pooled::prepare()` note:** current prototype uses `ctx.ext::<TenantContext>()`
for per-tenant `SET search_path`. After Extensions removal, resource impl reads
`ctx.scope().workspace_id` and derives tenant config. Or uses scoped resource
via `ResourceAction::configure()` (plan 10) which sets `search_path` in config.

### 3.5 HasResourcesExt extension trait

**New file:** `ext.rs`

```rust
use nebula_core::context::capability::HasResources;
use crate::{Resource, ResourceGuard, error::{Error, ErrorKind}};

/// Typed resource access for any context implementing HasResources.
///
/// Primary API for action/trigger authors: `ctx.resource::<Postgres>().await?`
pub trait HasResourcesExt: HasResources {
    fn resource<R: Resource>(&self)
        -> impl Future<Output = Result<ResourceGuard<R>, Error>> + Send
    where Self: Sized;

    fn try_resource<R: Resource>(&self)
        -> impl Future<Output = Result<Option<ResourceGuard<R>>, Error>> + Send
    where Self: Sized;
}

impl<C: HasResources + ?Sized> HasResourcesExt for C {
    async fn resource<R: Resource>(&self) -> Result<ResourceGuard<R>, Error> {
        self.resources().acquire_typed::<R>().await
    }

    async fn try_resource<R: Resource>(&self) -> Result<Option<ResourceGuard<R>>, Error> {
        match self.resource::<R>().await {
            Ok(guard) => Ok(Some(guard)),
            Err(e) if e.kind() == &ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}
```

Blanket impl means this works on `ActionRuntimeContext`, `TriggerRuntimeContext`,
`ResourceContext`, `TestContext` — any type that implements `HasResources`.

### 3.6 Credential rotation via ReloadOutcome

**New file:** `reload.rs`

```rust
/// Internal result of per-topology reload dispatch.
/// Used by Manager for both config changes and credential rotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReloadOutcome {
    /// Applied immediately. Next acquire gets fresh config/credential.
    SwappedImmediately,
    /// Old runtime draining via Arc refcount (Service topology).
    PendingDrain { old_generation: u64 },
    /// Daemon cancelled + restarting.
    Restarting,
    /// Fingerprint identical, credential unchanged.
    NoChange,
}
```

**TopologyRuntime gains `reload()` method:**

```rust
impl<R: Resource> TopologyRuntime<R> {
    pub(crate) async fn reload(
        &self,
        resource: &R,
        config: &R::Config,
        auth: &R::Auth,
        ctx: &ResourceContext,
    ) -> Result<ReloadOutcome, crate::Error> {
        match self {
            Self::Pool(pool) => { /* fingerprint check → set_fingerprint */ }
            Self::Resident(resident) => { /* destroy old → create new → ArcSwap */ }
            Self::Service(service) => { /* create new → swap → old drains via Arc */ }
            Self::Daemon(daemon) => { /* cancel → restart */ }
            Self::Transport(_) | Self::Exclusive(_) => { /* destroy → create */ }
            Self::EventSource(_) => { /* unsubscribe → resubscribe */ }
        }
    }
}
```

**Manager expanded:**

```rust
impl Manager {
    /// Existing: config hot-reload (expanded to use ReloadOutcome).
    pub async fn reload_config<R: Resource>(
        &self, new_config: R::Config, scope: &nebula_core::ScopeLevel,
    ) -> Result<ReloadOutcome, Error> { /* ... */ }

    /// New: credential rotation handler.
    /// Subscribed to EventBus<CredentialEvent> at Manager construction.
    pub(crate) async fn on_credential_refreshed(
        &self, credential_id: CredentialId,
    ) -> Result<(), Error> { /* lookup affected resources → reload each */ }

    pub(crate) async fn on_credential_revoked(
        &self, credential_id: CredentialId,
    ) -> Result<(), Error> { /* force drain + stop accepting */ }
}
```

**Manager tracks credential_id → resource_key mapping:**

```rust
// In ManagedResource (runtime/managed.rs):
pub(crate) struct ManagedResource<R: Resource> {
    pub(crate) resource: R,
    pub(crate) config: ArcSwap<R::Config>,
    pub(crate) topology: TopologyRuntime<R>,
    pub(crate) credential_id: Option<CredentialId>,  // NEW: tracked at registration
    pub(crate) status: ArcSwap<ResourceStatus>,
    pub(crate) generation: AtomicU64,
    // ...
}
```

### 3.7 Delete compat.rs

Remove `compat.rs` and its re-exports from `lib.rs`:

```rust
// DELETE from lib.rs:
// #[allow(deprecated)]
// pub use compat::{Context, Scope};
```

### 3.8 Manager gains LayerLifecycle

Manager receives `LayerLifecycle` at construction (child of engine layer, spec 08).
Internal shutdown uses `LayerLifecycle::shutdown(grace)` instead of custom drain logic.

```rust
impl Manager {
    pub fn new(lifecycle: LayerLifecycle, config: ManagerConfig) -> Self { ... }
}
```

Existing `ShutdownConfig` / `DrainTimeoutPolicy` become parameters to the
lifecycle-based shutdown, not a separate mechanism.

## 4. File changes

| Action | File | Lines |
|---|---|---|
| **Rename** | `handle.rs` → `guard.rs` | ~550 (rename + ~30 new Guard/TypedGuard impls) |
| **Delete** | `ctx.rs` | -235 |
| **Delete** | `compat.rs` | ~50 |
| **New** | `context.rs` | ~60 |
| **New** | `ext.rs` | ~40 |
| **New** | `reload.rs` | ~80 |
| **Update** | `resource.rs` | `&dyn Ctx` → `&ResourceContext` |
| **Update** | `manager.rs` | +credential rotation, +LayerLifecycle, reload_config expanded |
| **Update** | `runtime/managed.rs` | +credential_id field |
| **Update** | `runtime/mod.rs` | +reload() method on TopologyRuntime |
| **Update** | `runtime/*.rs` (7 files) | `&dyn Ctx` → `&ResourceContext` in signatures |
| **Update** | `topology/*.rs` (7 files) | `&dyn Ctx` → `&ResourceContext` in trait methods |
| **Update** | `lib.rs` | New re-exports, remove compat, rename ResourceHandle |
| **Update** | `events.rs` | No change (TracedEvent wrapping at eventbus layer) |
| **Update** | `error.rs` | No change (already well-designed) |

**Net:** -235 (ctx.rs) -50 (compat.rs) +180 (context+ext+reload) +30 (Guard impls) = ~-75 lines.
Plus mechanical `Ctx` → `ResourceContext` updates in ~20 call sites.

## 5. Cargo.toml changes

```toml
[dependencies]
# Nebula ecosystem
nebula-core = { path = "../core" }          # gains Context, Guard, Dependencies traits
nebula-metrics = { path = "../metrics" }
nebula-resilience = { path = "../resilience" }
nebula-resource-macros = { path = "macros" }
nebula-telemetry = { path = "../telemetry" }
nebula-error = { workspace = true }

# NO nebula-credential dependency — credential access via accessor traits from core

# Async runtime
tokio = { workspace = true, features = ["rt", "sync", "time", "macros"] }
tokio-util = { workspace = true }

# Core (unchanged)
arc-swap = { workspace = true }
dashmap = { workspace = true }
smallvec = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
```

No new external dependencies. No removed dependencies.

## 6. DeclaresDependencies for Resource

`#[derive(Resource)]` macro (in `nebula-resource-macros`) auto-generates
`DeclaresDependencies` from the `Auth` associated type + explicit attributes:

```rust
#[derive(Resource)]
#[uses_credential(DatabaseCredential)]          // from Auth type — could be auto-derived
#[uses_resource(CacheResource, optional, purpose = "query caching")]
struct PostgresResource;

// Generated:
impl DeclaresDependencies for PostgresResource {
    fn dependencies() -> Dependencies {
        Dependencies::new()
            .credential(CredentialRequirement::of::<DatabaseCredential>())
            .resource(ResourceRequirement::of::<CacheResource>()
                .optional()
                .purpose("query caching"))
    }
}
```

For resources with `type Auth = ()` — no credential requirement generated.

## 7. Edge cases

### 7.1 Credential rotation for scoped resources

Per plan 10: scoped resources are short-lived (per-execution). Credential
rotation events are irrelevant — the resource is destroyed when execution
completes. Manager only tracks `credential_id` for global resources.

### 7.2 Multiple resources sharing one credential

Manager reverse-index: `credential_id → Vec<ResourceKey>`. One credential
rotation triggers reload for all resources using that credential.

### 7.3 Revoked credential

`CredentialEvent::Revoked` is more severe than `Refreshed`:
- Stop accepting new acquires immediately
- Taint all outstanding guards
- Wait for drain (grace from LayerLifecycle)
- Emit `ResourceEvent::HealthChanged { healthy: false }`

### 7.4 Pooled::prepare() without Extensions

After `Extensions` removal, `prepare()` reads tenant info from
`ResourceContext::scope()` (has `workspace_id`, `org_id`). Resource impl
maps workspace_id → search_path. For complex mapping: inject lookup service
at registration via resource struct field, not via type-map.

Alternative: scoped resources via `ResourceAction::configure()` (plan 10)
provide per-execution config including `search_path` — no prepare() needed.

### 7.5 ReloadOutcome::PendingDrain tracking

Service topology returns `PendingDrain { old_generation }`. Manager must
track pending drains and report them in health snapshots. Old runtime
deallocated when last Arc ref drops — no explicit cleanup needed.

## 8. Testing criteria

- `ResourceGuard`: all existing ResourceHandle tests pass under new name +
  Guard/TypedGuard trait methods return correct values
- `ResourceContext`: implements Context + HasResources + HasCredentials,
  scope/principal/cancellation accessible
- `HasResourcesExt`: `ctx.resource::<R>()` resolves through accessor,
  `try_resource` returns None for missing
- `ReloadOutcome`: per-topology reload returns correct variant
- Credential rotation: mock CredentialEvent → Manager reloads affected resources
- Credential revocation: stop acquires + taint + drain
- `compat.rs` removal: no compile errors
- All existing topology/runtime/recovery tests pass with ctx type change

## 9. Performance targets

- `ResourceGuard` Guard trait methods: <10ns (field access)
- `HasResourcesExt::resource::<R>()`: same latency as current `manager.acquire()`
- Credential rotation: <100ms from event to first new-credential acquire
- ReloadOutcome dispatch: <1ms per topology (excluding create() time)
- No regression in existing acquire/release/pool benchmarks

## 10. Migration path

### PR 1: Rename + Guard impls (no breaking API)

1. `handle.rs` → `guard.rs`, `ResourceHandle` → `ResourceGuard`
2. Add `acquired_at` to Owned variant
3. Implement `nebula_core::Guard` + `TypedGuard`
4. `pub type ResourceHandle<R> = ResourceGuard<R>` deprecated alias
5. All tests green

### PR 2: Context migration

1. Delete `ctx.rs` + `compat.rs`
2. Add `context.rs` (ResourceContext struct)
3. Update `Resource::create` signature: `&dyn Ctx` → `&ResourceContext`
4. Update all topology traits: `&dyn Ctx` → `&ResourceContext`
5. Update all topology runtimes: pass ResourceContext
6. Update Manager: construct ResourceContext for lifecycle calls
7. Fix downstream compile errors (nebula-engine, nebula-action tests)

### PR 3: Extensions (typed access + reload + credential rotation)

1. Add `ext.rs` (HasResourcesExt)
2. Add `reload.rs` (ReloadOutcome)
3. Implement `TopologyRuntime::reload()` per-topology dispatch
4. Expand `Manager::reload_config()` to use ReloadOutcome
5. Add `Manager::on_credential_refreshed/revoked()`
6. Add `credential_id` tracking in ManagedResource
7. Subscribe Manager to `EventBus<CredentialEvent>`
8. Add LayerLifecycle to Manager constructor

### PR 4: DeclaresDependencies

1. Update `#[derive(Resource)]` macro to generate DeclaresDependencies
2. Add `#[uses_credential]` / `#[uses_resource]` attributes to macro

## 11. Open questions

### 11.1 ResourceAccessor::acquire_typed

`HasResourcesExt::resource::<R>()` calls `self.resources().acquire_typed::<R>()`.
`ResourceAccessor` (defined in nebula-core) is dyn-safe and uses `acquire_any()`
with `TypeId`-based dispatch. The concrete `acquire_typed::<R>()` method lives
as extension method on `ResourceAccessor` (not on the trait itself, since
generics break dyn-safety). Exact API shape deferred to implementation PR.

### 11.2 prepare() tenant resolution pattern

Document recommended pattern for per-tenant prepare() without Extensions:
workspace_id → lookup service → search_path. Or recommend scoped resources
(ResourceAction) as the primary per-tenant isolation mechanism.

### 11.3 ToolProvider trait (spec 27 §9.5)

New optional trait for resources that provide AI agent tools:

```rust
pub trait ToolProvider: Resource {
    fn tool_defs() -> Vec<ToolDef> where Self: Sized;
    fn call_tool(name: &str, input: Value, lease: &Self::Lease)
        -> impl Future<Output = Result<Value, ActionError>> + Send
    where Self: Sized;
}
```

`ResourceMetadata` gains `tools: Vec<ToolDef>` field. Added in PR
after spec 27 action types are implemented. Full design in spec 27 §9.5.
