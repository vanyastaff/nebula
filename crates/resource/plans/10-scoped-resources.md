# 10 — Scoped Resources: Per-Execution Lifecycle

---

## Overview

Scoped resources are created for a single execution (or a single branch of a DAG)
and destroyed when that scope completes. Downstream actions use the same
`ctx.resource::<R>()` API — they don't know whether the resource is scoped or global.

**Global** (Manager registration): shared across workflows, long-lived.
Example: shared Postgres pool for all executions.

**Scoped** (ResourceAction): per-execution or per-branch lifecycle.
Examples: temporary test database, isolated transaction pool, per-tenant connection
with dynamic credentials, ephemeral sandbox environment.

Scoped resources are one of Nebula's most powerful features — they enable
per-execution isolation without any changes to action code.

---

## ResourceAction recap (from 06-action-integration.md)

```rust
pub trait ResourceAction: Action {
    type Resource: Resource;

    /// Provide config for the scoped resource.
    async fn configure(&self, ctx: &ActionContext)
        -> Result<<Self::Resource as Resource>::Config>;

    /// Topology for the scoped resource. Default: Resident.
    fn topology(&self) -> ScopedTopology { ScopedTopology::Resident }

    /// Custom cleanup after all downstream nodes complete. Default: noop.
    async fn cleanup(&self, ctx: &ActionContext) -> Result<()> { Ok(()) }
}
```

---

## Resolution precedence

When an action calls `ctx.resource::<R>()`, the framework resolves in this order:

```
1. Scoped — is there a ResourceAction for R in a parent graph branch?
   → yes → acquire from scoped runtime (closest ancestor wins)
2. Global — is R registered in the Manager with matching resource_id?
   → yes → acquire from global Manager
3. Not found → ActionError::ResourceNotFound
```

**Scoped always wins over global.** This is intentional: it enables transparent
per-execution overrides without changing action code.

---

## Nested branch shadowing

In a DAG with multiple branches, each branch can have its own ResourceAction
for the same resource type. Shadowing follows **closest ancestor** semantics.

```
[Global Postgres Pool]  ← registered in Manager

[Execution starts]
  │
  ├── [TenantPoolAction: ResourceAction<Postgres>]     ← scope A
  │       config: { search_path: "tenant_a" }
  │       │
  │       ├── [QueryUsers]     ← ctx.resource::<Postgres>() → scope A (tenant_a)
  │       │
  │       └── [IsolatedTestAction: ResourceAction<Postgres>]   ← scope B (nested)
  │               config: { search_path: "test_schema" }
  │               │
  │               └── [InsertTestData]  ← ctx.resource::<Postgres>() → scope B (test_schema)
  │
  └── [QueryOrders]    ← ctx.resource::<Postgres>() → global pool (no scoped ancestor)
```

**Rules:**
- `InsertTestData` gets scope B (closest ancestor is `IsolatedTestAction`).
- `QueryUsers` gets scope A (closest ancestor is `TenantPoolAction`).
- `QueryOrders` gets global pool (no scoped Postgres in its branch).
- Scope B does NOT affect scope A or the global pool.

**Implementation:** Scoped resources are stored in a per-branch `ScopedResourceMap`
that is passed down the DAG execution. Each `ResourceAction` adds an entry.
Lookup walks up the ancestor chain.

---

## Credential resolution for scoped resources

Scoped resources use the same `CredentialStore` as global resources, but the
**scope** passed to `resolve_erased()` differs:

- **Global resource:** scope = the scope at registration time (e.g., `Organization("acme")`).
- **Scoped resource:** scope = the execution scope (e.g., `Execution("run-42")`).

This means scoped resources can use different credentials than global ones,
enabling per-execution credential isolation:

```rust
impl ResourceAction for TenantDatabaseAction {
    type Resource = Postgres;

    async fn configure(&self, ctx: &ActionContext)
        -> Result<PgResourceConfig>
    {
        // Dynamic config based on execution context
        let tenant = ctx.ext::<TenantContext>()
            .ok_or(ActionError::missing_context("TenantContext"))?;
        Ok(PgResourceConfig {
            search_path: Some(tenant.schema.clone()),
            application_name: format!("nebula-{}", tenant.name),
            ..Default::default()
        })
    }
}

// Framework resolves credential at execution scope:
// credential_store.resolve::<DatabaseCredential>(&ScopeLevel::Execution(exec_id))
// This may return a different credential than the global pool's credential.
```

**Credential rotation:** Scoped resources are short-lived (per-execution).
Credential rotation events are irrelevant — the resource is destroyed when
the execution completes, long before any rotation period.

---

## Cancellation and cleanup ordering

When an execution (or branch) is cancelled:

```
1. CancellationToken fires for the branch
2. Running actions in the branch receive cancellation
3. Actions complete or abort (timeout)
4. ResourceAction::cleanup() called (in reverse order for nested scopes)
5. Framework calls Resource::shutdown() on scoped runtime
6. Framework calls Resource::destroy() on scoped runtime
7. Scoped runtime removed from ScopedResourceMap
```

**Key invariants:**
- `cleanup()` is called even on cancellation (like `finally` in try/catch).
- Nested scopes destroy inner-to-outer (scope B destroyed before scope A).
- Global resources are NOT affected by scoped resource cancellation.
- If `cleanup()` or `destroy()` fails, the error is logged but does not
  block the cancellation flow.

**Timeout:** Scoped resource cleanup has a configurable timeout (default: 30s).
If cleanup exceeds this, the runtime is dropped without graceful shutdown.

---

## Scope conflicts

**Same resource type, same branch level:** Not allowed. The second `ResourceAction`
for the same `R` at the same branch level produces a compile-time or registration-time
error. Only one scoped resource of each type per branch level.

**Same resource type, different branches:** Allowed. Each branch has independent
scoped resources. Actions in branch A cannot see scoped resources from branch B.

**Scoped overrides global with different topology:** Allowed. A global `Postgres`
may be `Pool(max_size=20)`, while a scoped `Postgres` may be
`Pool(max_size=2)` or even `Resident`. The topology is determined by
`ResourceAction::topology()`.

---

## Use cases

### Temporary test database

> **NOTE:** `configure()` may call `ctx.resource::<R>()` to use global resources.
> These global resources must be registered in the Manager before the workflow starts.
> If not registered, `configure()` will return `ActionError::ResourceNotFound`.

```rust
impl ResourceAction for TestDatabaseAction {
    type Resource = Postgres;

    async fn configure(&self, ctx: &ActionContext) -> Result<PgResourceConfig> {
        // Create temp schema using the GLOBAL pool (must be registered beforehand)
        let db = ctx.resource::<Postgres>().await?; // use global pool
        let schema = format!("test_{}", ctx.execution_id());
        db.execute(&format!("CREATE SCHEMA {schema}"), &[]).await?;
        Ok(PgResourceConfig {
            search_path: Some(schema),
            ..Default::default()
        })
    }

    async fn cleanup(&self, ctx: &ActionContext) -> Result<()> {
        let db = ctx.resource::<Postgres>().await?; // global pool (scoped already destroyed)
        let schema = format!("test_{}", ctx.execution_id());
        db.execute(&format!("DROP SCHEMA {schema} CASCADE"), &[]).await?;
        Ok(())
    }
}
```

### Per-tenant isolated pool
```rust
impl ResourceAction for TenantPoolAction {
    type Resource = Postgres;

    fn topology(&self) -> ScopedTopology {
        ScopedTopology::Pool(pool::Config {
            min_size: 1, max_size: 5,
            ..Default::default()
        })
    }

    async fn configure(&self, ctx: &ActionContext) -> Result<PgResourceConfig> {
        let tenant = ctx.ext::<TenantContext>().unwrap();
        Ok(PgResourceConfig {
            search_path: Some(tenant.schema.clone()),
            ..Default::default()
        })
    }
}
```

### Ephemeral sandbox (Browser)
```rust
impl ResourceAction for SandboxBrowserAction {
    type Resource = HeadlessBrowser;

    fn topology(&self) -> ScopedTopology {
        ScopedTopology::Pool(pool::Config {
            min_size: 0, max_size: 3,
            ..Default::default()
        })
    }

    async fn configure(&self, ctx: &ActionContext) -> Result<BrowserConfig> {
        Ok(BrowserConfig {
            headless: true,
            user_data_dir: None, // fresh profile per execution
            ..Default::default()
        })
    }

    async fn cleanup(&self, _ctx: &ActionContext) -> Result<()> {
        // Framework destroys all browser pages automatically.
        // cleanup() can do extra audit logging if needed.
        Ok(())
    }
}
```

---

## Interaction with other features

| Feature | Scoped behavior |
|---------|----------------|
| **RecoveryGate** | Scoped resources do NOT participate in RecoveryGroups (too short-lived) |
| **AcquireResilience** | Scoped resources use default resilience (no custom config) |
| **Config reload** | Not applicable — scoped lifetime < reload interval |
| **Credential rotation** | Not applicable — scoped lifetime < rotation interval |
| **Metrics** | Scoped resources emit metrics with `scope=execution:{id}` label |
| **Health checks** | Scoped pools run health checks normally during their lifetime |
| **EventBus** | Scoped register/destroy events emitted for observability |
