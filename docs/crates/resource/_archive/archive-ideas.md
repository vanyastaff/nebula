## Archived Ideas (from `docs/archive/architecture-v2.md`)

### Typed pool abstraction draft

Historical design proposed a typed pool registry:

```rust
pub struct ResourcePool {
    pools: HashMap<TypeId, Box<dyn TypedPool>>,
    metrics: PoolMetrics,
}

#[async_trait]
pub trait TypedPool: Send + Sync {
    type Resource: Send + Sync + 'static;

    async fn acquire(&self) -> Result<PooledResource<Self::Resource>, Error>;
    fn stats(&self) -> PoolStats;
    async fn health_check(&self) -> Result<HealthStatus, Error>;
}
```

### Lifecycle hooks (draft)

- `on_create`
- `on_acquire`
- `on_release`
- `on_destroy`

The current crate already models similar lifecycle concerns; this section is kept as a
reference for future API evolution and observability hooks.

