# Contract: ResourceProvider (Action Port)

**Status**: Implemented (existing)
**Location**: `crates/action/src/provider.rs` (trait), `crates/engine/src/resource.rs` (implementation)

## ResourceProvider Trait (Action Side)

Port trait defined in the action crate. Actions use this to acquire managed resources without knowing the underlying resource management system.

```rust
#[async_trait]
pub trait ResourceProvider: Send + Sync {
    /// Acquire a resource instance by key.
    /// Returns type-erased Box<dyn Any + Send> — caller must downcast.
    async fn acquire(&self, key: &str) -> Result<Box<dyn std::any::Any + Send>, ActionError>;
}
```

## Usage in ActionContext

```rust
// Actions access resources through context
impl ProcessAction for MyAction {
    async fn execute(&self, input: Input, ctx: &ActionContext) -> Result<...> {
        // Acquire by key — returns type-erased box
        let resource = ctx.resource("my_database").await?;

        // Downcast to concrete type
        let handle = resource.downcast::<ResourceHandle>().expect("type mismatch");

        // Use the resource...
    }
}
```

## ActionContext Integration

```rust
impl ActionContext {
    /// Inject ResourceProvider (called by engine during setup)
    pub fn with_resources(mut self, resources: Arc<dyn ResourceProvider>) -> Self;

    /// Acquire a resource by key (convenience method)
    pub async fn resource(&self, key: &str) -> Result<Box<dyn Any + Send>, ActionError>;
}
```

## Engine Bridge: Resources

```rust
// In crates/engine/src/resource.rs
pub(crate) struct Resources {
    manager: Arc<nebula_resource::Manager>,
    workflow_id: String,
    execution_id: String,
    cancellation: CancellationToken,
}

#[async_trait]
impl ResourceProvider for Resources {
    async fn acquire(&self, key: &str) -> Result<Box<dyn Any + Send>, ActionError> {
        let ctx = Context::new(Scope::Global, &self.workflow_id, &self.execution_id)
            .with_cancellation(self.cancellation.clone());

        let guard = self.manager.acquire(key, &ctx).await
            .map_err(|e| ActionError::fatal(format!("resource acquire failed: {e}")))?;

        Ok(Box::new(ResourceHandle::new(guard)))
    }
}
```

## Invariants

1. `acquire()` MUST respect the resource pool's acquire timeout.
2. `acquire()` MUST respect cancellation (return immediately if token is cancelled).
3. The returned `Box<dyn Any>` wraps a `ResourceHandle` which wraps a `Guard`.
4. When the `ResourceHandle` is dropped, the instance MUST be returned to the pool.
5. If no ResourceProvider is configured, `ctx.resource()` returns `ActionError::Fatal`.

## Lifecycle Flow

```
Action calls ctx.resource("key")
    → ActionContext delegates to ResourceProvider::acquire("key")
    → Resources (engine bridge) creates Context with scope/cancellation
    → Manager::acquire("key", ctx) looks up pool, acquires instance
    → Pool returns Guard<R::Instance> with return-to-pool callback
    → Manager wraps in AnyGuard → ResourceHandle
    → Bridge wraps in Box<dyn Any + Send>
    → Action downcasts and uses
    → On drop: Guard callback returns instance to pool
```
