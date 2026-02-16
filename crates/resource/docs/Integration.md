# Integration Guide

How `nebula-resource` connects to the engine and action layers.

## Architecture

```
crates/action        crates/engine          crates/resource
+-----------------+  +-------------------+  +--------------------+
| ActionContext   |  | Resources         |  | Manager            |
|   .resource()  -|->| (ResourceProvider)|->|   .acquire()       |
|                 |  |                   |  |   .register()      |
| ResourceProvider|  | builds Context,   |  |   .shutdown()      |
| (port trait)    |  | wires Scope       |  |                    |
+-----------------+  +-------------------+  +--------------------+
     Domain              System                   System
```

`nebula-action` defines the `ResourceProvider` port trait. `nebula-engine`
implements it via a bridge adapter (`Resources`) that wraps
`nebula_resource::Manager`. Actions never depend on `nebula-resource` directly.

## Engine Bridge (`crates/engine/src/resource.rs`)

The `Resources` struct adapts `Manager` to `ResourceProvider`:

```rust
pub(crate) struct Resources {
    manager: Arc<Manager>,
    scope: Scope,
    workflow_id: String,
    execution_id: String,
    cancellation: CancellationToken,
}

let resources = Resources::new(manager, "workflow-abc", "exec-123", cancellation);
```

Its `ResourceProvider::acquire` builds a `Context` and delegates to the manager:

```rust
async fn acquire(&self, key: &str) -> Result<Box<dyn Any + Send>, ActionError> {
    let ctx = Context::new(self.scope.clone(), &self.workflow_id, &self.execution_id)
        .with_cancellation(self.cancellation.clone());
    let guard = self.manager.acquire(key, &ctx).await
        .map_err(|e| ActionError::fatal(format!("resource acquire failed: {e}")))?;
    Ok(Box::new(ResourceHandle::new(guard)))
}
```

The returned `ResourceHandle` wraps the type-erased `AnyGuard`. When the
handle drops, the pool guard returns the instance to the pool.

## Action Usage

Actions call `ActionContext::resource(key)`:

```rust
async fn execute(&self, input: Value, ctx: &ActionContext) -> Result<Value, ActionError> {
    let handle = ctx.resource("postgres").await?;
    let handle = handle.downcast::<ResourceHandle>()
        .map_err(|_| ActionError::fatal("unexpected resource type"))?;
    let conn: &PgConnection = handle.get::<PgConnection>()
        .ok_or_else(|| ActionError::fatal("wrong instance type"))?;
    // Use conn... when handle drops, connection returns to pool.
    Ok(result)
}
```

Without a resource provider configured, `.resource()` returns
`ActionError::Fatal("no resource provider configured")`.

## Scope Wiring

The engine bridge builds a `Scope` from execution context:

```rust
let scope = Scope::execution_in_workflow(&execution_id, &workflow_id, None);
// Produces: Scope::Execution { execution_id, workflow_id: Some(...), tenant_id: None }
```

`Manager::acquire` checks scope compatibility using `Strategy::Hierarchical`,
meaning the resource scope must **contain** the caller's scope:

| Resource scope | Caller scope | Allowed? |
|---|---|---|
| `Global` | any | Yes |
| `Tenant("A")` | `Execution { tenant_id: Some("A"), .. }` | Yes |
| `Tenant("A")` | `Execution { tenant_id: Some("B"), .. }` | No |
| `Workflow("wf-1")` | `Execution { workflow_id: Some("wf-1"), .. }` | Yes |
| `Workflow("wf-1")` | `Execution { workflow_id: Some("wf-2"), .. }` | No |

## Credential Pass-through

With the `credentials` feature enabled, the engine bridge can attach a
credential provider to the resource `Context`:

```rust
let resources = Resources::new(manager, wf_id, exec_id, cancel)
    .with_credentials(credential_provider);
```

Inside `acquire`, this is forwarded to the `Context` via
`ctx.with_credentials(provider)`. Resource implementations receive the
context in `Resource::create()` and can use `ctx.credentials` to fetch
secrets needed during instance creation (e.g., database passwords).

## Lifecycle Hooks

Register hooks on the `Manager` to intercept lifecycle operations:

```rust
// Built-in hooks
mgr.hooks().register(Arc::new(AuditHook));  // logs all events, priority 10
mgr.hooks().register(Arc::new(SlowAcquireHook::new(Duration::from_secs(2))));  // priority 90
```

Custom hooks implement `ResourceHook`:

```rust
impl ResourceHook for MyHook {
    fn name(&self) -> &str { "my-hook" }
    fn priority(&self) -> u32 { 50 }  // lower = runs first
    fn events(&self) -> Vec<HookEvent> { vec![HookEvent::Acquire, HookEvent::Release] }
    fn filter(&self) -> HookFilter { HookFilter::Prefix("db-".into()) }

    fn before<'a>(&'a self, event: &'a HookEvent, resource_id: &'a str, ctx: &'a Context)
        -> Pin<Box<dyn Future<Output = HookResult> + Send + 'a>> {
        Box::pin(async { HookResult::Continue })
        // Or: HookResult::Cancel(Error::Unavailable { .. })
    }

    fn after<'a>(&'a self, event: &'a HookEvent, resource_id: &'a str,
                  ctx: &'a Context, success: bool)
        -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async { /* observability only; errors never propagate */ })
    }
}
```

Hook execution during `Manager::acquire`:
1. Before-hooks run in priority order. `HookResult::Cancel` aborts acquire.
2. Pool acquisition happens.
3. After-hooks run with `success: true/false`. Errors are logged, not propagated.

`HookFilter` variants: `All` (default), `Resource(id)` (exact match),
`Prefix(prefix)` (startsWith).
