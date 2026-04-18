# 23 — Cross-crate foundation: Context, Guard, Dependencies, Scope

> **Status:** DRAFT
> **Authority:** subordinate to `docs/PRODUCT_CANON.md`. Canon wins on conflict.
> **Parent:** [`./README.md`](./README.md)
> **Scope:** Defines cross-cutting abstractions (context, guards, dependencies,
> scope) that live in `nebula-core` and are used uniformly across all business
> crates (`nebula-resource`, `nebula-credential`, `nebula-action`, `nebula-engine`,
> `nebula-testing`).
> **Depends on:** none — this spec is foundation for everything else.
> **Consumers:** all business-layer crates + `nebula-engine` + `nebula-testing`
> **Related specs:** 02 (tenancy), 06 (IDs), 13 (workflow versioning), 14 (stateful),
> 17 (multi-process), 18 (observability), 21 (schema), 22 (credential v3)
> **Supersedes parts of:** existing `nebula-resource::ctx`, `nebula-credential::context`,
> `nebula-action::context`, `nebula-action::dependency::ActionDependencies`

## 1. Problem

Multiple business crates currently duplicate cross-cutting concerns with
inconsistent shapes, making it hard to compose them and adding friction
to every change.

### 1.1 Context types duplicated across 5 places

- `nebula-resource::Ctx` trait + `BasicCtx` struct
- `nebula-credential::CredentialContext` struct
- `nebula-action::ActionContext` struct
- `nebula-action::TriggerContext` struct
- `nebula-action::Context` base trait

Each has its own `execution_id`, `cancellation_token`, `workflow_id`
fields (or similar). Every time we add a cross-cutting field (e.g. `trace_id`
from spec 18, `workspace_id` from spec 02, `clock` for `TestClock` support
from spec 20), we touch N crates.

**Additional problem**: `nebula-resource::Ctx` uses a type-map `Extensions`
for extension data, while `CredentialContext` uses explicit fields —
two inconsistent patterns for the same concept.

### 1.2 RAII wrappers named inconsistently

- `nebula-credential::CredentialGuard<S: Zeroize>` — RAII + zeroize on drop
- `nebula-resource::ResourceHandle<R: Resource>` — RAII + release-to-pool on drop

Both are RAII wrappers with typed parameters and Drop-based cleanup. They
should share a uniform naming convention (`Guard` for both). The current
split — `Guard` vs `Handle` — reflects history, not semantic distinction.

### 1.3 Dependencies model is fragmented

`ActionDependencies` trait currently exposes 5 parallel methods:

```rust
fn credential() -> Option<Box<dyn AnyCredential>>;
fn resources() -> Vec<Box<dyn AnyResource>>;
fn credential_keys() -> Vec<CredentialKey>;
fn resource_keys() -> Vec<ResourceKey>;
fn credential_types() -> Vec<TypeId>;
```

Three parallel representations (boxed objects / keys / TypeIds) of the same
information. Plus **single credential only** (`Option<_>` instead of `Vec<_>`) —
actions needing multiple credentials (e.g., source+destination integration)
cannot declare them.

Neither resources nor credentials declare dependencies on other components
today. `CachedHttpResource` wrapping `HttpResource + RedisResource` has no
declarative surface; `OAuth2Credential` needing `HttpResource` for refresh
(spec 22 deferred) has no way to express it.

### 1.4 Scope types misaligned with spec 02

`nebula-resource::ScopeLevel` uses `String` for `Organization` and `Project`:

```rust
pub enum ScopeLevel {
    Global,
    Organization(String),      // should be OrgId (spec 06)
    Project(String),           // should be Workspace (spec 02) + WorkspaceId (spec 06)
    Workflow(WorkflowId),
    Execution(ExecutionId),
}
```

And identity fields (`workflow_version_id` from spec 13, `attempt_id` from
spec 14, `trace_id` from spec 18, `instance_id` from spec 17) are scattered
across different contexts without a unified identity container.

### 1.5 Root cause

Each crate grew its own abstractions for identity, dependencies, and
lifetime management because there was no shared foundation. This spec
defines that foundation in `nebula-core` so all business crates can
build on top of it uniformly.

## 2. Decision

Introduce four foundational modules in `nebula-core`, consumed by all
business crates.

### 2.1 Context (Q1)

Layered trait hierarchy with capability-based composition:

```
Context (base — identity / tenancy / lifecycle / clock)
    ├── HasResources (adds: resources() -> &dyn ResourceAccessor)
    ├── HasCredentials (adds: credentials() -> &dyn CredentialAccessor)
    ├── HasLogger (adds: logger() -> &dyn Logger)
    ├── HasMetrics (adds: metrics() -> &dyn MetricsEmitter)
    ├── HasEventBus (adds: eventbus() -> &dyn EventEmitter)
    ├── HasNodeIdentity [action] (adds: node_id + attempt_id)
    └── HasTriggerScheduling [trigger] (adds: scheduler + emitter + health)
```

Umbrella marker traits via blanket impls:

```rust
// nebula-action
pub trait ActionContext:
    Context + HasResources + HasCredentials + HasLogger
    + HasMetrics + HasEventBus + HasNodeIdentity
{}
impl<T> ActionContext for T where
    T: Context + HasResources + HasCredentials + HasLogger
       + HasMetrics + HasEventBus + HasNodeIdentity
{}

pub trait TriggerContext:
    Context + HasResources + HasCredentials + HasLogger
    + HasMetrics + HasEventBus + HasTriggerScheduling
{}
```

Diamond inheritance — `ActionContext` and `TriggerContext` are **siblings**
both extending `HasResources + HasCredentials`, neither subtype of the other
(LSP compliance — triggers are not a special kind of action).

**Concrete structs** in `nebula-engine`:

- `ActionRuntimeContext` implements `ActionContext`
- `TriggerRuntimeContext` implements `TriggerContext`

Both internally share `BaseContext` for DRY.

**Internal domain contexts** in respective crates (for `Credential::resolve`
and `Resource::create` callbacks — narrower surface than umbrella):

- `nebula-credential::CredentialContext` — `BaseContext` + `refresh_coordinator` + `HasResources` (for OAuth2 refresh via HttpResource)
- `nebula-resource::ResourceContext` — `BaseContext` + `HasResources + HasCredentials` (for resource-to-resource dependencies)

**Test context** in `nebula-testing`:

- `TestContext` struct implementing all capability traits with spy/mock defaults

### 2.2 Guards (Q2)

Unified `Guard` trait in `nebula-core`, two concrete types in domain crates:

```rust
// nebula-core::guard
pub trait Guard: Send + Sync + 'static {
    fn guard_kind(&self) -> &'static str;
    fn acquired_at(&self) -> std::time::Instant;
    fn age(&self) -> std::time::Duration { self.acquired_at().elapsed() }
}

pub trait TypedGuard: Guard {
    type Inner: ?Sized;
    fn as_inner(&self) -> &Self::Inner;
}
```

Concrete:

- `nebula-credential::CredentialGuard<S: Zeroize>` (existing, gains `Guard` impl)
- `nebula-resource::ResourceGuard<R: Resource>` (**renamed** from `ResourceHandle`, gains `Guard` impl)

Both implement `Deref` (to `S` and `R::Lease` respectively), `Drop` (zeroize
and release-to-pool), `#[must_use]`, redacted `Debug`. **Neither** implements
`Serialize`, `Deserialize`, or `Display` — compile error if inserted into
serialized output.

**Typed access via extension traits**:

```rust
// nebula-resource
pub trait HasResourcesExt: HasResources {
    async fn resource<R: Resource>(&self) -> Result<ResourceGuard<R>, ResourceError>
    where Self: Sized;

    async fn try_resource<R: Resource>(&self)
        -> Result<Option<ResourceGuard<R>>, ResourceError>
    where Self: Sized;
}
impl<C: HasResources + ?Sized> HasResourcesExt for C { /* ... */ }

// nebula-credential
pub trait HasCredentialsExt: HasCredentials {
    async fn credential<C: Credential>(&self)
        -> Result<CredentialGuard<C::Scheme>, CredentialError>
    where Self: Sized;

    async fn try_credential<C: Credential>(&self)
        -> Result<Option<CredentialGuard<C::Scheme>>, CredentialError>
    where Self: Sized;
}
impl<Ctx: HasCredentials + ?Sized> HasCredentialsExt for Ctx { /* ... */ }
```

**Author-facing code** is fully typed with no string keys:

```rust
let pool = ctx.resource::<PostgresResource>().await?;
let token = ctx.credential::<GithubToken>().await?;
```

### 2.3 Dependencies (Q3)

Unified `Dependencies` container and `DeclaresDependencies` trait in `nebula-core`:

```rust
// nebula-core::dependencies
pub struct Dependencies {
    credentials: Vec<CredentialRequirement>,
    resources: Vec<ResourceRequirement>,
}

pub trait DeclaresDependencies {
    fn dependencies() -> Dependencies where Self: Sized;
}
```

`Action`, `Resource`, and `Credential` all extend `DeclaresDependencies`.

**Derive macro attributes** (same names on all three derives):

```rust
#[uses_credential(Type)]                              // required, no purpose
#[uses_credential(Type, optional)]                    // optional
#[uses_credential(Type, purpose = "why")]             // required with purpose
#[uses_credential(Type, optional, purpose = "why")]   // both

#[uses_credentials([Type1, Type2(optional, purpose = "telemetry")])]  // bulk

#[uses_resource(Type[, purpose = "why"])]
#[uses_resources([Type1, Type2(purpose = "cache")])]
```

Derive macro aggregates single + bulk attributes into a single
`DeclaresDependencies` impl. Compile-time type assertions ensure each
referenced type implements the correct trait.

**Multi-credential support** (was `Option<_>`, now `Vec<_>`):

```rust
#[derive(Action)]
#[action(key = "github.sync_to_slack")]
#[uses_credentials([GithubToken, SlackBotToken])]
#[uses_resource(HttpResource)]
struct SyncAction;
```

**Resource-to-resource** and **credential-to-resource** dependencies supported
via `ResourceContext: HasResources` and `CredentialContext: HasResources`.

**Credential-to-credential is forbidden** — compile error with guidance
pointing to `uses_resource(HttpResource)` (for OAuth2 refresh) or external
provider pattern (for bootstrap credentials).

### 2.4 Scope (Q4)

Five-variant enum for registration granularity, typed identity fields in a
separate struct:

```rust
// nebula-core::scope
pub enum ScopeLevel {
    Global,
    Organization(OrgId),
    Workspace(WorkspaceId),
    Workflow(WorkflowId),
    Execution(ExecutionId),
}

pub struct Scope {
    // Registration-granularity fields
    pub org_id: Option<OrgId>,
    pub workspace_id: Option<WorkspaceId>,
    pub workflow_id: Option<WorkflowId>,
    pub execution_id: Option<ExecutionId>,

    // Additional identity fields (for observability, version-aware caching, etc.)
    pub workflow_version_id: Option<WorkflowVersionId>,
    pub attempt_id: Option<AttemptId>,
    pub node_id: Option<NodeId>,
    pub trigger_id: Option<TriggerId>,
    pub instance_id: Option<InstanceId>,
}
```

**Access rule**: `caller.scope.can_access(&target.scope_level)` — target scope
must contain caller scope (target is broader-or-equal).

**Registration rule**: `dep.scope ⊇ dependent.scope` for all declared dependencies.

**`Global` semantics**: per-engine-instance global. Cross-instance sharing
requires external coordination (database-backed cache, message broker).
Documented explicitly.

### 2.5 Module boundaries — where each type lives

| Module | Crate | Contents |
|---|---|---|
| `context` | `nebula-core` | `Context`, `BaseContext`, `HasX` capability traits |
| `guard` | `nebula-core` | `Guard`, `TypedGuard`, debug helpers |
| `dependencies` | `nebula-core` | `Dependencies`, `DeclaresDependencies`, `Requirement`s |
| `scope` | `nebula-core` | `ScopeLevel`, `Scope` |
| `accessor` | `nebula-core` | Interface traits (`ResourceAccessor`, `CredentialAccessor`, `Clock`, `Logger`, `MetricsEmitter`, `EventEmitter`, `RefreshCoordinator`) |
| Action-specific capabilities | `nebula-action` | `HasNodeIdentity`, `HasTriggerScheduling`, umbrella `ActionContext`/`TriggerContext` |
| Internal contexts | domain crates | `CredentialContext` (nebula-credential), `ResourceContext` (nebula-resource) |
| Concrete runtime contexts | `nebula-engine` | `ActionRuntimeContext`, `TriggerRuntimeContext` |
| Test context | `nebula-testing` | `TestContext` + `TestContextBuilder` |

## 3. Data model

### 3.1 `nebula-core::context`

```rust
// nebula-core/src/context/mod.rs
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::{
    accessor::{Clock, ResourceAccessor, CredentialAccessor, Logger, MetricsEmitter, EventEmitter},
    scope::Scope,
    OrgId, WorkspaceId, WorkflowId, ExecutionId,
    obs::{TraceId, SpanId, Principal},
};

/// Identity + tenancy + observability fields shared by all contexts.
///
/// Domain-specific contexts embed `BaseContext` and delegate `Context` methods
/// to it via macro or manual impl.
#[derive(Clone)]
pub struct BaseContext {
    pub scope: Scope,
    pub principal: Principal,
    pub trace_id: Option<TraceId>,
    pub span_id: Option<SpanId>,
    pub cancellation: CancellationToken,
    pub clock: Arc<dyn Clock>,
}

impl BaseContext {
    pub fn builder() -> BaseContextBuilder { BaseContextBuilder::default() }
}

// Builder in same file — fluent construction
#[derive(Default)]
pub struct BaseContextBuilder {
    scope: Scope,
    principal: Option<Principal>,
    trace_id: Option<TraceId>,
    span_id: Option<SpanId>,
    cancellation: Option<CancellationToken>,
    clock: Option<Arc<dyn Clock>>,
}

impl BaseContextBuilder {
    pub fn scope(mut self, scope: Scope) -> Self { self.scope = scope; self }
    pub fn principal(mut self, p: Principal) -> Self { self.principal = Some(p); self }
    pub fn trace_id(mut self, t: TraceId) -> Self { self.trace_id = Some(t); self }
    pub fn cancellation(mut self, c: CancellationToken) -> Self { self.cancellation = Some(c); self }
    pub fn clock(mut self, c: Arc<dyn Clock>) -> Self { self.clock = Some(c); self }

    pub fn build(self) -> BaseContext {
        BaseContext {
            scope: self.scope,
            principal: self.principal.expect("principal required"),
            trace_id: self.trace_id,
            span_id: self.span_id,
            cancellation: self.cancellation.unwrap_or_else(CancellationToken::new),
            clock: self.clock.expect("clock required"),
        }
    }
}

/// Base context trait — all contexts implement this.
pub trait Context: Send + Sync {
    fn scope(&self) -> &Scope;
    fn principal(&self) -> &Principal;
    fn trace_id(&self) -> Option<&TraceId>;
    fn span_id(&self) -> Option<&SpanId>;
    fn cancellation(&self) -> &CancellationToken;
    fn clock(&self) -> &dyn Clock;

    // Convenience accessors
    fn execution_id(&self) -> Option<ExecutionId> { self.scope().execution_id }
    fn workflow_id(&self) -> Option<WorkflowId> { self.scope().workflow_id }
    fn workspace_id(&self) -> Option<WorkspaceId> { self.scope().workspace_id }
    fn org_id(&self) -> Option<OrgId> { self.scope().org_id }
}

impl Context for BaseContext {
    fn scope(&self) -> &Scope { &self.scope }
    fn principal(&self) -> &Principal { &self.principal }
    fn trace_id(&self) -> Option<&TraceId> { self.trace_id.as_ref() }
    fn span_id(&self) -> Option<&SpanId> { self.span_id.as_ref() }
    fn cancellation(&self) -> &CancellationToken { &self.cancellation }
    fn clock(&self) -> &dyn Clock { self.clock.as_ref() }
}

// ── Capability traits ──────────────────────────────────────────────────────

pub trait HasResources: Context {
    fn resources(&self) -> &dyn ResourceAccessor;
}

pub trait HasCredentials: Context {
    fn credentials(&self) -> &dyn CredentialAccessor;
}

pub trait HasLogger: Context {
    fn logger(&self) -> &dyn Logger;
}

pub trait HasMetrics: Context {
    fn metrics(&self) -> &dyn MetricsEmitter;
}

pub trait HasEventBus: Context {
    fn eventbus(&self) -> &dyn EventEmitter;
}
```

### 3.2 `nebula-core::accessor` — interface traits

```rust
// nebula-core/src/accessor/mod.rs
use std::{any::Any, future::Future, sync::Arc};
use chrono::{DateTime, Utc};
use crate::{CredentialKey, ResourceKey};

/// Clock abstraction for deterministic testing.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
    fn monotonic(&self) -> std::time::Instant;
}

/// System clock — real time source.
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> { Utc::now() }
    fn monotonic(&self) -> std::time::Instant { std::time::Instant::now() }
}

/// Log level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel { Trace, Debug, Info, Warn, Error }

/// Scoped logger.
pub trait Logger: Send + Sync {
    fn log(&self, level: LogLevel, message: &str);
    fn log_with_fields(&self, level: LogLevel, message: &str, fields: &[(&str, &dyn std::fmt::Debug)]);
}

/// Metrics emitter.
pub trait MetricsEmitter: Send + Sync {
    fn counter(&self, name: &str, value: u64, labels: &[(&str, &str)]);
    fn gauge(&self, name: &str, value: f64, labels: &[(&str, &str)]);
    fn histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]);
}

/// Event bus emitter.
pub trait EventEmitter: Send + Sync {
    fn emit(&self, topic: &str, payload: serde_json::Value);
}

/// Resource accessor — dyn-safe, returns type-erased handles.
///
/// Consumer code uses `HasResourcesExt::resource::<R>()` for typed access.
/// The `acquire_any` method is the internal entry point — not meant for direct use.
pub trait ResourceAccessor: Send + Sync {
    fn has(&self, key: &ResourceKey) -> bool;

    fn acquire_any<'a>(
        &'a self,
        key: &'a ResourceKey,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send + Sync>, Box<dyn std::error::Error + Send + Sync>>> + Send + 'a>>;

    fn try_acquire_any<'a>(
        &'a self,
        key: &'a ResourceKey,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Option<Box<dyn Any + Send + Sync>>, Box<dyn std::error::Error + Send + Sync>>> + Send + 'a>>;
}

/// Credential accessor — dyn-safe, returns type-erased guards.
///
/// Consumer code uses `HasCredentialsExt::credential::<C>()` for typed access.
pub trait CredentialAccessor: Send + Sync {
    fn has(&self, key: &CredentialKey) -> bool;

    fn resolve_any<'a>(
        &'a self,
        key: &'a CredentialKey,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send + Sync>, Box<dyn std::error::Error + Send + Sync>>> + Send + 'a>>;

    fn try_resolve_any<'a>(
        &'a self,
        key: &'a CredentialKey,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Option<Box<dyn Any + Send + Sync>>, Box<dyn std::error::Error + Send + Sync>>> + Send + 'a>>;
}

/// Refresh coordinator — single-flight per credential.
pub trait RefreshCoordinator: Send + Sync {
    fn acquire_refresh<'a>(
        &'a self,
        credential_id: &'a CredentialKey,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<RefreshToken, Box<dyn std::error::Error + Send + Sync>>> + Send + 'a>>;

    fn release_refresh(&self, token: RefreshToken);
}

pub struct RefreshToken(pub u64);
```

### 3.3 `nebula-core::guard`

```rust
// nebula-core/src/guard.rs
use std::{fmt, time::Instant};

/// Base trait for all RAII guards.
pub trait Guard: Send + Sync + 'static {
    /// Stable kind identifier for metrics labels, logs, debug format.
    fn guard_kind(&self) -> &'static str;

    /// When this guard was acquired — for lifetime tracking, metrics, expiry checks.
    fn acquired_at(&self) -> Instant;

    /// How long this guard has been held.
    fn age(&self) -> std::time::Duration {
        self.acquired_at().elapsed()
    }
}

/// Typed guard — exposes inner type for generic helpers.
pub trait TypedGuard: Guard {
    type Inner: ?Sized;
    fn as_inner(&self) -> &Self::Inner;
}

/// Helper: fully redacted Debug format.
///
/// Output: `Guard<credential>[REDACTED]`
pub fn debug_redacted<G: Guard>(g: &G, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Guard<{}>[REDACTED]", g.guard_kind())
}

/// Helper: Debug format with type info but no content.
///
/// Output: `Guard<resource, inner=PgPool, age=1.2s>`
pub fn debug_typed<G: TypedGuard>(g: &G, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(
        f,
        "Guard<{}, inner={}, age={:?}>",
        g.guard_kind(),
        std::any::type_name::<G::Inner>(),
        g.age()
    )
}
```

### 3.4 `nebula-core::dependencies`

```rust
// nebula-core/src/dependencies.rs
use std::any::TypeId;
use crate::{CredentialKey, ResourceKey};

#[derive(Debug, Clone, Default)]
pub struct Dependencies {
    credentials: Vec<CredentialRequirement>,
    resources: Vec<ResourceRequirement>,
}

impl Dependencies {
    pub fn new() -> Self { Self::default() }

    pub fn credential<C: 'static>(mut self) -> Self
    where
        C: CredentialLike,
    {
        self.credentials.push(CredentialRequirement::of::<C>());
        self
    }

    pub fn credential_with<C: 'static>(
        mut self,
        configure: impl FnOnce(CredentialRequirement) -> CredentialRequirement,
    ) -> Self
    where
        C: CredentialLike,
    {
        self.credentials.push(configure(CredentialRequirement::of::<C>()));
        self
    }

    pub fn optional_credential<C: 'static>(mut self) -> Self
    where
        C: CredentialLike,
    {
        self.credentials.push(CredentialRequirement::of::<C>().optional());
        self
    }

    pub fn resource<R: 'static>(mut self) -> Self
    where
        R: ResourceLike,
    {
        self.resources.push(ResourceRequirement::of::<R>());
        self
    }

    pub fn resource_with<R: 'static>(
        mut self,
        configure: impl FnOnce(ResourceRequirement) -> ResourceRequirement,
    ) -> Self
    where
        R: ResourceLike,
    {
        self.resources.push(configure(ResourceRequirement::of::<R>()));
        self
    }

    pub fn optional_resource<R: 'static>(mut self) -> Self
    where
        R: ResourceLike,
    {
        self.resources.push(ResourceRequirement::of::<R>().optional());
        self
    }

    // Accessors
    pub fn credentials(&self) -> &[CredentialRequirement] { &self.credentials }
    pub fn resources(&self) -> &[ResourceRequirement] { &self.resources }
    pub fn is_empty(&self) -> bool { self.credentials.is_empty() && self.resources.is_empty() }
}

/// Trait implemented by Action / Resource / Credential — declares their dependencies.
pub trait DeclaresDependencies {
    fn dependencies() -> Dependencies where Self: Sized;
}

/// Blanket default for types that have no dependencies.
/// Derive macro generates overriding impl when `#[uses_*]` attributes present.
pub trait CredentialLike {
    const KEY_STR: &'static str;
}

pub trait ResourceLike {
    const KEY_STR: &'static str;
}

// ── Requirements ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CredentialRequirement {
    key: CredentialKey,
    type_id: TypeId,
    required: bool,
    purpose: Option<&'static str>,
}

impl CredentialRequirement {
    pub fn of<C: CredentialLike + 'static>() -> Self {
        Self {
            key: CredentialKey::new(C::KEY_STR),
            type_id: TypeId::of::<C>(),
            required: true,
            purpose: None,
        }
    }

    pub fn optional(mut self) -> Self { self.required = false; self }
    pub fn purpose(mut self, purpose: &'static str) -> Self { self.purpose = Some(purpose); self }

    pub fn key(&self) -> &CredentialKey { &self.key }
    pub fn type_id(&self) -> TypeId { self.type_id }
    pub fn is_required(&self) -> bool { self.required }
    pub fn is_optional(&self) -> bool { !self.required }
    pub fn purpose_text(&self) -> Option<&'static str> { self.purpose }
}

#[derive(Debug, Clone)]
pub struct ResourceRequirement {
    key: ResourceKey,
    type_id: TypeId,
    required: bool,
    purpose: Option<&'static str>,
}

impl ResourceRequirement {
    pub fn of<R: ResourceLike + 'static>() -> Self {
        Self {
            key: ResourceKey::new(R::KEY_STR),
            type_id: TypeId::of::<R>(),
            required: true,
            purpose: None,
        }
    }

    pub fn optional(mut self) -> Self { self.required = false; self }
    pub fn purpose(mut self, purpose: &'static str) -> Self { self.purpose = Some(purpose); self }

    pub fn key(&self) -> &ResourceKey { &self.key }
    pub fn type_id(&self) -> TypeId { self.type_id }
    pub fn is_required(&self) -> bool { self.required }
    pub fn is_optional(&self) -> bool { !self.required }
    pub fn purpose_text(&self) -> Option<&'static str> { self.purpose }
}
```

### 3.5 `nebula-core::scope`

```rust
// nebula-core/src/scope.rs
use crate::{
    OrgId, WorkspaceId, WorkflowId, WorkflowVersionId,
    ExecutionId, AttemptId, NodeId, TriggerId, InstanceId,
};

/// Registration-granularity scope buckets for resources and credentials.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ScopeLevel {
    /// Per-engine-instance global. Cross-instance sharing requires
    /// external coordination (DB, message queue, etc.).
    Global,
    Organization(OrgId),
    Workspace(WorkspaceId),
    Workflow(WorkflowId),
    Execution(ExecutionId),
}

impl ScopeLevel {
    pub fn depth(&self) -> u8 {
        match self {
            Self::Global => 0,
            Self::Organization(_) => 1,
            Self::Workspace(_) => 2,
            Self::Workflow(_) => 3,
            Self::Execution(_) => 4,
        }
    }

    pub fn is_global(&self) -> bool { matches!(self, Self::Global) }
}

/// Full identity context — all IDs relevant to current execution chain.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    // Registration-granularity fields
    pub org_id: Option<OrgId>,
    pub workspace_id: Option<WorkspaceId>,
    pub workflow_id: Option<WorkflowId>,
    pub execution_id: Option<ExecutionId>,

    // Additional identity fields
    pub workflow_version_id: Option<WorkflowVersionId>,
    pub attempt_id: Option<AttemptId>,
    pub node_id: Option<NodeId>,
    pub trigger_id: Option<TriggerId>,
    pub instance_id: Option<InstanceId>,
}

impl Scope {
    pub fn global() -> Self { Self::default() }

    pub fn in_org(org_id: OrgId) -> Self {
        Self { org_id: Some(org_id), ..Default::default() }
    }

    pub fn in_workspace(org_id: OrgId, workspace_id: WorkspaceId) -> Self {
        Self {
            org_id: Some(org_id),
            workspace_id: Some(workspace_id),
            ..Default::default()
        }
    }

    /// Deepest set scope level — used to determine registration bucket.
    pub fn level(&self) -> ScopeLevel {
        if let Some(id) = self.execution_id {
            ScopeLevel::Execution(id)
        } else if let Some(id) = self.workflow_id {
            ScopeLevel::Workflow(id)
        } else if let Some(id) = self.workspace_id {
            ScopeLevel::Workspace(id)
        } else if let Some(id) = self.org_id {
            ScopeLevel::Organization(id)
        } else {
            ScopeLevel::Global
        }
    }

    /// Check whether this scope can access a resource registered at the given level.
    ///
    /// Strict containment: resource scope must be broader-or-equal to caller scope.
    pub fn can_access(&self, registered: &ScopeLevel) -> bool {
        match registered {
            ScopeLevel::Global => true,
            ScopeLevel::Organization(o) => self.org_id == Some(*o),
            ScopeLevel::Workspace(w) => self.workspace_id == Some(*w),
            ScopeLevel::Workflow(w) => self.workflow_id == Some(*w),
            ScopeLevel::Execution(e) => self.execution_id == Some(*e),
        }
    }
}

/// Principal — who initiated this operation.
#[derive(Debug, Clone)]
pub enum Principal {
    User(UserId),
    ServiceAccount(ServiceAccountId),
    Workflow { workflow_id: WorkflowId, trigger: Option<TriggerId> },
    System,
}
```

### 3.6 Domain capability traits (in `nebula-action`)

```rust
// nebula-action/src/context/capability.rs
use nebula_core::context::Context;
use nebula_core::{NodeId, AttemptId};
use crate::capability::{TriggerScheduler, ExecutionEmitter, TriggerHealth};
use std::sync::Arc;

pub trait HasNodeIdentity: Context {
    fn node_id(&self) -> NodeId;
    fn attempt_id(&self) -> AttemptId;
}

pub trait HasTriggerScheduling: Context {
    fn scheduler(&self) -> &dyn TriggerScheduler;
    fn emitter(&self) -> &dyn ExecutionEmitter;
    fn health(&self) -> &TriggerHealth;
}

// Umbrella marker traits — blanket-impl'd
pub trait ActionContext:
    Context
    + nebula_core::context::HasResources
    + nebula_core::context::HasCredentials
    + nebula_core::context::HasLogger
    + nebula_core::context::HasMetrics
    + nebula_core::context::HasEventBus
    + HasNodeIdentity
{}

impl<T: ?Sized> ActionContext for T where
    T: Context
       + nebula_core::context::HasResources
       + nebula_core::context::HasCredentials
       + nebula_core::context::HasLogger
       + nebula_core::context::HasMetrics
       + nebula_core::context::HasEventBus
       + HasNodeIdentity
{}

pub trait TriggerContext:
    Context
    + nebula_core::context::HasResources
    + nebula_core::context::HasCredentials
    + nebula_core::context::HasLogger
    + nebula_core::context::HasMetrics
    + nebula_core::context::HasEventBus
    + HasTriggerScheduling
{}

impl<T: ?Sized> TriggerContext for T where
    T: Context
       + nebula_core::context::HasResources
       + nebula_core::context::HasCredentials
       + nebula_core::context::HasLogger
       + nebula_core::context::HasMetrics
       + nebula_core::context::HasEventBus
       + HasTriggerScheduling
{}
```

### 3.7 Concrete runtime contexts (in `nebula-engine`)

```rust
// nebula-engine/src/context/action.rs
use std::sync::Arc;
use nebula_core::{
    context::{BaseContext, Context, HasResources, HasCredentials, HasLogger, HasMetrics, HasEventBus},
    accessor::{ResourceAccessor, CredentialAccessor, Logger, MetricsEmitter, EventEmitter, Clock},
    scope::{Scope, Principal},
    NodeId, AttemptId, obs::{TraceId, SpanId},
};
use tokio_util::sync::CancellationToken;
use nebula_action::context::{HasNodeIdentity, ActionContext};

pub struct ActionRuntimeContext {
    base: BaseContext,
    node_id: NodeId,
    attempt_id: AttemptId,
    resources: Arc<dyn ResourceAccessor>,
    credentials: Arc<dyn CredentialAccessor>,
    logger: Arc<dyn Logger>,
    metrics: Arc<dyn MetricsEmitter>,
    eventbus: Arc<dyn EventEmitter>,
}

impl ActionRuntimeContext {
    pub fn builder() -> ActionRuntimeContextBuilder { ActionRuntimeContextBuilder::default() }
}

// Base Context delegates to BaseContext
impl Context for ActionRuntimeContext {
    fn scope(&self) -> &Scope { self.base.scope() }
    fn principal(&self) -> &Principal { self.base.principal() }
    fn trace_id(&self) -> Option<&TraceId> { self.base.trace_id() }
    fn span_id(&self) -> Option<&SpanId> { self.base.span_id() }
    fn cancellation(&self) -> &CancellationToken { self.base.cancellation() }
    fn clock(&self) -> &dyn Clock { self.base.clock() }
}

impl HasResources for ActionRuntimeContext {
    fn resources(&self) -> &dyn ResourceAccessor { self.resources.as_ref() }
}

impl HasCredentials for ActionRuntimeContext {
    fn credentials(&self) -> &dyn CredentialAccessor { self.credentials.as_ref() }
}

impl HasLogger for ActionRuntimeContext {
    fn logger(&self) -> &dyn Logger { self.logger.as_ref() }
}

impl HasMetrics for ActionRuntimeContext {
    fn metrics(&self) -> &dyn MetricsEmitter { self.metrics.as_ref() }
}

impl HasEventBus for ActionRuntimeContext {
    fn eventbus(&self) -> &dyn EventEmitter { self.eventbus.as_ref() }
}

impl HasNodeIdentity for ActionRuntimeContext {
    fn node_id(&self) -> NodeId { self.node_id }
    fn attempt_id(&self) -> AttemptId { self.attempt_id }
}

// Automatic: implements ActionContext via blanket impl
```

Similarly `TriggerRuntimeContext` implements `Context + HasResources + HasCredentials + HasLogger + HasMetrics + HasEventBus + HasTriggerScheduling` → automatic `TriggerContext` via blanket.

### 3.8 Internal domain contexts

```rust
// nebula-credential/src/context.rs
use std::sync::Arc;
use nebula_core::{
    context::{BaseContext, Context, HasResources},
    accessor::{ResourceAccessor, Clock, RefreshCoordinator, MetricsEmitter},
    scope::{Scope, Principal},
};

pub struct CredentialContext {
    base: BaseContext,
    refresh_coordinator: Arc<dyn RefreshCoordinator>,
    metrics: Arc<dyn MetricsEmitter>,
    // HasResources for OAuth2 refresh via HttpResource (spec 22)
    resources: Arc<dyn ResourceAccessor>,
    // NOTE: no HasCredentials — prevents infinite recursion during resolution
}

impl Context for CredentialContext { /* delegates to base */ }
impl HasResources for CredentialContext {
    fn resources(&self) -> &dyn ResourceAccessor { self.resources.as_ref() }
}

impl CredentialContext {
    pub fn refresh_coordinator(&self) -> &dyn RefreshCoordinator {
        self.refresh_coordinator.as_ref()
    }
    pub fn metrics(&self) -> &dyn MetricsEmitter { self.metrics.as_ref() }
}

// nebula-resource/src/context.rs
pub struct ResourceContext {
    base: BaseContext,
    // Resource can acquire other resources (for composed resources)
    resources: Arc<dyn ResourceAccessor>,
    // Resource can resolve credentials during create() (Resource::Auth binding)
    credentials: Arc<dyn CredentialAccessor>,
}

impl Context for ResourceContext { /* delegates to base */ }
impl HasResources for ResourceContext {
    fn resources(&self) -> &dyn ResourceAccessor { self.resources.as_ref() }
}
impl HasCredentials for ResourceContext {
    fn credentials(&self) -> &dyn CredentialAccessor { self.credentials.as_ref() }
}
```

### 3.9 `CredentialGuard` and `ResourceGuard`

```rust
// nebula-credential/src/guard.rs
use std::{fmt, ops::Deref, time::Instant};
use nebula_core::guard::{Guard, TypedGuard, debug_redacted};
use zeroize::Zeroize;

#[must_use = "dropping a CredentialGuard immediately zeroizes the secret"]
pub struct CredentialGuard<S: Zeroize> {
    inner: S,
    acquired_at: Instant,
}

impl<S: Zeroize> CredentialGuard<S> {
    pub fn new(inner: S) -> Self {
        Self { inner, acquired_at: Instant::now() }
    }
}

impl<S: Zeroize> Guard for CredentialGuard<S> {
    fn guard_kind(&self) -> &'static str { "credential" }
    fn acquired_at(&self) -> Instant { self.acquired_at }
}

impl<S: Zeroize> TypedGuard for CredentialGuard<S> {
    type Inner = S;
    fn as_inner(&self) -> &S { &self.inner }
}

impl<S: Zeroize> Deref for CredentialGuard<S> {
    type Target = S;
    fn deref(&self) -> &S { &self.inner }
}

impl<S: Zeroize> Drop for CredentialGuard<S> {
    fn drop(&mut self) { self.inner.zeroize(); }
}

impl<S: Zeroize + Clone> Clone for CredentialGuard<S> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), acquired_at: Instant::now() }
    }
}

impl<S: Zeroize> fmt::Debug for CredentialGuard<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_redacted(self, f)
    }
}

// NO Serialize, NO Display, NO Default — compile errors enforce protection

// nebula-resource/src/guard.rs (renamed from handle.rs)
#[must_use = "dropping a ResourceGuard releases the resource to the pool"]
pub struct ResourceGuard<R: Resource> {
    inner: HandleInner<R>,
    resource_key: ResourceKey,
    topology_tag: TopologyTag,
    drain_counter: Option<Arc<(AtomicU64, Notify)>>,
    acquired_at: Instant,
}

impl<R: Resource> Guard for ResourceGuard<R> {
    fn guard_kind(&self) -> &'static str { "resource" }
    fn acquired_at(&self) -> Instant { self.acquired_at }
}

impl<R: Resource> TypedGuard for ResourceGuard<R> {
    type Inner = R::Lease;
    fn as_inner(&self) -> &R::Lease { /* extract from HandleInner */ }
}

impl<R: Resource> Deref for ResourceGuard<R> {
    type Target = R::Lease;
    fn deref(&self) -> &R::Lease { self.as_inner() }
}

impl<R: Resource> Drop for ResourceGuard<R> {
    fn drop(&mut self) {
        // existing release logic from ResourceHandle
    }
}

impl<R: Resource> fmt::Debug for ResourceGuard<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResourceGuard")
            .field("kind", &self.guard_kind())
            .field("resource_key", &self.resource_key)
            .field("age", &self.age())
            .finish_non_exhaustive()
    }
}

// NO Serialize, NO Display — compile errors enforce protection
```

### 3.10 Extension traits for typed access

```rust
// nebula-resource/src/ext.rs
use nebula_core::context::HasResources;
use crate::{ResourceGuard, Resource, ResourceError};

pub trait HasResourcesExt: HasResources {
    async fn resource<R: Resource>(&self) -> Result<ResourceGuard<R>, ResourceError>
    where Self: Sized
    {
        let boxed = self.resources().acquire_any(&R::key()).await
            .map_err(|e| ResourceError::Backend(e))?;
        boxed.downcast::<ResourceGuard<R>>()
            .map(|b| *b)
            .map_err(|_| ResourceError::TypeMismatch {
                expected: std::any::type_name::<R>(),
                key: R::key(),
            })
    }

    async fn try_resource<R: Resource>(&self) -> Result<Option<ResourceGuard<R>>, ResourceError>
    where Self: Sized
    {
        match self.resources().try_acquire_any(&R::key()).await {
            Ok(None) => Ok(None),
            Ok(Some(boxed)) => {
                let guard = boxed.downcast::<ResourceGuard<R>>()
                    .map(|b| Some(*b))
                    .map_err(|_| ResourceError::TypeMismatch {
                        expected: std::any::type_name::<R>(),
                        key: R::key(),
                    })?;
                Ok(guard)
            }
            Err(e) => Err(ResourceError::Backend(e)),
        }
    }
}

impl<C: HasResources + ?Sized> HasResourcesExt for C {}

// nebula-credential/src/ext.rs
pub trait HasCredentialsExt: HasCredentials {
    async fn credential<C: Credential>(&self) -> Result<CredentialGuard<C::Scheme>, CredentialError>
    where Self: Sized
    {
        let boxed = self.credentials().resolve_any(&C::key()).await
            .map_err(|e| CredentialError::Backend(e))?;
        boxed.downcast::<CredentialGuard<C::Scheme>>()
            .map(|b| *b)
            .map_err(|_| CredentialError::TypeMismatch {
                expected: std::any::type_name::<C>(),
                key: C::key(),
            })
    }

    async fn try_credential<C: Credential>(&self) -> Result<Option<CredentialGuard<C::Scheme>>, CredentialError>
    where Self: Sized
    { /* analogous to try_resource */ }
}

impl<C: HasCredentials + ?Sized> HasCredentialsExt for C {}
```

## 4. Flows

### 4.1 Context construction at execution start

```
1. Engine receives trigger → creates execution record
2. Engine constructs BaseContext:
   - scope: Scope { org_id, workspace_id, workflow_id, workflow_version_id,
                     execution_id, attempt_id: AttemptId(1), node_id: None, ... }
   - principal: Principal::Workflow { workflow_id, trigger: Some(trigger_id) }
   - trace_id: generated for new execution
   - cancellation: child of engine's cancellation
   - clock: real clock (or TestClock in tests)
3. Engine initializes capability providers:
   - ManagerResourceAccessor (holds Weak<Manager>)
   - ManagerCredentialAccessor (holds Weak<Manager>)
   - ScopedLogger (tagged with execution_id)
   - Metrics emitter (labelled with workspace/workflow)
   - EventBus publisher
4. Engine builds ActionRuntimeContext for each node in DAG:
   - Updates node_id + attempt_id as it dispatches nodes
   - Passes to StatelessAction::execute(input, &ctx)
```

### 4.2 Resource acquisition with dependency resolution

```
Action code:
  let pool = ctx.resource::<PostgresResource>().await?;

Flow:
1. HasResourcesExt::resource::<PostgresResource>() called
2. Internally: self.resources().acquire_any(&PostgresResource::key()).await
3. ManagerResourceAccessor looks up registered resource
4. Manager validates scope: ctx.scope().can_access(&resource_scope)?
   - If denied → ResourceError::ScopeDenied
5. Manager calls Resource::create(&config, &auth, &resource_context)
   - resource_context = constructed internally with Weak<Manager>-backed accessor
   - If resource depends on other resources (declared via #[uses_resource]):
     → Resource::create() calls resource_context.resource::<Dep>().await?
     → Recursive acquire through same Manager
     → Dependency chain traversed bottom-up
6. Acquired ResourceGuard<PostgresResource> returned
7. Action dereferences: pool.execute(...).await?
8. Action drops guard → release callback returns lease to pool
```

### 4.3 Multi-credential action

```rust
#[derive(Action)]
#[uses_credentials([GithubToken, SlackBotToken])]
#[uses_resource(HttpResource)]
struct SyncAction;

impl StatelessAction for SyncAction {
    async fn execute<C: ActionContext>(&self, input: Input, ctx: &C) -> Result<...> {
        // Two credentials resolved independently
        let gh: CredentialGuard<SecretToken> = ctx.credential::<GithubToken>().await?;
        let slack: CredentialGuard<SecretToken> = ctx.credential::<SlackBotToken>().await?;
        let http: ResourceGuard<HttpResource> = ctx.resource::<HttpResource>().await?;

        // Use both credentials to cross-reference APIs
        let prs = http.get("https://api.github.com/...")
            .bearer_auth(gh.expose_secret(|t| t.to_string()))
            .send().await?;

        http.post("https://slack.com/api/chat.postMessage")
            .bearer_auth(slack.expose_secret(|t| t.to_string()))
            .json(&message).send().await?;

        Ok(ActionResult::Success(...))
    }
}
```

Both credentials resolved in parallel. Each has own `CredentialGuard`, each
auto-zeroizes on drop.

### 4.4 Resource-to-resource dependency chain

```rust
#[derive(Resource)]
#[resource(key = "cached_http", topology = "resident", auth = "()")]
#[uses_resources([HttpResource, RedisResource(purpose = "cache backend")])]
struct CachedHttpResource;

impl Resource for CachedHttpResource {
    type Config = CachedHttpConfig;
    type Runtime = CachedHttpRuntime;
    type Lease = CachedHttpRuntime;
    type Error = CachedHttpError;
    type Auth = ();

    async fn create(
        &self,
        config: &Self::Config,
        _auth: &(),
        ctx: &ResourceContext,
    ) -> Result<Self::Runtime, Self::Error> {
        // ResourceContext implements HasResources → typed acquire
        let http = ctx.resource::<HttpResource>().await?;
        let redis = ctx.resource::<RedisResource>().await?;

        Ok(CachedHttpRuntime::new(http, redis, config))
    }
}
```

Flow:

```
Action requests CachedHttpResource →
  Manager checks dep graph: [HttpResource, RedisResource]
  Manager acquires inner resources first (topological order):
    1. HttpResource → creates (no deps)
    2. RedisResource → creates (needs RedisCredential — resolved first)
  Then Manager acquires CachedHttpResource:
    - Builds ResourceContext with HasResources impl
    - Calls CachedHttpResource::create(config, (), &rctx)
    - create() calls rctx.resource::<HttpResource>() → returns ResourceGuard<HttpResource>
    - create() calls rctx.resource::<RedisResource>() → returns ResourceGuard<RedisResource>
    - create() wraps them into CachedHttpRuntime
  ResourceGuard<CachedHttpResource> returned to action
```

### 4.5 Dynamic credential per-execution

```
Action declares: #[uses_credential(DynamicPostgresCredential)]

Execution starts → engine pre-checks all declared deps:
  - DynamicPostgresCredential has DYNAMIC = true → flagged ephemeral
  - Static credentials (if any) pre-resolved at action start
  - Dynamic credentials lazy-resolved on first access

Action code: let db = ctx.credential::<DynamicPostgresCredential>().await?;
  - CredentialAccessor sees DYNAMIC = true
  - Skips cache, always resolves fresh
  - Calls DynamicPostgresCredential::resolve(values, &cred_context)
  - Vault issues new credential with lease_id + TTL
  - CredentialGuard<IdentityPassword> returned

Execution ends (success or failure):
  - Framework iterates live dynamic leases for this execution
  - For each: calls Credential::release(state, &ctx)
  - Dynamic Postgres revokes Vault lease → connection dropped at Vault side
  - AuditEvent::DynamicReleased emitted
```

### 4.6 Scope access denied

```rust
// Resource registered at Workspace scope
manager.register::<WorkspaceResource>(RegisterOptions {
    scope: ScopeLevel::Workspace(WorkspaceId::parse("ws_abc").unwrap()),
    ...
});

// Caller from different workspace
let ctx = /* execution in workspace ws_xyz */;
let result = ctx.resource::<WorkspaceResource>().await;

// Flow:
//   1. HasResourcesExt::resource::<WorkspaceResource>()
//   2. acquire_any(&WorkspaceResource::key())
//   3. Manager looks up resource → scope = ScopeLevel::Workspace(ws_abc)
//   4. Manager calls caller_scope.can_access(&ScopeLevel::Workspace(ws_abc))
//      - self.workspace_id == Some(ws_xyz), but checking against ws_abc → false
//      - Returns false
//   5. Manager returns ResourceError::ScopeDenied {
//        resource: WorkspaceResource::key(),
//        registered_at: ScopeLevel::Workspace(ws_abc),
//        accessed_from: ScopeLevel::Execution(exe_...),
//      }
//   6. HasResourcesExt::resource() wraps and returns Err
```

### 4.7 DAG cycle detection at registration

```
Scenario: circular dependency
  ResourceA → requires ResourceB
  ResourceB → requires ResourceA

Flow:
1. manager.register::<ResourceA>(RegisterOptions { ... })
   - Reads ResourceA::dependencies() → [ResourceB]
   - Stores in pending graph
2. manager.register::<ResourceB>(RegisterOptions { ... })
   - Reads ResourceB::dependencies() → [ResourceA]
   - Adds edge
3. manager.validate_dependencies()
   - Runs Tarjan SCC on dependency graph
   - Detects cycle: { ResourceA, ResourceB }
   - Returns Err(RegistrationError::DependencyCycle {
       cycle: [ResourceKey("a"), ResourceKey("b"), ResourceKey("a")]
     })
4. Engine fails to start — operator fixes by removing one edge
```

## 5. Edge cases

### 5.1 Arc cycle prevention in accessor

`ResourceAccessor` impl holds `Arc<Manager>`. `Manager` holds `ResourceGuard`s
with callbacks referencing Manager. Potential `Arc` cycle.

**Solution**: Use `Weak<Manager>` inside `ManagerResourceAccessor`. Upgrade to
`Arc` only during `acquire_any` call, drop immediately after. Manager lifetime
is anchored by outer owner (engine), not by accessor references.

```rust
pub struct ManagerResourceAccessor {
    manager: std::sync::Weak<Manager>,
}

#[async_trait::async_trait]
impl ResourceAccessor for ManagerResourceAccessor {
    async fn acquire_any(&self, key: &ResourceKey) -> Result<Box<dyn Any + Send + Sync>, _> {
        let manager = self.manager.upgrade()
            .ok_or("manager has been dropped")?;
        manager.acquire_any_internal(key).await
    }
}
```

### 5.2 In-flight circular acquire during create()

Even with static DAG validation, runtime dynamic state could trigger recursive
acquire (e.g., resource creation triggers another resource whose create calls
back). Supplementary check:

```rust
impl Manager {
    thread_local! {
        static IN_FLIGHT: RefCell<HashSet<ResourceKey>> = RefCell::new(HashSet::new());
    }

    async fn acquire_any_internal(&self, key: &ResourceKey) -> Result<Box<dyn Any + Send + Sync>, _> {
        IN_FLIGHT.with(|s| {
            if !s.borrow_mut().insert(key.clone()) {
                return Err(Error::RuntimeCycle { key: key.clone() });
            }
            Ok(())
        })?;

        let _guard = scopeguard::guard((), |_| {
            IN_FLIGHT.with(|s| { s.borrow_mut().remove(key); });
        });

        // Proceed with acquire
        ...
    }
}
```

Static Tarjan check at registration catches most cases; in-flight tracker
catches runtime-only cycles.

### 5.3 Missing credential type at runtime

Action declares `#[uses_credential(GithubToken)]`, engine starts, but no
`GithubToken` credential instance configured in the workspace. At first
access:

```
ctx.credential::<GithubToken>().await
  → CredentialAccessor::resolve_any(&GithubToken::key())
  → Manager looks up → not found for current workspace
  → Returns Err(CredentialError::NotConfigured { key, workspace })
```

Engine may pre-validate at execution start (fail fast) rather than at first
access (fail late). Trade-off: pre-validation adds startup overhead but
catches config errors earlier. Default: **pre-validate at execution start**
for required deps; optional deps (`try_credential`) lazy-resolved.

### 5.4 Optional dependency that exists but fails to resolve

```rust
#[uses_credential(MetricsCredential, optional)]

// Action code
let metrics = ctx.try_credential::<MetricsCredential>().await.ok().flatten();

// If MetricsCredential exists but resolve fails (e.g., API unreachable):
//   - Err returned from try_credential
//   - Action decides: fail action (.?) or continue without metrics (.ok())
//   - try_ variant returns Result<Option<_>, _> — distinguishes "not configured"
//     from "configured but failed"
```

`.ok().flatten()` → `None` on either "not configured" or "failed to resolve".
Action author can distinguish by checking the error if needed.

### 5.5 Scope type mismatch at compile time

```rust
// Expected: ResourceKey
let k: ResourceKey = CredentialKey::new("x");  // ← compile error

// DeclaresDependencies macro generates compile-time assertions:
//   fn _assert_resource<R: Resource>() {}
//   _assert_resource::<GithubToken>();  // ← fails: GithubToken doesn't impl Resource

// Parser prevents swapping `uses_credential` and `uses_resource`:
#[uses_credential(HttpResource)]  // ← macro expansion triggers _assert_credential::<HttpResource>()
                                  // HttpResource doesn't impl Credential → compile error
```

### 5.6 `Debug` impls accidentally added to guards

Rust allows `#[derive(Debug)]` which would expose inner content. Guards
use **manual `impl Debug`** that delegates to `debug_redacted` or `debug_typed`.

Additional guard: no `#[derive(Debug)]` on `CredentialGuard` — would require
`S: Debug` bound which most secret types have, and we don't want standard
Debug exposing them. Explicit manual impl prevents accident.

### 5.7 `Clone` on `CredentialGuard`

`CredentialGuard<S: Zeroize + Clone>` allows cloning when `S: Clone`. Rare
(most secret types are move-only). When cloned:

- `inner.clone()` — likely creates new allocation with same bytes
- `acquired_at = Instant::now()` — new guard has fresh timestamp
- Both guards will zeroize on drop — but memory regions are different, so
  both cleanups are safe

Concern: more copies = more memory to wipe. Best practice: don't clone
secrets unless absolutely necessary.

### 5.8 Cross-workspace cross-reference

Resource registered at `Workspace(A)` referenced by action in `Workspace(B)`.
At registration time, Manager doesn't know future callers — DAG check only
compares declared dependencies by TypeId. Scope violation caught at
**runtime** via `can_access`.

Future consideration: registration-time cross-scope check if dependent is
also declared. E.g., if `ActionX` is registered at `Workspace(B)` scope AND
declares `uses_resource(SharedResourceA)` where `SharedResourceA` lives in
`Workspace(A)`, registration fails with `ScopeTooNarrow`.

### 5.9 `TestContext` missing capability

Test authors build `TestContext` with fluent builder. Fields have defaults
(no-op logger, empty resource accessor, etc.). If test exercises action that
calls `ctx.resource::<T>()` without adding mock for `T`, `try_acquire_any`
returns `None` → test-specific error.

```rust
let ctx = TestContext::builder()
    .workspace_id(ws_id)
    .with_mock_resource::<MockPg>()  // registers MockPg
    .build();

// Action accessing Postgres works
// Action accessing UnknownResource fails with helpful test error
```

## 6. Configuration surface

`nebula-core` is a library crate with no runtime configuration. All
behavior is controlled by:

- Compile-time: types, trait impls, derive macro attributes
- Runtime: `BaseContextBuilder` / `TestContextBuilder` parameter injection

### 6.1 `Cargo.toml` features

```toml
[package]
name = "nebula-core"

[features]
default = []
# Test helpers — enables fluent builders not needed in prod builds
test-helpers = []
```

Business crates opt-in to test helpers: `nebula-core = { path = "...", features = ["test-helpers"] }`.

## 7. Testing criteria

### 7.1 `nebula-core` unit tests

- **`Scope::level()`** — 5 cases (deepest field set determines level)
- **`Scope::can_access()`** — containment matrix: 5 scope levels × 5 scope levels = 25 cases, verify truth table
- **`Dependencies` builder** — fluent chain produces correct Vec
- **`CredentialRequirement::of::<C>()`** — populates key/type_id from `C::KEY_STR`
- **`Guard::age()`** — returns `acquired_at.elapsed()`
- **`debug_redacted` / `debug_typed`** — output format matches expected

### 7.2 Context construction tests

- **`BaseContextBuilder::build()`** — required fields enforced
- **`ActionRuntimeContextBuilder`** — all capabilities set, `ActionContext` blanket impl fires
- **`TriggerRuntimeContextBuilder`** — same for trigger
- **`TestContext`** — builds with all capability traits, default Noop impls

### 7.3 Capability trait integration tests

For each capability trait (`HasResources`, `HasCredentials`, ...), a test:
- Implements custom impl of the trait
- Verifies `ActionContext` blanket impl fires when all capabilities present
- Verifies compile error if any capability missing

### 7.4 Derive macro tests

- **`#[uses_credential(Type)]`** on Action generates `DeclaresDependencies` impl with one credential
- **`#[uses_credentials([...])]`** bulk form generates multi-credential
- **Mixed single + bulk** merges into single `Dependencies`
- **`#[uses_credential(HttpResource)]`** → compile error (type doesn't impl `Credential`)
- **Credential derive with `#[uses_credential(Other)]`** → compile error (cred→cred forbidden)
- **Purpose extraction**: `#[uses_credential(X, purpose = "text")]` sets purpose field

### 7.5 Runtime integration tests

- **Resource acquire through `ctx.resource::<R>()`** — returns typed guard
- **Credential resolve through `ctx.credential::<C>()`** — returns typed guard
- **Multi-credential action** — two `ctx.credential::<_>()` calls succeed
- **Resource-to-resource dep** — inner resource acquired during create()
- **Scope denied** — cross-workspace access returns error
- **Dependency cycle** — Tarjan SCC detects at registration
- **Runtime cycle** — in-flight tracker catches recursive acquire

### 7.6 Compile-fail tests (trybuild)

```rust
// No Serialize on CredentialGuard
fn _reject_serialize() {
    let guard: CredentialGuard<SecretToken> = /* ... */;
    let _ = serde_json::to_string(&guard);  // compile error: CredentialGuard does not implement Serialize
}

// No Display on guards
fn _reject_display() {
    let guard: CredentialGuard<SecretToken> = /* ... */;
    println!("{}", guard);  // compile error: CredentialGuard does not implement Display
}

// Uses_credential on non-credential type
#[derive(Action)]
#[uses_credential(HttpResource)]  // compile error: HttpResource does not implement Credential
struct BadAction;

// Credential → credential forbidden
#[derive(Credential)]
#[uses_credential(OtherCred)]  // compile error: credentials cannot depend on other credentials
struct CircularCred;
```

## 8. Performance targets

| Operation | Target | Rationale |
|---|---|---|
| `BaseContext::builder().build()` | < 1 µs | Called once per execution |
| `Scope::can_access()` | < 10 ns | Called on every resource/credential acquire |
| `Scope::level()` | < 5 ns | Frequently read |
| Resource acquire (cache hit) | < 1 µs | Hot path per action |
| Resource acquire (cache miss, no deps) | < 10 µs | First acquire per resource type |
| Resource acquire with 3 deps | < 30 µs | Dependency chain traversal |
| Credential resolve (static, cache hit) | < 1 µs | Hot path |
| `Dependencies::new().credential::<C>()...build()` | zero alloc beyond `Vec::push` | Registration time only |
| Derive macro expansion | < 50 ms per type | Build time |

Measured via `criterion` benches in `crates/core/benches/`:

- `bench_scope.rs` — `can_access` truth table
- `bench_context.rs` — Context construction + capability access
- `bench_dependencies.rs` — Requirement building

## 9. Module boundaries

`nebula-core` sits at the foundation — no dependencies on other business crates.

```
Cross-cutting: nebula-error, nebula-telemetry (future)
            │
            ▼
nebula-core  (this spec — Context, Guard, Dependencies, Scope, Accessor traits)
     ▲
     │
     ├── nebula-schema  (extends core types, adds schema primitives)
     ├── nebula-validator  (already exists, core types from here)
     │
     ▼
Business layer:
     ├── nebula-resource  (implements Resource trait, uses ResourceContext, provides ManagerResourceAccessor)
     ├── nebula-credential  (implements Credential trait, uses CredentialContext, provides ManagerCredentialAccessor)
     │
     ▼
     ├── nebula-action  (domain capability traits + umbrella contexts + derive macros)
     │
     ▼
nebula-engine  (concrete runtime contexts, construction, wiring)
     │
     ▼
nebula-testing  (TestContext + builder + mocks for all capabilities)
```

**`nebula-core` depends on**: nothing from business layer. Only:
- `serde` (for typed IDs serialization)
- `thiserror` (for errors)
- `tokio-util` (for `CancellationToken`)
- `chrono` (for `DateTime<Utc>`)
- `zeroize` (for secret-aware types)

**`nebula-core` does NOT depend on**:
- Any business-layer crate (`nebula-resource`, `nebula-credential`, `nebula-action`)
- Any exec-layer crate (`nebula-engine`, `nebula-runtime`, `nebula-storage`)
- `tokio` runtime (types use standard `Future`, not runtime-specific)

Interface traits (`ResourceAccessor`, `CredentialAccessor`, `Clock`, ...)
live in core but concrete impls live in domain crates. Classic dependency
inversion — core defines "what", domains define "how".

## 10. Migration path

### 10.1 PR sequence

**PR 0 — this spec** (now)
Add `docs/plans/2026-04-15-arch-specs/23-cross-crate-foundation.md`. Link
from README.md and COMPACT.md.

**PR 1 — `nebula-core` foundation modules**
Add new modules to `nebula-core`:
- `src/context/` — `Context`, `BaseContext`, `BaseContextBuilder`, capability traits
- `src/accessor/` — interface traits (`Clock`, `Logger`, `MetricsEmitter`, etc.)
- `src/guard.rs` — `Guard`, `TypedGuard`, debug helpers
- `src/dependencies.rs` — `Dependencies`, `CredentialRequirement`, `ResourceRequirement`, `DeclaresDependencies`
- `src/scope.rs` — `ScopeLevel`, `Scope`, `Principal`

Requires typed IDs from spec 06 (`OrgId`, `WorkspaceId`, `InstanceId`,
`AttemptId`, `WorkflowVersionId`, `TriggerId`). Introduce any missing ID
types as part of this PR.

Green on `cargo check -p nebula-core && cargo nextest run -p nebula-core`.

**PR 2 — `nebula-resource` migration**
- Rename `ResourceHandle<R>` → `ResourceGuard<R>`
- Impl `Guard` + `TypedGuard` on `ResourceGuard`
- Rename internal `Ctx` → reuse `nebula-core::Context`
- Introduce `ResourceContext` struct implementing `HasResources + HasCredentials`
- Delete local `ScopeLevel` — use `nebula-core::scope::ScopeLevel`
- Add `DeclaresDependencies` impls (start with default empty)
- Update `Resource::create` signature: `ctx: &dyn Ctx` → `ctx: &ResourceContext`
- Add `ManagerResourceAccessor { weak: Weak<Manager> }`
- Impl `ResourceAccessor` on `ManagerResourceAccessor`
- Update all internal call sites
- Green on full workspace check

**PR 3 — `nebula-credential` migration**
- Impl `Guard` + `TypedGuard` on `CredentialGuard<S: Zeroize>`
- Rework `CredentialContext` to embed `BaseContext` + resource accessor
- Update `Credential::resolve / refresh / test / revoke` signatures to use new context
- Migrate `nebula-parameter` → `nebula-schema` from spec 21 (can be combined if done together)
- Add `DeclaresDependencies` — credentials can `#[uses_resource(...)]`
- Update OAuth2 credential to demo resource-based HTTP if feasible (optional for this PR)
- Green on full workspace check

**PR 4 — `nebula-action` migration**
- Delete old `ActionContext` struct
- Introduce capability traits: `HasNodeIdentity`, `HasTriggerScheduling`
- Introduce umbrella traits `ActionContext` + `TriggerContext` with blanket impls
- Delete old `ActionDependencies` trait
- Update `Action` supertrait bounds: extend `DeclaresDependencies`
- Update domain traits (`StatelessAction`, `StatefulAction`, `TriggerAction`, ...) to accept `impl ActionContext` / `impl TriggerContext`
- Update derive macro to generate `DeclaresDependencies` from `#[uses_credential(...)]` / `#[uses_resource(...)]` attributes
- Support single + bulk attribute forms
- Add compile-time type assertions
- Green on full workspace check

**PR 5 — `nebula-engine` runtime contexts**
- Implement `ActionRuntimeContext` struct with all capability impls
- Implement `TriggerRuntimeContext` struct with all capability impls
- Add `BaseContextBuilder` usage at execution start
- Wire `ManagerResourceAccessor` / `ManagerCredentialAccessor` into contexts
- Green on full workspace check + integration tests

**PR 6 — `nebula-testing` test context**
- Add `TestContext` struct with all capability impls (all mockable)
- Add `TestContextBuilder` fluent API
- Add `SpyLogger`, `SpyMetrics`, `SpyEventBus`, `MockResourceAccessor`, `MockCredentialAccessor`
- Delete old `TestContextBuilder` in nebula-action (replaced)
- Green on full workspace check

**PR 7 — canon fold-in**
Update `docs/PRODUCT_CANON.md` §11.x (new section for cross-crate foundation
contract) referencing this spec. Remove any stale mentions of old context
/ dependency patterns.

### 10.2 Breaking changes

Internal to workspace (pre-1.0 — acceptable):

- `nebula-resource::ResourceHandle` → `nebula-resource::ResourceGuard`
- `nebula-resource::Ctx` / `BasicCtx` → `nebula-resource::ResourceContext`
- `nebula-resource::ScopeLevel` — moved to `nebula-core::scope`, variants renamed (`Project` → `Workspace`, `String` IDs → typed IDs)
- `nebula-credential::CredentialContext` — completely reworked (new fields)
- `nebula-action::ActionContext` — struct replaced by trait + runtime struct in engine
- `nebula-action::ActionDependencies` — replaced by `nebula-core::DeclaresDependencies`
- `nebula-action::Context` (base trait) — moved to `nebula-core::Context`

All breakages handled by PR 2–6. External users (none yet, pre-1.0) would
need to update imports and attribute syntax.

### 10.3 Attribute migration

Old:
```rust
// Action
fn credential() -> Option<Box<dyn AnyCredential>> { Some(Box::new(GithubToken)) }
```

New:
```rust
#[derive(Action)]
#[uses_credential(GithubToken)]
struct MyAction;
```

Derive macro generates the `DeclaresDependencies` impl. Old manual method
implementation ignored.

### 10.4 Runtime wire format

No runtime wire format changes. Context, scope, and dependencies are
in-memory structures — they never serialize to storage or network.

Credential / resource state (serialized to storage) remains governed by
spec 22 / existing resource schemas.

## 11. Open questions

### 11.1 Cross-workspace resource sharing

Strict containment means `Workspace(A)` resource inaccessible from
`Workspace(B)`. Future need: explicit `share_with(workspace_id)` mechanism
where a Global resource can be restricted to N workspaces, or a workspace
resource can be shared with specific sibling workspaces.

**Deferred**: not in v1. Add when operational need arises.

### 11.2 Global vs Instance renaming

Should `ScopeLevel::Global` be renamed to `ScopeLevel::Instance` to reflect
per-engine-instance semantics? Current decision: keep `Global` (hierarchy
convention, documentation note). Revisit if users confused.

### 11.3 Credential-to-credential dependencies

Currently forbidden to prevent recursion. If bootstrap credentials become
common (e.g., Vault token fetched via another OIDC credential), the
external provider pattern (spec 22) handles it without this dependency
edge. Watch for cases that external provider can't cover.

### 11.4 Manager split of responsibilities

`nebula-resource::Manager` is 2003 lines. This spec recommends (not mandates)
splitting:

- `manager/core.rs` — registration + lookup
- `manager/acquire.rs` — topology dispatch
- `manager/shutdown.rs` — phased shutdown
- `manager/metrics.rs` — ResourceOpsMetrics integration

Pure refactor for maintainability, no behavior change. Can be a separate PR
after PR 2.

### 11.5 Extension traits in prelude

`HasResourcesExt::resource::<R>()` requires import of `HasResourcesExt`.
Convention: every business crate exposes `prelude` module that re-exports
capability + extension traits. Action author:

```rust
use nebula_action::prelude::*;
// Brings: ActionContext trait, HasResourcesExt, HasCredentialsExt, Guard, ...
```

Single import covers typical action writing surface.

### 11.6 `RefreshCoordinator` placement

Currently in `nebula-core::accessor`. Alternative: `nebula-resilience`
(thundering herd prevention is general concurrency primitive). Spec 22
has this as open question. This spec places interface in core for now,
pending decision.

### 11.7 Observability span correlation

`span_id: Option<SpanId>` in `BaseContext` is informational. Actual span
management (creation, entry, exit) is done by `nebula-telemetry` using
`tracing` crate. Context just carries current span for correlation.
Open: should guards emit start/end spans automatically, or leave to caller?

### 11.8 Performance of extension trait pattern

Benchmark: does `HasResourcesExt::resource::<R>()` add measurable overhead
vs direct generic method? Expected: negligible (single downcast, static
dispatch). Validate with criterion once implemented.

### 11.9 Multiple instances of same resource type

Spec 02: workspace has 1 `PostgresResource` registered. What if user needs
TWO Postgres connections (main DB + analytics DB)?

Option A: separate types (`MainDbResource`, `AnalyticsDbResource`) — strict
type safety, verbose.

Option B: parameterized resource key (`PostgresResource::with_id("analytics")`)
— one type, runtime-distinguished instances. Requires extending `ResourceKey`
to include instance ID.

**Current recommendation**: separate types for v1 (matches spec 06 typed ID
philosophy). Revisit if pattern emerges.

### 11.10 `attempt_id` always present in ActionContext?

At execution start: `attempt_id = AttemptId(1)`. Retry → new attempt with
`attempt_id = AttemptId(2)`. But what about stateless-action-in-control-flow
executions that don't really "attempt" anything?

**Decision**: `attempt_id` is always set to `AttemptId(attempt_number)`
where `attempt_number ≥ 1`. Semantically represents "which execution pass
is this" — applies uniformly to retryable and non-retryable actions.

## Appendix A — Full `nebula-core` module layout after PR 1

```
crates/core/src/
├── lib.rs
├── error.rs                  (existing)
├── ids/                      (typed IDs from spec 06)
│   ├── mod.rs
│   ├── org.rs                (OrgId)
│   ├── workspace.rs          (WorkspaceId)
│   ├── workflow.rs           (WorkflowId, WorkflowVersionId)
│   ├── execution.rs          (ExecutionId, AttemptId)
│   ├── node.rs               (NodeId)
│   ├── instance.rs           (InstanceId — new, spec 17)
│   ├── trigger.rs            (TriggerId — new)
│   ├── credential.rs         (CredentialKey, UserId, ServiceAccountId)
│   └── resource.rs           (ResourceKey)
├── context/                  (new in this spec)
│   ├── mod.rs
│   ├── base.rs               (BaseContext, BaseContextBuilder)
│   ├── context_trait.rs      (Context trait)
│   └── capability.rs         (HasResources, HasCredentials, HasLogger, HasMetrics, HasEventBus)
├── accessor/                 (new in this spec)
│   ├── mod.rs
│   ├── clock.rs              (Clock trait, SystemClock)
│   ├── logger.rs             (Logger trait, LogLevel)
│   ├── metrics.rs            (MetricsEmitter trait)
│   ├── events.rs             (EventEmitter trait)
│   ├── resource.rs           (ResourceAccessor trait)
│   ├── credential.rs         (CredentialAccessor trait)
│   └── refresh.rs            (RefreshCoordinator trait, RefreshToken)
├── guard.rs                  (new: Guard, TypedGuard, debug helpers)
├── dependencies.rs           (new: Dependencies, Requirements, DeclaresDependencies)
├── scope.rs                  (new: ScopeLevel, Scope, Principal)
├── obs.rs                    (new: TraceId, SpanId types for spec 18 correlation)
└── auth.rs                   (existing: AuthScheme trait, AuthPattern)
```

Total new code: ~2–3K lines of foundation types and traits, no business
logic. Small compared to the ~80K lines of business code that will depend
on it.

## Appendix B — Example action after full migration

```rust
// User action code after all migrations complete

use nebula_action::prelude::*;
use nebula_credential::credentials::{GithubToken, SlackBotToken};
use nebula_resource::resources::{HttpResource, PostgresResource};
use nebula_schema::{Field, Schema};

#[derive(Action, Input, Output)]
#[action(key = "github.sync_to_slack", category = "integration")]
#[uses_credentials([GithubToken, SlackBotToken])]
#[uses_resources([HttpResource, PostgresResource(purpose = "sync state")])]
pub struct SyncGithubToSlack {
    #[field(label = "GitHub Repo", required)]
    pub repo: String,

    #[field(label = "Slack Channel", required)]
    pub channel: String,

    #[field(label = "Poll Interval (min)", min = 1, max = 60, default = 5)]
    pub poll_interval_min: u32,
}

#[async_trait]
impl StatelessAction for SyncGithubToSlack {
    type Input = Self;
    type Output = SyncResult;

    async fn execute<C: ActionContext>(
        &self,
        input: Self::Input,
        ctx: &C,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Typed resource + credential access
        let github = ctx.credential::<GithubToken>().await?;
        let slack = ctx.credential::<SlackBotToken>().await?;
        let http = ctx.resource::<HttpResource>().await?;
        let db = ctx.resource::<PostgresResource>().await?;

        // Read identity from context
        let execution_id = ctx.execution_id().expect("action has execution_id");
        let workspace_id = ctx.workspace_id().expect("workspace-scoped action");

        ctx.logger().log_with_fields(
            LogLevel::Info,
            "starting github→slack sync",
            &[
                ("repo", &input.repo as &dyn std::fmt::Debug),
                ("execution_id", &execution_id as &dyn std::fmt::Debug),
            ],
        );

        // Fetch from GitHub using token (Deref access)
        let prs = fetch_prs(&*http, &*github, &input.repo).await?;

        // Post to Slack using token
        post_to_slack(&*http, &*slack, &input.channel, &prs).await?;

        // Record sync state in DB
        db.execute(
            "INSERT INTO sync_log (workspace_id, execution_id, repo, channel, count) VALUES ($1, $2, $3, $4, $5)",
            &workspace_id,
            &execution_id,
            &input.repo,
            &input.channel,
            &(prs.len() as i64),
        ).await?;

        // Emit metric
        ctx.metrics().counter(
            "github_sync_total",
            1,
            &[("workspace", workspace_id.as_str()), ("repo", &input.repo)],
        );

        // Guards auto-released on scope exit:
        //   - github / slack → zeroized
        //   - http / db → returned to pools
        Ok(ActionResult::Success(SyncResult { synced: prs.len() }))
    }
}
```

## Changelog

- **2026-04-15** — initial draft. Defines cross-crate foundation for Context,
  Guard, Dependencies, Scope. Consolidates Q1–Q4 decisions from multi-round
  expert Q&A session. Replaces scattered `Ctx`/`ActionContext`/`CredentialContext`/
  `ActionDependencies`/`ScopeLevel` in favor of unified types living in
  `nebula-core`. Introduces capability trait pattern (`HasResources`,
  `HasCredentials`, ...) with umbrella traits for `ActionContext`/`TriggerContext`
  via blanket impls. Renames `ResourceHandle` → `ResourceGuard`. Adds
  multi-credential support, resource-to-resource dependencies, credential-to-
  resource dependencies (prevents credential-to-credential recursion).
  Introduces `uses_credential` / `uses_credentials` / `uses_resource` /
  `uses_resources` derive attribute syntax. 5 scope level variants aligned
  with spec 02 tenancy hierarchy. 7-PR migration sequence.
