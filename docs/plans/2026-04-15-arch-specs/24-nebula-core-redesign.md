# 24 — `nebula-core` redesign: cleanup + spec 23 integration

> **Status:** DRAFT
> **Authority:** subordinate to `docs/PRODUCT_CANON.md`. Canon wins on conflict.
> **Parent:** [`./README.md`](./README.md)
> **Scope:** Complete redesign of `nebula-core` crate — remove dead code, add
> spec 23 modules (Context, Guard, Dependencies, Scope), migrate IDs to
> prefixed ULID via `domain-key` v0.5, integrate spec 08 cancellation
> primitives, establish naming conventions.
> **Depends on:** 06 (IDs), 08 (cancellation), 19 (error taxonomy), 23 (cross-crate foundation)
> **Consumers:** every crate in the workspace
> **Supersedes:** current `nebula-core` layout (16 files → 15 entries, ~40 public types)

## 1. Problem

`nebula-core` has accumulated dead code, inconsistent naming, and missing
vocabulary types. Of 16 current source files:

- **6 files are dead** — zero imports outside core (`constants.rs`, `types.rs`,
  `traits.rs`, `secret_string.rs`, `serde_secret.rs`, `option_serde_secret.rs`)
- **3 types need rename/removal** — `ProjectId` → `WorkspaceId`, `TenantId`/
  `RoleId`/`OwnerId` have zero consumers
- **IDs use UUID v4** — spec 06 requires prefixed ULID (Stripe-style)
- **`NodeId` is UUID-backed** — should be `NodeKey` (authored string, `Key<NodeDomain>`)
- **`ScopeLevel` has 6 variants** including `Action` — spec 23 defines 5 variants
- **Spec 23 modules missing** — Context, Guard, Dependencies, Scope redesign
  not yet implemented
- **`SecretString` is a DRY violation** — reimplements `secrecy` crate, 93% used
  by `nebula-credential` only
- **`CoreError` has 15 variants** — most are auth/resource concerns that don't
  belong in core vocabulary crate

### 1.1 Inclusion criteria (established in Q&A)

**What belongs in `nebula-core`:** shared contracts and vocabulary types — trait
definitions, ID newtypes, domain keys, error types — without which cross-crate
composition is impossible.

**What does NOT belong:** entity models (`User`, `Organization`), RBAC logic
(`Permission`, `OrgRole`), settings structs, domain-specific constants, business
logic. These belong in their domain crates even if referenced by multiple consumers.

**Rationale:** strict contracts boundary (axum-core model) prevents god-crate
compilation cascade and keeps SRP intact. Specs 02-04 proposed entity structs
for core; those placements are overridden by this spec after SRP audit.

## 2. Decision

Complete redesign of `nebula-core` with:

1. **Delete** 6 dead files + 3 orphan ID types
2. **Add** spec 23 modules (context, accessor, guard, dependencies, scope)
3. **Add** spec 08 cancellation primitives (lifecycle, shutdown)
4. **Migrate** IDs from UUID to prefixed ULID via `domain-key` v0.5
5. **Rename** `NodeId` → `NodeKey` (authored string, `Key<NodeDomain>`)
6. **Replace** `SecretString` with `secrecy` crate (wrapper in `nebula-credential`)
7. **Shrink** `CoreError` to 5 variants (only real core operations)
8. **Move** `InterfaceVersion` to `nebula-action`, `Version` → `semver` crate
9. **Establish** formal naming convention for the workspace

## 3. Naming convention (new — workspace-wide)

This convention applies to all Nebula crates, not just `nebula-core`.

### 3.1 Identifier naming

| Suffix | Semantics | Backing type | Examples |
|---|---|---|---|
| `FooKey` | Author-defined string identifier | `domain_key::Key<D>` | `ActionKey`, `NodeKey`, `ResourceKey`, `CredentialKey`, `PluginKey`, `ParameterKey` |
| `FooId` | System-generated unique identifier | `domain_key::Ulid<D>` | `ExecutionId`, `WorkflowId`, `OrgId`, `WorkspaceId`, `CredentialId`, `AttemptId` |

**Rule:** seeing `*Key` means a validated string the developer/user wrote.
Seeing `*Id` means a prefixed ULID the system generated. No exceptions.

### 3.2 Trait / struct naming

| Pattern | Semantics | Location | Examples |
|---|---|---|---|
| `HasFoo` | Capability trait (provides access to Foo) | `nebula-core` | `HasResources`, `HasCredentials`, `HasLogger`, `HasMetrics`, `HasEventBus` |
| `FooAccessor` | Dyn-safe service trait (async operations) | `nebula-core` | `ResourceAccessor`, `CredentialAccessor` |
| `FooGuard<T>` | RAII wrapper with Drop cleanup | Domain crate | `CredentialGuard<S: Zeroize>`, `ResourceGuard<R: Resource>` |
| `FooContext` | Internal domain context struct | Domain crate | `CredentialContext`, `ResourceContext` |
| `FooRuntimeContext` | Concrete engine implementation | `nebula-engine` | `ActionRuntimeContext`, `TriggerRuntimeContext` |
| `DeclaresFoo` | Declaration trait | `nebula-core` | `DeclaresDependencies` |
| `FooRequirement` | Dependency declaration struct | `nebula-core` | `CredentialRequirement`, `ResourceRequirement` |
| `FooLike` | Marker trait for requirement type bounds | `nebula-core` | `CredentialLike`, `ResourceLike` |

### 3.3 Forbidden patterns

| Pattern | Replacement | Reason |
|---|---|---|
| `*Handle` for RAII wrappers | `*Guard` | Unified naming (spec 23) |
| `*Ctx` abbreviation | Full `*Context` | Readability |
| Stringly-typed IDs | `FooKey` or `FooId` newtype | Type safety |
| `Option<Box<dyn AnyFoo>>` single-item | `Vec<FooRequirement>` | Multi-item support (spec 23) |
| 5-method dependency trait | `DeclaresDependencies` (1 method) | Unified model (spec 23) |

## 4. Target module structure

```
nebula-core/src/
├── lib.rs                  (pub use re-exports + prelude)
│
├── id/
│   ├── mod.rs              (PrefixedId docs, IdParseError re-export)
│   └── types.rs            (~16 Ulid<D> newtypes via define_ulid!)
│
├── keys.rs                 (ActionKey, PluginKey, CredentialKey, ResourceKey,
│                            ParameterKey, NodeKey + compile-time macros)
│
├── context/
│   ├── mod.rs              (Context trait, BaseContext struct, BaseContextBuilder)
│   └── capability.rs       (HasResources, HasCredentials, HasLogger,
│                            HasMetrics, HasEventBus)
│
├── accessor.rs             (ResourceAccessor, CredentialAccessor, Logger,
│                            MetricsEmitter, EventEmitter, Clock, SystemClock,
│                            RefreshCoordinator, RefreshToken)
│
├── guard.rs                (Guard trait, TypedGuard trait)
│
├── dependencies.rs         (Dependencies, DeclaresDependencies,
│                            CredentialRequirement, ResourceRequirement,
│                            CredentialLike, ResourceLike)
│
├── scope.rs                (ScopeLevel 5 variants, Scope struct 9 fields,
│                            Principal enum, ScopeResolver trait)
│
├── lifecycle.rs            (LayerLifecycle, ShutdownOutcome — spec 08)
│
├── auth.rs                 (AuthScheme trait, AuthPattern enum — merged)
│
├── event.rs                (CredentialEvent)
│
├── obs.rs                  (TraceId, SpanId type aliases)
│
├── error.rs                (CoreError 5 variants)
│
└── serde_helpers.rs        (duration_opt_ms)
```

15 entries (13 files + 2 directories). No file exceeds ~250 lines.
Subdirectories only where content naturally splits (IDs: infra + types;
Context: base + capabilities).

## 5. Data model

### 5.1 IDs (`id/`)

Migration from `domain_key::Uuid<D>` (UUID v4) to `domain_key::Ulid<D>`
(prefixed ULID). Requires `domain-key` v0.5 with new `ulid` feature.

```rust
// id/types.rs
use domain_key::define_ulid;

// System-generated identifiers (FooId convention)
define_ulid!(pub OrgIdDomain        => OrgId,              prefix = "org");
define_ulid!(pub WorkspaceIdDomain  => WorkspaceId,        prefix = "ws");
define_ulid!(pub WorkflowIdDomain   => WorkflowId,         prefix = "wf");
define_ulid!(pub WorkflowVersionIdDomain => WorkflowVersionId, prefix = "wfv");
define_ulid!(pub ExecutionIdDomain  => ExecutionId,         prefix = "exe");
define_ulid!(pub AttemptIdDomain    => AttemptId,           prefix = "att");
define_ulid!(pub InstanceIdDomain   => InstanceId,          prefix = "nbl");
define_ulid!(pub TriggerIdDomain    => TriggerId,           prefix = "trg");
define_ulid!(pub TriggerEventIdDomain => TriggerEventId,    prefix = "evt");
define_ulid!(pub UserIdDomain       => UserId,              prefix = "usr");
define_ulid!(pub ServiceAccountIdDomain => ServiceAccountId, prefix = "svc");
define_ulid!(pub CredentialIdDomain => CredentialId,        prefix = "cred");
define_ulid!(pub ResourceIdDomain   => ResourceId,          prefix = "res");
define_ulid!(pub SessionIdDomain    => SessionId,           prefix = "sess");
```

Each type: 16 bytes, `Copy`, `Eq`/`Ord`/`Hash`, `Display` (prefixed
Crockford Base32), `FromStr` (validates prefix), `Serialize`/`Deserialize`
(as prefixed string), `created_at() -> DateTime<Utc>`.

### 5.2 Keys (`keys.rs`)

Unchanged from current except addition of `NodeKey`:

```rust
use domain_key::{define_domain, key_type};

// Author-defined string identifiers (FooKey convention)
define_domain!(pub ParameterDomain, "parameter");  key_type!(pub ParameterKey, ParameterDomain);
define_domain!(pub CredentialDomain, "credential"); key_type!(pub CredentialKey, CredentialDomain);
define_domain!(pub ActionDomain, "action");         key_type!(pub ActionKey, ActionDomain);
define_domain!(pub ResourceDomain, "resource");     key_type!(pub ResourceKey, ResourceDomain);
define_domain!(pub PluginDomain, "plugin");         key_type!(pub PluginKey, PluginDomain);
define_domain!(pub NodeDomain, "node");             key_type!(pub NodeKey, NodeDomain); // NEW
```

`NodeKey` replaces `NodeId`. It is a validated string slug (lowercase
alphanumeric + underscores/hyphens), unique within a workflow definition.
Generated by frontend from `ActionKey` + dedup suffix; author may rename.

### 5.3 CoreError (`error.rs`)

Reduced from 15 variants to 5 — only errors that core operations actually produce:

```rust
use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum CoreError {
    /// Prefixed ULID failed to parse (wrong prefix, malformed ULID).
    #[error("invalid ID: expected prefix `{expected_prefix}_`, got `{raw}`")]
    InvalidId {
        raw: String,
        expected_prefix: &'static str,
    },

    /// Domain key failed validation.
    #[error("invalid key in domain `{domain}`: `{raw}`")]
    InvalidKey {
        raw: String,
        domain: &'static str,
    },

    /// Scope containment violation (e.g. execution-scoped resource accessing
    /// workflow-scoped credential in a different workflow).
    #[error("scope violation: {source} cannot access {target}")]
    ScopeViolation {
        source: String,
        target: String,
    },

    /// Dependency cycle detected during registration (Tarjan SCC).
    #[error("dependency cycle: {}", path.join(" -> "))]
    DependencyCycle {
        path: Vec<&'static str>,
    },

    /// Required dependency not registered.
    #[error("missing dependency: `{required_by}` requires `{name}`")]
    DependencyMissing {
        name: &'static str,
        required_by: &'static str,
    },
}
```

Implements `nebula_error::Classify`. Previous variants for auth, resource,
timeout, serialization — removed; those belong in domain crate error types.

### 5.4 ScopeLevel + Scope + Principal (`scope.rs`)

Redesigned per spec 23. Key changes from current:
- 6 variants → 5 (remove `Action`)
- `Project(ProjectId)` → `Workspace(WorkspaceId)`
- All variants use typed IDs instead of strings
- Add `Scope` struct (9 optional ID fields)
- Add `Principal` enum

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScopeLevel {
    Global,
    Organization(OrgId),
    Workspace(WorkspaceId),
    Workflow(WorkflowId),
    Execution(ExecutionId),
    // Action variant REMOVED — node-level scope is expressed via
    // Scope { execution_id, node_key, attempt_id } fields, not a ScopeLevel variant
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scope {
    pub org_id: Option<OrgId>,
    pub workspace_id: Option<WorkspaceId>,
    pub workflow_id: Option<WorkflowId>,
    pub workflow_version_id: Option<WorkflowVersionId>,
    pub execution_id: Option<ExecutionId>,
    pub node_key: Option<NodeKey>,
    pub attempt_id: Option<AttemptId>,
    pub trigger_id: Option<TriggerId>,
    pub instance_id: Option<InstanceId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Principal {
    User(UserId),
    ServiceAccount(ServiceAccountId),
    Workflow { workflow_id: WorkflowId, trigger_id: Option<TriggerId> },
    System,
}
```

`ScopeResolver` trait retained (redesigned to use typed IDs):

```rust
pub trait ScopeResolver {
    fn workflow_for_execution(&self, exec_id: &ExecutionId) -> Option<WorkflowId>;
    fn workspace_for_workflow(&self, workflow_id: &WorkflowId) -> Option<WorkspaceId>;
    fn org_for_workspace(&self, workspace_id: &WorkspaceId) -> Option<OrgId>;
}
```

`ScopedId` and `ChildScopeType` — **deleted** (zero external consumers).

### 5.5 Lifecycle (`lifecycle.rs`)

From spec 08, integrated into core module structure:

```rust
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// One layer in the cancellation hierarchy (spec 08).
///
/// Four layers: Process → Engine → Execution → Node.
/// Each layer's grace period is strictly shorter than its parent's.
pub struct LayerLifecycle {
    pub token: CancellationToken,
    pub tasks: TaskTracker,
}

impl LayerLifecycle {
    pub fn root() -> Self { /* CancellationToken::new() + TaskTracker::new() */ }
    pub fn child(&self) -> Self { /* parent.child_token() + new tracker */ }
    pub async fn shutdown(&self, grace: Duration) -> ShutdownOutcome { /* spec 08 */ }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownOutcome {
    Graceful,
    GraceExceeded,
}
```

### 5.6 Context, Accessor, Guard, Dependencies

Defined in spec 23 — no changes from that spec. This spec controls
**where they live in the module tree**, not their API design:

| Spec 23 concept | Module in this spec |
|---|---|
| `Context` trait + `BaseContext` + `BaseContextBuilder` | `context/mod.rs` |
| `HasResources` / `HasCredentials` / `HasLogger` / `HasMetrics` / `HasEventBus` | `context/capability.rs` |
| `ResourceAccessor` / `CredentialAccessor` / `Logger` / `MetricsEmitter` / `EventEmitter` | `accessor.rs` |
| `Clock` trait + `SystemClock` | `accessor.rs` |
| `RefreshCoordinator` trait + `RefreshToken` | `accessor.rs` |
| `Guard` trait + `TypedGuard` trait | `guard.rs` |
| `Dependencies` + `DeclaresDependencies` + `*Requirement` + `*Like` | `dependencies.rs` |

### 5.7 Auth (`auth.rs`)

Merge `auth.rs` + `auth_pattern.rs` into single file. No API changes —
`AuthScheme` trait and `AuthPattern` enum stay as-is.

### 5.8 Observability (`obs.rs`)

```rust
/// Trace identifier for distributed tracing (spec 18).
/// Newtype over u128 matching W3C Trace Context trace-id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TraceId(pub u128);

/// Span identifier for distributed tracing (spec 18).
/// Newtype over u64 matching W3C Trace Context parent-id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpanId(pub u64);
```

## 6. Cargo.toml (target)

```toml
[package]
name = "nebula-core"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
description = "Core vocabulary types and trait contracts for the Nebula workflow engine"

[dependencies]
chrono = { workspace = true, features = ["serde"] }
domain-key = { workspace = true, features = ["serde", "ulid"] }
nebula-error = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio-util = { workspace = true }

# REMOVED: zeroize (moved with SecretString to nebula-credential)
# REMOVED: postcard (moved to nebula-runtime if needed)
```

## 7. Deletions

### 7.1 Files deleted entirely

| File | Reason | External imports |
|---|---|---|
| `constants.rs` | Kitchen-sink defaults, 0 consumers | 0 |
| `types.rs` | Dead types: Status, Priority, OperationResult, OperationContext, ProjectType, RoleScope, Version, utils | 0 (except Version/InterfaceVersion — relocated) |
| `traits.rs` | Dead: HasContext, Scoped — replaced by spec 23 Context system | 0 |
| `secret_string.rs` | DRY violation — replaced by `secrecy` crate | 0 outside credential |
| `serde_secret.rs` | Moves with SecretString to `nebula-credential` | 0 outside credential |
| `option_serde_secret.rs` | Moves with SecretString to `nebula-credential` | 0 outside credential |

### 7.2 Types relocated

| Type | From | To | Reason |
|---|---|---|---|
| `SecretString` | `nebula-core` | `secrecy` crate + thin wrapper in `nebula-credential` | DRY (93% credential consumer) |
| `InterfaceVersion` | `nebula-core::types` | `nebula-action::metadata` | Only action/plugin consumers |
| `Version` | `nebula-core::types` | `semver::Version` (external crate) | DRY — standard Rust crate |

### 7.3 Types deleted (zero consumers)

`TenantId`, `RoleId`, `OwnerId`, `ProjectId` (→ `WorkspaceId`),
`Status`, `Priority`, `OperationResult<T>`, `OperationContext`,
`ProjectType`, `RoleScope`, `ScopedId`, `ChildScopeType`,
`HasContext`, `Scoped`, `types::utils::*`.

### 7.4 Types renamed

| Old | New | Reason |
|---|---|---|
| `NodeId` (UUID) | `NodeKey` (`Key<NodeDomain>`) | Authored string, not generated ID. Matches `*Key` convention. |
| `ProjectId` | `WorkspaceId` | Spec 02 naming |
| `ScopeLevel::Project(ProjectId)` | `ScopeLevel::Workspace(WorkspaceId)` | Spec 02 naming |
| `ScopeLevel::Action(ExecutionId, NodeId)` | — (deleted) | Node scope via `Scope` struct fields |

## 8. Edge cases

### 8.1 NodeKey validation in workflow definitions

`NodeKey` must be unique within a single `WorkflowDefinition.nodes` list.
Validation at `workflow::validate_workflow()` — not at type level.
`domain_key::Key<NodeDomain>` validates format (slug); uniqueness is
workflow-level business logic.

### 8.2 NodeKey generation in visual editor

Frontend generates `NodeKey` from `ActionKey` + dedup suffix:
- First `http_request` node → `NodeKey("http_request")`
- Second → `NodeKey("http_request_2")`
- User may rename to `NodeKey("fetch_users")`

Backend only validates format + uniqueness within workflow.

### 8.3 ULID monotonicity

`domain-key` v0.5 `ulid-monotonic` feature provides monotonic generation
within a single process/thread for hot append paths (e.g. `execution_nodes`
inserts). Without the feature, standard ULID generation (ms-precision
timestamp + random) is used.

### 8.4 Migration of existing UUID data

Existing persisted UUIDs (in SQLite/Postgres) need migration:
- Storage layer reads both UUID and ULID formats during transition
- New writes use ULID
- Background migration job converts old UUIDs
- `domain-key` v0.5 provides `Ulid::from_uuid_bytes()` for zero-downtime migration

### 8.5 `secrecy` crate integration

`nebula-credential` changes:
1. Add `secrecy` dependency
2. Create `RedactedSecret<S: Zeroize>` newtype wrapping `secrecy::Secret<S>`
   with redacted-by-default `Serialize`
3. Update all scheme types to use `secrecy::SecretString` or `RedactedSecret`
4. Move `serde_secret` / `option_serde_secret` helpers into credential crate

## 9. Configuration surface

No new configuration. `nebula-core` is a library crate — all configuration
comes from consumer crates (engine config for grace periods, etc.).

`LayerLifecycle` grace periods are set by callers:
- Process: `NEBULA_SHUTDOWN_TIMEOUT` env (default 60s)
- Engine: engine config (default 45s)
- Execution: per-execution (default 30s)
- Node: `ActionMetadata::cancel_grace` (default 30s, max 5min)

## 10. Testing criteria

### 10.1 ID system

- All 14 `*Id` types: `new()` produces non-nil ULID, `Display` outputs
  correct prefix, `FromStr` validates prefix, serde roundtrip, `Copy`
  semantics, `created_at()` returns reasonable timestamp
- `NodeKey`: validates slug format, rejects invalid chars, `Display`/`FromStr`
  roundtrip
- Compile-time type safety: `ExecutionId` cannot be passed where `WorkflowId`
  expected (compile-fail test)

### 10.2 Scope

- `ScopeLevel::is_contained_in()` hierarchy: 5 levels, strict ordering
- `ScopeLevel::is_contained_in_strict()` with mock `ScopeResolver`
- `Scope` struct: all optional fields, serde roundtrip
- `Principal` enum: all variants, serde roundtrip

### 10.3 CoreError

- All 5 variants produce correct `error_code()` via `Classify`
- `is_retryable()` returns false for all (core errors are not transient)
- `Display` messages are human-readable

### 10.4 Lifecycle

- `LayerLifecycle::root()` creates independent token + tracker
- `LayerLifecycle::child()` token is cancelled when parent cancels
- `shutdown()` returns `Graceful` when tasks complete within grace
- `shutdown()` returns `GraceExceeded` when tasks exceed grace

### 10.5 Context / Accessor / Guard / Dependencies

Per spec 23 testing criteria. Verified by this spec's module structure.

## 11. Performance targets

- ID generation: < 100ns per ULID (monotonic mode)
- ID parsing: < 200ns per prefixed string → typed ID
- Scope containment check: < 50ns (no allocation)
- CoreError Display: no allocation for static messages
- `LayerLifecycle::child()`: < 1µs (token clone + tracker init)

## 12. Module boundaries

| Module | Crate | Owns |
|---|---|---|
| `id/` | `nebula-core` | All `*Id` newtypes, `IdParseError` |
| `keys` | `nebula-core` | All `*Key` newtypes including `NodeKey`, compile-time macros |
| `context/` | `nebula-core` | `Context` trait, `BaseContext`, capability traits |
| `accessor` | `nebula-core` | Trait definitions only — impls in domain crates |
| `guard` | `nebula-core` | `Guard` + `TypedGuard` traits only — impls in domain crates |
| `dependencies` | `nebula-core` | Dependency declaration types and `DeclaresDependencies` trait |
| `scope` | `nebula-core` | `ScopeLevel`, `Scope`, `Principal`, `ScopeResolver` |
| `lifecycle` | `nebula-core` | `LayerLifecycle`, `ShutdownOutcome` |
| `auth` | `nebula-core` | `AuthScheme` trait, `AuthPattern` enum |
| `event` | `nebula-core` | `CredentialEvent` |
| `obs` | `nebula-core` | `TraceId`, `SpanId` |
| `error` | `nebula-core` | `CoreError` (5 variants) |
| `serde_helpers` | `nebula-core` | `duration_opt_ms` |

## 13. Migration path

### PR 0: domain-key v0.5

**Separate crate, separate repo.** Add `Ulid<D>` + `UlidDomain` trait +
`define_ulid!` macro + `ulid` / `ulid-monotonic` features. Publish to
crates.io. No nebula changes yet.

### PR 1: nebula-core cleanup (deletions only)

1. Delete `constants.rs`, `types.rs`, `traits.rs`
2. Delete `secret_string.rs`, `serde_secret.rs`, `option_serde_secret.rs`
3. Remove `zeroize` and `postcard` from `Cargo.toml`
4. Fix compile errors in consumers:
   - `Version` → `semver::Version` (add `semver` dep to nebula-workflow, etc.)
   - `InterfaceVersion` → define in `nebula-action::metadata` (already re-exported there)
   - `SecretString` → `secrecy::SecretString` in nebula-credential + thin wrapper
5. Delete `TenantId`, `RoleId`, `OwnerId`, `ScopedId`, `ChildScopeType`
6. All tests green, no new features

### PR 2: ID migration

1. Update `domain-key` to v0.5 in workspace `Cargo.toml`
2. Replace `define_uuid!` with `define_ulid!` in `id/types.rs`
3. Add `NodeKey` to `keys.rs`
4. Rename `ProjectId` → `WorkspaceId` everywhere
5. Rename `NodeId` → `NodeKey` everywhere (including `nebula-workflow`, `nebula-action`)
6. Update `ScopeLevel`: remove `Action` variant, rename `Project` → `Workspace`
7. Add `Scope` struct, `Principal` enum
8. Rewrite `ScopeResolver` with typed IDs
9. Storage migration: UUID → ULID column format (dual-read during transition)

### PR 3: spec 23 modules

1. Add `context/` (Context trait, BaseContext, capability traits)
2. Add `accessor.rs` (all accessor trait definitions)
3. Add `guard.rs` (Guard, TypedGuard)
4. Add `dependencies.rs` (Dependencies, DeclaresDependencies, Requirements)
5. Add `lifecycle.rs` (LayerLifecycle, ShutdownOutcome from spec 08)
6. Add `obs.rs` (TraceId, SpanId)
7. Add `tokio-util` dependency
8. Rewrite `error.rs` (CoreError → 5 variants)
9. Merge `auth.rs` + `auth_pattern.rs`
10. Rewrite `lib.rs` with new module structure and prelude

### PR 4: consumer migration

1. `nebula-resource`: `ResourceHandle` → `ResourceGuard`, `Ctx` → `ResourceContext`,
   local `ScopeLevel` → `nebula-core::ScopeLevel`
2. `nebula-credential`: `SecretString` → `secrecy`, add `RedactedSecret` wrapper,
   adopt `CredentialContext` from spec 23
3. `nebula-action`: `ActionContext` struct → trait, `ActionDependencies` → `DeclaresDependencies`
4. `nebula-engine`: add `ActionRuntimeContext`, `TriggerRuntimeContext`

### PR 5: testing + canon

1. `nebula-testing`: add `TestContext` implementing all capability traits
2. Update PRODUCT_CANON.md: §11.12 (cross-crate foundation), §12.13
   (capability trait DI pattern), §12.14 (credential/resource dep rules)
3. Update COMPACT.md with naming convention

## 14. Open questions

### 14.1 `serde_helpers` expansion

Currently only `duration_opt_ms`. If more shared serde helpers emerge
(e.g. for ULID serialization), this module grows naturally. No action needed now.

### 14.2 `LogLevel` placement

Spec 23 proposes `LogLevel` enum in core. Alternative: re-export from
`tracing::Level`. Deferred — decide when implementing `Logger` trait.

### 14.3 `Clock` trait async

Current spec 23 design has `fn now() -> DateTime<Utc>` (sync). If
distributed clock or NTP-aware clock is needed later, may need async
variant. Deferred — sync is correct for v1.

### 14.4 `domain-key` backward compatibility

Existing code uses `domain_key::Uuid<D>`. After v0.5, `Uuid<D>` still
exists (feature-gated). Migration is `Uuid<D>` → `Ulid<D>` per PR 2.
No breaking change in domain-key itself — additive features only.
