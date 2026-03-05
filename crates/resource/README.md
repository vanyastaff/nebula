# nebula-resource

Resource lifecycle and pooling runtime for the Nebula workflow engine.

## Overview

`nebula-resource` provides:

- typed resource contracts via `Resource` and `Config`
- pooling with acquire/recycle/cleanup lifecycle
- scope isolation (global/tenant/workflow/execution/action/custom)
- manager-level orchestration (registration, dependency order, shutdown)
- health, quarantine, hooks, events, and optional autoscaling/metrics

## Quick Example

```rust
use nebula_core::ResourceKey;
use nebula_resource::{Context, Manager, PoolConfig, Resource, ResourceMetadata, Scope};

#[derive(Clone)]
struct MyConfig;

impl nebula_resource::Config for MyConfig {}

struct MyResource;

impl Resource for MyResource {
    type Config = MyConfig;
    type Instance = String;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::from_key(ResourceKey::try_from("my-resource").expect("valid key"))
    }

    async fn create(&self, _config: &Self::Config, _ctx: &Context) -> nebula_resource::Result<Self::Instance> {
        Ok("instance".to_string())
    }
}

let manager = Manager::new();
manager.register(MyResource, MyConfig, PoolConfig::default())?;

let key = ResourceKey::try_from("my-resource")?;
let ctx = Context::new(Scope::Global, nebula_resource::WorkflowId::new(), nebula_resource::ExecutionId::new());
let _guard = manager.acquire(&key, &ctx).await?;
```

## Notes

- Reuse checks use `is_reusable()` (not `is_valid()`).
- The built-in HTTP resource implementation was moved from core module code to `examples/http_resource.rs`.

## Documentation

- crate-level docs: `src/lib.rs`
- full design docs: `../../docs/crates/resource/README.md`
- implementation tutorials: `docs/` in this crate
