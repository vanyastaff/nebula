# API

## Public Surface

- stable APIs:
  - `Resource`, `Config`, `Context`, `Scope`, `Strategy`
  - `Manager`, `ManagerBuilder`, `Pool`, `PoolConfig`, `PoolStrategy`
  - `ResourceProvider`, `ResourceRef`
  - `Error` and `Result`
- experimental APIs:
  - autoscaling and some advanced observability behavior should be treated as evolving contracts.
- hidden/internal APIs:
  - `manager_guard`, `manager_pool` are internal orchestration details.

## Usage Patterns

- register once at startup, acquire many times during execution.
- prefer typed acquire (`acquire_typed`) where call site has static resource type.
- use scoped registration (`register_scoped`) for tenant/workflow isolation.

## Minimal Example

```rust
use nebula_resource::{Config, Context, Manager, PoolConfig, Resource, Scope};

#[derive(Clone)]
struct DbConfig { dsn: String }

impl Config for DbConfig {
    fn validate(&self) -> nebula_resource::Result<()> {
        if self.dsn.is_empty() {
            return Err(nebula_resource::Error::configuration("dsn is empty"));
        }
        Ok(())
    }
}

struct DbResource;

impl Resource for DbResource {
    type Config = DbConfig;
    type Instance = String;

    fn id(&self) -> &str { "db.main" }

    async fn create(
        &self,
        cfg: &Self::Config,
        _ctx: &Context,
    ) -> nebula_resource::Result<Self::Instance> {
        Ok(format!("conn:{}", cfg.dsn))
    }
}

# async fn run() -> nebula_resource::Result<()> {
let manager = Manager::new();
manager.register(DbResource, DbConfig { dsn: "postgres://...".into() }, PoolConfig::default())?;

let ctx = Context::new(Scope::Global, "wf-1", "exec-1");
let guard = manager.acquire("db.main", &ctx).await?;
let _conn = guard.as_any().downcast_ref::<String>().unwrap();
# Ok(()) }
```

## Advanced Example

```rust
use nebula_resource::{Context, Manager, ManagerBuilder, PoolConfig, Scope};

# async fn run() -> nebula_resource::Result<()> {
let manager = ManagerBuilder::new().build();

let scope = Scope::workflow_in_tenant("wf-orders", "tenant-a");
manager.register_scoped(MyResource, (), PoolConfig::default(), scope)?;

let ctx = Context::new(
    Scope::execution_in_workflow("exec-42", "wf-orders", Some("tenant-a".into())),
    "wf-orders",
    "exec-42",
);

let typed = manager.acquire_typed(MyResource, &ctx).await?;
let _instance = typed.get();
# Ok(()) }
# struct MyResource;
# impl nebula_resource::Resource for MyResource {
#   type Config = (); type Instance = String;
#   fn id(&self)->&str{"my"}
#   async fn create(&self,_:&(),_:&Context)->nebula_resource::Result<Self::Instance>{Ok("ok".into())}
# }
```

## Error Semantics

- retryable errors:
  - `PoolExhausted`, `Timeout`, `Unavailable { retryable: true }`, `CircuitBreakerOpen`.
- fatal errors:
  - structural misconfiguration (`Configuration`, `Validation`) and non-retryable unavailable states.
- validation errors:
  - `Config::validate` and `PoolConfig::validate` must fail fast before activation.

## Compatibility Rules

- what changes require major version bump:
  - scope containment semantics
  - acquire/release lifecycle guarantees
  - error variant meaning changes
  - hook/event schema contract breaks
- deprecation policy:
  - additive APIs first, at least one minor cycle before removal unless security-critical.
