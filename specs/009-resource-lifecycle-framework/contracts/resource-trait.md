# Contract: Resource Trait

**Status**: Implemented (existing)
**Location**: `crates/resource/src/resource.rs`

## Resource Trait

The core contract for all managed resources. Modeled after bb8::ManageConnection.

```rust
pub trait Resource: Send + Sync + 'static {
    /// Configuration type (deserializable, validatable).
    type Config: Config;

    /// Instance type (what actions receive and use).
    type Instance: Send + Sync + 'static;

    /// Unique identifier for this resource type (e.g., "postgres", "redis").
    fn id(&self) -> &str;

    /// Create a new instance from configuration.
    /// Called when the pool needs to grow or replace a failed instance.
    async fn create(
        &self,
        config: &Self::Config,
        context: &Context,
    ) -> Result<Self::Instance, Error>;

    /// Check if an instance is still usable.
    /// Called before handing an idle instance to a caller.
    /// Default: always valid.
    async fn is_valid(&self, instance: &Self::Instance) -> Result<bool, Error> {
        let _ = instance;
        Ok(true)
    }

    /// Prepare an instance for reuse after being returned to the pool.
    /// Called when an instance is returned. Can reset state, refresh credentials.
    /// Default: no-op.
    async fn recycle(&self, instance: &mut Self::Instance) -> Result<(), Error> {
        let _ = instance;
        Ok(())
    }

    /// Clean up an instance when it is permanently removed from the pool.
    /// Called during eviction, shutdown, or when recycle fails.
    /// Default: drop.
    async fn cleanup(&self, instance: Self::Instance) -> Result<(), Error> {
        drop(instance);
        Ok(())
    }

    /// Resource dependencies — which resources must be initialized first.
    /// Returns resource IDs (matching other resources' `id()` values).
    /// Default: no dependencies.
    fn dependencies(&self) -> Vec<&str> {
        Vec::new()
    }
}
```

## Config Trait

```rust
pub trait Config: Send + Sync + 'static {
    /// Validate configuration before resource creation.
    /// Called at registration time (fail-fast).
    /// Default: always valid.
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
}
```

## Invariants

1. `id()` MUST return a stable, unique string for each resource type.
2. `create()` MUST produce a fully initialized, ready-to-use instance.
3. `is_valid()` MUST NOT have side effects (read-only probe).
4. `recycle()` MAY mutate the instance (e.g., reset transaction state).
5. `cleanup()` MUST release all underlying OS resources (connections, file handles).
6. `dependencies()` MUST NOT include self (no self-dependency).
7. Dependency cycles are detected and rejected at registration time.

## Extension Points

- **Phase 4**: Resources MAY also implement `HealthCheckable` for periodic health monitoring.
- **Phase 7**: `#[derive(Resource)]` will generate boilerplate for `id()`, `create()`, `cleanup()`.
