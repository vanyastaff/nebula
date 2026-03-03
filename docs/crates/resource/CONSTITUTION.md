# nebula-resource Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Nebula is a workflow automation platform. When a "Query Postgres → Transform → Push to Slack"
workflow runs, the "Query Postgres" action needs a live database connection. Not a new
connection on every execution — a pooled, health-checked, scoped connection from a managed
pool. And that connection must use the right credential for the right tenant, and must
never be shared with a different tenant's workflow.

**nebula-resource is the connection management layer of the Nebula platform.**

It answers one question: *How does a workflow action get the external connection it needs —
already warmed, already authenticated, already health-checked — without knowing how pooling,
credentials, or tenant isolation work?*

```
workflow engine registers "postgres.main" resource at startup
    ↓
action execution calls ctx.acquire("postgres.main")
    ↓
resource manager checks scope (tenant-a workflow only)
    ↓
pool returns idle, health-verified connection (< 1ms)
    ↓
action runs SQL query, drops guard
    ↓
guard drop returns connection to pool, emits Released event
    ↓
background health checker validates pool state
    ↓
bad connection detected → quarantine → create fresh replacement
```

This is the resource contract. Acquire is cheap. Failure is explicit. Scope is enforced.

---

## User Stories

### Story 1 — Action Developer Gets a Database Connection (P1)

A developer writing a "Query Users" action needs a live Postgres connection. They should
not write pool logic, connection strings, credential lookups, or retry logic. They should
just get a connection and use it.

**Acceptance**:
```rust
#[derive(Action)]
#[resources([DatabaseResource])]
pub struct QueryUserAction;

impl ProcessAction for QueryUserAction {
    async fn execute(&self, input: Input, ctx: &ActionContext) -> Result<Output> {
        let db = ctx.acquire_typed(DatabaseResource, &ctx.resource_ctx()).await?;
        let users = db.query("SELECT * FROM users").await?;
        Ok(users)
    }
}
// Connection returned to pool automatically when db guard drops
```
No pool logic, no credential lookup, no retry in action code.

### Story 2 — Platform Operator Registers Resources at Startup (P1)

A platform operator configures a Nebula deployment with one Postgres pool (global),
one Redis cache (tenant-scoped), and one S3 client (per-workflow). All three should
be registered once, validated at boot, and available to all executions under the correct scope.

**Acceptance**:
- `manager.register(PostgresResource, config, PoolConfig::default())` → validated at startup
- `manager.register_scoped(RedisResource, config, pool_cfg, Scope::tenant("tenant-a"))` → tenant-isolated
- Invalid config (e.g., empty DSN) → startup fails fast with `Configuration` error
- Resources registered in dependency order via declared dependency graph

### Story 3 — Workflow Engine Handles a Degraded Resource Gracefully (P1)

A Postgres pool loses connectivity due to network flap. The engine should not propagate
panics or return garbage — it should detect the failure, quarantine the pool, and surface
a retryable error to the action so the workflow execution can be retried at engine level.

**Acceptance**:
- Health checker detects consecutive failures → quarantine pool
- `acquire()` during quarantine → `Unavailable { retryable: true }`
- Action caller applies retry policy from `resilience` crate
- Pool exits quarantine after health probe succeeds → resumes serving
- `Quarantined` and `QuarantineReleased` events emitted via `EventBus`

### Story 4 — Multi-Tenant Isolation is Never Bypassed (P1)

Tenant A's workflow must never receive a connection from Tenant B's pool. Even if
the resource id is the same, scope enforcement must prevent cross-tenant acquisition.

**Acceptance**:
- `Scope::workflow_in_tenant("wf-orders", "tenant-a")` registers resource for tenant-a only
- `acquire()` from `tenant-b` execution context → `ScopeViolation` error
- Scope violation logged with full context before error returned
- No configuration flag or "admin override" bypasses scope enforcement

### Story 5 — Platform API Observes Resource Fleet Status (P2)

An operator needs a live dashboard showing pool utilization, health, and quarantine
state for all registered resources. The API layer should read this without modifying
pool state.

**Acceptance**:
- `manager.list_status()` → `Vec<ResourceStatus>` with metadata, health, pool stats, quarantine
- `manager.event_bus().subscribe()` → `broadcast::Receiver<ResourceEvent>` for SSE streaming
- API layer is read-only — it never registers, deregisters, or drains resources
- `ResourceEvent` stream carries typed events: `HealthChanged`, `Acquired`, `Quarantined`, etc.

---

## Core Principles

### I. Acquire is the Hot Path — It Must be Cheap

**The acquire-use-release cycle MUST be the lowest-latency path in the system.
Pool lookup, scope validation, and idle-instance return must complete in microseconds.**

**Rationale**: A workflow with 50 sequential steps calls `acquire` 50 times per execution.
At 1000 concurrent executions, that is 50 000 acquires per second. Any per-acquire overhead
compounds directly into workflow latency. The pool must be fast first, observable second.

**Rules**:
- Idle instance acquire path MUST have no I/O, no lock contention on hot paths
- `acquire_typed` MUST compile to a monomorphic, inline-friendly call
- Observability hooks MUST be non-blocking and MUST NOT be on the critical path
- `PoolExhausted` MUST surface as a retryable error — never silently block forever

### II. Scope is the Security Boundary

**A resource registered under Scope A MUST NEVER be acquired by a caller in Scope B.
Scope enforcement is non-negotiable, deny-by-default, and carries no override path.**

**Rationale**: Multi-tenant workflow automation means tenant-a's Postgres pool and
tenant-b's Postgres pool may have the same resource id but completely different
credentials and data. A scope violation is a data breach, not an operational error.

**Rules**:
- MUST validate scope on every `acquire`, `acquire_typed`, and `list`
- `ScopeViolation` MUST log the full context (caller scope, resource scope, resource id) before returning `Err`
- Cache MUST be keyed by `(resource_id, scope_id)` — never serve cross-scope hits
- Scope containment check MUST be the first guard in acquire — before pool lookup
- MUST NOT provide "admin mode" or "bypass scope" API

### III. Health and Quarantine are First-Class

**A resource pool's health is as important as its availability. Health failures MUST be
detected proactively, not only when an action surfaces an error.**

**Rationale**: n8n-style platforms wait for an action to fail before detecting pool health.
Nebula detects health degradation in the background and quarantines pools before actions
start failing. This turns silent errors into explicit platform signals.

**Rules**:
- Background `HealthChecker` MUST run independently of acquire/release
- Health failure → quarantine MUST emit `Quarantined` event before stopping service
- Quarantine exit (recovery) MUST require successful health probe — not just time elapsed
- `HealthState` transitions MUST be logged and observable via `EventBus`
- Validity check on instance reuse MUST happen on every acquire — not just at creation

### IV. Lifecycle Ownership is Explicit

**The `Pool` owns the instance lifecycle: create → validate → acquire → use → release →
recycle or destroy. No caller can bypass this sequence.**

**Rationale**: If callers can hold connections past pool limits or return them without
validation, the pool invariants break. RAII guards enforce correct return. Pool tracks
every outstanding instance. Resource leaks are impossible by construction.

**Rules**:
- All acquired instances MUST be wrapped in a `ResourceGuard` with RAII drop semantics
- `Pool` MUST track in-flight count — `guard.drop()` MUST decrement atomically
- `Resource::recycle` MUST be called before returning to pool — not after
- `Resource::cleanup` MUST be called on destroy — not skipped for performance
- Lifecycle hooks (`on_acquire`, `on_release`, `on_create`, `on_destroy`) MUST be additive

### V. Observability is an Additive Layer

**Core acquire/release MUST work without hooks, metrics, or tracing.
Observability is feature-gated and never on the critical error path.**

**Rationale**: Observability failures (metrics endpoint down, hook panics) must never
degrade the resource pool's ability to serve connections. Hooks are fire-and-forget.
Events are broadcast — missing one event does not break the pool.

**Rules**:
- `metrics`, `tracing`, `credentials` features MUST compile out cleanly
- `EventBus::send` MUST be non-blocking — lagging subscribers dropped, not blocked
- Hook failures MUST be logged but MUST NOT cancel the acquire
- Exception: pre-acquire hook with explicit cancellation intent is allowed — but must be documented

---

## Production Vision

### The resource fleet at scale

In a production Nebula deployment running hundreds of tenants, the resource manager
is the connection broker for the entire fleet:

```
ManagerBuilder
    │
    ├── GlobalResources (Scope::Global)
    │       ├── s3.default: S3Client (shared read-only, no credentials)
    │       └── cache.shared: Redis (fleet-wide expression cache)
    │
    ├── TenantResources (Scope::Tenant("tenant-a"))
    │       ├── db.main: PgPool (tenant-a credentials, max 20 connections)
    │       └── slack.webhook: SlackClient (tenant-a token)
    │
    └── TenantResources (Scope::Tenant("tenant-b"))
            ├── db.main: PgPool (tenant-b credentials, different host)
            └── stripe.api: StripeClient (tenant-b key)
```

Each pool:
- Health-checked every 30s by background `HealthChecker`
- Quarantined automatically on N consecutive failures
- Metrics exported to Prometheus via `metrics` feature
- Events streamed to `nebula-api` SSE endpoint for operator dashboard

### From the archives: Typed Pool Registry

The archive `archive-ideas.md` describes a `TypedPool` abstraction:
```rust
pub struct ResourcePool {
    pools: HashMap<TypeId, Box<dyn TypedPool>>,
    metrics: PoolMetrics,
}

pub trait TypedPool: Send + Sync {
    async fn acquire(&self) -> Result<PooledResource<Self::Resource>, Error>;
    fn stats(&self) -> PoolStats;
    async fn health_check(&self) -> Result<HealthStatus, Error>;
}
```
Current implementation has `acquire_typed` which achieves the same end. The production
path is to ensure the typed path is zero-cost via monomorphization.

### From the archives: Unified Resource State Machine

The archive `legacy-PROPOSALS.md` proposes formalizing instance states:
```
Created → Ready → Borrowed → Recycling → Ready (or) → Quarantined → Destroyed
```
Exposing this state machine as `ResourceEvent` variants enables operator debugging
and correctness auditing without internal code changes.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|---------|-------|
| `Manager::list_status()` + `get_status()` | Critical | Required for API observability endpoint |
| Credential refresh hook (P-004) | High | Short-lived OAuth2 tokens expire mid-pool |
| Back-pressure policy profiles | Medium | FailFast/WaitWithTimeout/Adaptive per resource |
| Partial config reload (non-destructive) | Medium | Avoid full pool swap for timeout tuning |
| Typed key wrapper (`ResourceKey<T>`) | Low | Compile-time id safety, archive P-001 |

---

## Key Decisions

### D-001: String Resource IDs as Primary Registry Key

**Decision**: `Manager` indexes pools by `Resource::id()` string, not by `TypeId`.

**Rationale**: Multiple flavors of the same resource type (e.g., `db.main`, `db.replica`)
need distinct ids. String ids allow explicit config-level naming and dynamic lookup.

**Rejected**: `TypeId`-only registry — cannot distinguish multiple instances of same type.
Numeric ids — no ergonomic config story.

### D-002: Scope Containment is Deny-By-Default

**Decision**: Scope compatibility uses parent chain consistency with deny-by-default.
Missing parent scope in child context does not imply access.

**Rationale**: Hard multi-tenant isolation requires transitive safety. Ambiguous scope
(no parent info) must not grant access — it must fail closed.

**Rejected**: Opt-in scope enforcement — requires every caller to opt-in, creating gaps.

### D-003: Health Split into Validation and Monitoring

**Decision**: Fast-path validity check happens on every acquire (is this instance still usable?).
Background `HealthChecker` monitors liveness trends and triggers quarantine.

**Rationale**: Mixing liveness monitoring into the acquire hot path is expensive. Background
health checking catches degradation early without per-acquire overhead.

**Rejected**: Health check only on acquire failure — too late, pool is already serving bad connections.

### D-004: Observability as Additive Feature-Gated Layer

**Decision**: `metrics`, `tracing`, `credentials` features are optional. Core acquire/release
works without them.

**Rationale**: Embedded deployments, test environments, and lightweight edge workers should
not pay for Prometheus client dependencies or tracing overhead.

**Rejected**: Mandatory observability — breaks embedded use cases, increases binary size.

---

## Open Proposals

### P-001: Typed Resource Key (`ResourceKey<T>`)

**Problem**: String id mismatch between registration and acquire is a runtime bug.
`manager.acquire("db.mian", &ctx)` compiles fine but panics at runtime.

**Proposal**: Optional `ResourceKey<T>` wrapper that encodes type and id at compile time.
String ids remain for dynamic/config-driven paths.

**Impact**: Additive — string API unchanged. Typed path is new.

### P-002: Back-Pressure Policy Profiles

**Problem**: All resources currently use the same acquire behavior. High-value resources
(Stripe API) should fail fast under exhaustion. Low-value resources (internal cache)
should wait with timeout.

**Proposal**: `AcquirePolicy` enum: `FailFast`, `WaitWithTimeout(Duration)`, `Adaptive`.
Registered per resource. Engine can override per execution context.

**Impact**: New `PoolConfig` field. Existing resources default to current behavior (WaitWithTimeout).

### P-003: Credential Refresh Hook

**Problem**: OAuth2 access tokens expire in ~1 hour. A connection created with a fresh
token may still be in the pool 90 minutes later with an expired token. The pool is
unaware.

**Proposal**: Pre-acquire hook that calls `credential_provider.is_fresh(instance_credential)`.
Stale credentials → recycle instance with fresh credential before serving.

**Dependency**: Requires `nebula-credential` `Refreshable` trait. Feature-gated.

---

## Non-Negotiables

1. **Scope enforcement on every acquire** — no bypass, no admin mode, no exceptions
2. **`ScopeViolation` logs full context** — caller scope, resource scope, resource id
3. **Guard RAII is mandatory** — no raw `Pool::return()` in public API
4. **`EventBus::send` is non-blocking** — lagging subscriber dropped, pool never blocked
5. **Health check on instance reuse** — `Resource::is_valid()` called on every acquire
6. **`PoolExhausted` is retryable** — never silently block; always surface to caller

---

## Related Documents

- **[CONTRACT.md](CONTRACT.md)** — Stable API contract for resource and SDK authors (2026+, decade of AI): Rust 1.93 baseline, native async, key/metadata taxonomy, error model, and nebula_sdk as single facade.

---

## Governance

Amendments require:
- PATCH: wording — PR with explanation
- MINOR: new lifecycle hook, event variant, or proposal — review with note in DECISIONS.md
- MAJOR: changing scope containment semantics, acquire lifecycle contract, or removing
  RAII guard guarantees — full architecture review required

All PRs must verify:
- Scope enforcement not bypassed in any new acquire path
- Guard RAII invariant preserved — no raw returns
- EventBus path remains non-blocking
