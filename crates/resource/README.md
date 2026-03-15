# nebula-resource

Resource lifecycle and pooling runtime for the Nebula workflow engine.

Provides typed resource contracts, bounded connection pooling, scope isolation,
health monitoring, quarantine with automatic recovery, lifecycle event streaming,
and pre/post hooks — all behind a single `Manager` API.

## Quick Start

```rust
use nebula_core::ResourceKey;
use nebula_resource::{Config, Context, Manager, PoolConfig, Resource, ResourceMetadata, Scope};

#[derive(Clone)]
struct MyConfig;
impl Config for MyConfig {}

struct MyResource;

impl Resource for MyResource {
    type Config   = MyConfig;
    type Instance = String;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::from_key(ResourceKey::try_from("my-resource").expect("valid key"))
    }

    async fn create(
        &self,
        _config: &MyConfig,
        _ctx: &Context,
    ) -> nebula_resource::Result<String> {
        Ok("instance".to_string())
    }
}

let manager = Manager::new();
manager.register(MyResource, MyConfig, PoolConfig::default())?;

let key = ResourceKey::try_from("my-resource")?;
let ctx = Context::new(
    Scope::Global,
    nebula_resource::WorkflowId::new(),
    nebula_resource::ExecutionId::new(),
);
let guard = manager.acquire(&key, &ctx).await?;
// `guard` returns the instance to the pool on drop.
// Call `guard.taint()` before dropping to force cleanup instead of recycle.
```

## Notes

- Reuse validation uses `is_reusable()` (not `is_valid()`).
- Reuse `use nebula_resource::prelude::*` to import the most common types at once.

## Documentation

| Document | Contents |
|----------|----------|
| [`docs/README.md`](docs/README.md) | Overview, concepts, feature matrix, crate layout |
| [`docs/architecture.md`](docs/architecture.md) | Module map, data flow, design invariants |
| [`docs/api-reference.md`](docs/api-reference.md) | Complete typed API reference |
| [`docs/pooling.md`](docs/pooling.md) | Pool config, strategies, backpressure, auto-scaling |
| [`docs/health-and-quarantine.md`](docs/health-and-quarantine.md) | Health checks, quarantine, recovery |
| [`docs/events-and-hooks.md`](docs/events-and-hooks.md) | EventBus catalog, subscriptions, hook system |
| [`docs/adapters.md`](docs/adapters.md) | Writing Resource adapter / driver crates |
