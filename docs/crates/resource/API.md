# API

## Define a resource

```rust
use nebula_resource::{Config, Context, Resource, Result};

#[derive(Clone)]
struct DbConfig {
    dsn: String,
}

impl Config for DbConfig {
    fn validate(&self) -> Result<()> {
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

    fn id(&self) -> &str {
        "db.main"
    }

    async fn create(&self, cfg: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
        Ok(format!("conn:{}", cfg.dsn))
    }
}
```

## Register and acquire

```rust
use nebula_resource::{Context, Manager, PoolConfig, Scope};

let manager = Manager::new();
manager.register(
    DbResource,
    DbConfig { dsn: "postgres://...".into() },
    PoolConfig::default(),
)?;

let ctx = Context::new(Scope::Global, "wf-1", "exec-1");
let guard = manager.acquire("db.main", &ctx).await?;
let conn = guard.as_any().downcast_ref::<String>().unwrap();
```

## Typed acquire helper

```rust
let typed = manager.acquire_typed(DbResource, &ctx).await?;
let conn: &String = typed.get();
```

## Scope-aware registration

```rust
use nebula_resource::{Scope, Strategy};

manager.register_scoped(
    DbResource,
    cfg,
    PoolConfig::default(),
    Scope::tenant("tenant-a"),
)?;

assert!(Strategy::Hierarchical.is_compatible(
    &Scope::tenant("tenant-a"),
    &Scope::workflow_in_tenant("wf1", "tenant-a")
));
```

## Health and quarantine

- `ManagerBuilder::health_config(...)` configures background checks.
- `Manager::set_health_state(...)` updates effective state.
- `QuarantineManager` blocks acquire for quarantined resource ids.

## Shutdown

```rust
manager.shutdown().await?;
```

Use `shutdown_with_config` when phased graceful shutdown timing must be controlled.
