# Architectural Improvements for nebula-resource

## Executive Summary

After thorough analysis of the codebase, this document proposes **28 architectural improvements** categorized by priority and impact. The current implementation shows strong design foundations but has significant technical debt and incomplete integration points that limit its effectiveness.

**Total improvements proposed**: 28
**Critical (P0)**: 4 - Must fix immediately
**High priority (P1)**: 11 - Significant impact, schedule soon
**Medium priority (P2)**: 9 - Nice to have, can wait
**Low priority (P3)**: 4 - Polish, future consideration

**Quick wins vs long-term**: 8 quick wins (< 8 hours each), 12 medium-term (8-40 hours), 8 long-term refactorings (40+ hours)

**Biggest architectural insight**: The Manager-Pool integration is fundamentally broken because the manager doesn't actually use pools - it stores instances directly in a DashMap. This defeats the entire purpose of the pooling system and creates a major architectural mismatch that must be resolved before the system can be production-ready.

---

## Table of Contents

1. [API Design Improvements](#1-api-design-improvements)
2. [Type System Enhancements](#2-type-system-enhancements)
3. [Trait Hierarchy Refinements](#3-trait-hierarchy-refinements)
4. [Module Structure](#4-module-structure)
5. [Performance Optimizations](#5-performance-optimizations)
6. [Error Handling](#6-error-handling)
7. [Concurrency Patterns](#7-concurrency-patterns)
8. [Safety & Correctness](#8-safety--correctness)
9. [Testing Architecture](#9-testing-architecture)
10. [Dependencies Cleanup](#10-dependencies-cleanup)
11. [Implementation Order](#implementation-order)
12. [Appendix: Design Alternatives Considered](#appendix-design-alternatives-considered)

---

## 1. API Design Improvements

### I-001: Fix Manager-Pool Integration Disconnect

**Priority**: P0 (CRITICAL)
**Effort**: 24 hours
**Impact**: The ResourceManager doesn't actually use pools - it stores instances directly, defeating the entire pooling architecture.

#### Problem

The `ResourceManager` stores instances directly in a `DashMap`:

```rust
instances: Arc<DashMap<String, Arc<dyn Any + Send + Sync>>>,
```

The `get_by_id` method creates instances on-demand and stores them, but never uses the `PoolManager` that's imported and available. This means:
- Pooling features (min/max size, reuse, health checks) are completely bypassed
- Resources are never actually pooled
- The `PoolManager` field is dead code
- Users think they're getting pooling but they're not

#### Current Code

```rust
// manager/mod.rs
pub struct ResourceManager {
    registry: Arc<RwLock<HashMap<ResourceId, Arc<dyn ResourceFactory>>>>,
    instances: Arc<DashMap<String, Arc<dyn Any + Send + Sync>>>, // ❌ Direct storage
    #[cfg(feature = "pooling")]
    pool_manager: Arc<PoolManager>, // ❌ Never actually used!
    // ...
}

async fn get_by_id<T>(&self, resource_id: &ResourceId, context: &ResourceContext)
    -> ResourceResult<ResourceGuard<T>>
{
    let instance_key = self.build_instance_key(resource_id, &context.scope);

    // Try to get existing instance - this is NOT pooling!
    if let Some(instance) = self.instances.get(&instance_key) {
        if let Ok(typed_instance) = self.cast_to_typed_instance::<T>(instance.value()) {
            return Ok(self.create_guard(typed_instance));
        }
    }

    // Create new instance directly - pools are bypassed
    self.create_instance(resource_id, context).await
}
```

#### Proposed Solution

**Option A: Manager delegates to pools** (Recommended)

```rust
// manager/mod.rs
pub struct ResourceManager {
    registry: Arc<RwLock<HashMap<ResourceId, Arc<dyn ResourceFactory>>>>,
    metadata_cache: Arc<DashMap<ResourceId, ResourceMetadata>>,

    // Remove: direct instance storage
    // Add: pool manager is now the authoritative storage
    pool_manager: Arc<PoolManager>,

    // Track which resources use pooling
    pooled_resources: Arc<DashMap<ResourceId, bool>>,

    // For non-pooled resources only
    singleton_instances: Arc<DashMap<String, Arc<dyn Any + Send + Sync>>>,

    // ...
}

async fn get_by_id<T>(&self, resource_id: &ResourceId, context: &ResourceContext)
    -> ResourceResult<ResourceGuard<T>>
{
    // Check if this resource type supports pooling
    let metadata = self.get_metadata(resource_id)
        .ok_or_else(|| ResourceError::unavailable(
            resource_id.unique_key(), "Resource not registered", false
        ))?;

    if metadata.poolable {
        // Acquire from pool
        let pool_key = self.build_pool_key(resource_id, &context.scope);

        // Get or create pool for this resource type + scope
        let pool = self.pool_manager
            .get_or_create_pool::<T>(&pool_key, || {
                self.create_pool_factory(resource_id, context)
            })
            .await?;

        // Acquire from pool (this handles creation, reuse, health checks)
        let pooled_resource = pool.acquire().await?;

        // Convert to ResourceGuard
        self.create_guard_from_pooled(pooled_resource, pool)
    } else {
        // Non-pooled resource: singleton behavior
        self.get_singleton(resource_id, context).await
    }
}

// New helper: Create pool factory
fn create_pool_factory<T>(&self, resource_id: &ResourceId, context: &ResourceContext)
    -> impl Fn() -> BoxFuture<'static, ResourceResult<TypedResourceInstance<T>>>
{
    let factory = self.get_factory(resource_id)?;
    let config = self.get_default_config(resource_id)?;
    let context = context.clone();

    move || {
        let factory = factory.clone();
        let config = config.clone();
        let context = context.clone();

        Box::pin(async move {
            let instance = factory.create_instance(config, &context, &HashMap::new()).await?;
            // Cast and wrap
            Ok(TypedResourceInstance::new(instance, /* metadata */))
        })
    }
}
```

**Option B: Explicit pooling API** (Alternative - more explicit but more boilerplate)

```rust
// Separate APIs for pooled vs non-pooled
impl ResourceManager {
    /// Get a pooled resource (fails if resource doesn't support pooling)
    pub async fn acquire_pooled<T>(&self, resource_id: &ResourceId, context: &ResourceContext)
        -> ResourceResult<PooledResourceGuard<T>> { /* ... */ }

    /// Get a singleton resource (creates once, reuses forever)
    pub async fn get_singleton<T>(&self, resource_id: &ResourceId, context: &ResourceContext)
        -> ResourceResult<ResourceGuard<T>> { /* ... */ }

    /// Smart get: uses pooling if available, otherwise singleton
    pub async fn get<T>(&self, context: &ResourceContext)
        -> ResourceResult<ResourceGuard<T>> { /* ... */ }
}
```

#### Benefits

- **Correctness**: Pooling actually works as documented
- **Performance**: Resource reuse, connection warming, automatic sizing
- **Observability**: Pool metrics are now meaningful
- **Consistency**: One source of truth for resource lifecycle

#### Migration Path

1. Add `get_or_create_pool` method to `PoolManager`
2. Refactor `get_by_id` to check `metadata.poolable` and route accordingly
3. Move singleton logic to separate `get_singleton` method
4. Update `ResourceGuard` Drop to return to pool instead of no-op
5. Add integration tests verifying pool reuse
6. Update documentation to reflect actual behavior

---

### I-002: Remove Type Erasure Smell in Manager

**Priority**: P1 (HIGH)
**Effort**: 16 hours
**Impact**: Current type mapping via string matching is fragile and defeats type safety.

#### Problem

The `find_resource_id_for_type<T>` method uses string matching on type names:

```rust
fn find_resource_id_for_type<T>(&self) -> ResourceResult<ResourceId>
where
    T: 'static,
{
    let type_id = TypeId::of::<T>();

    // This is terrible - uses string matching!
    for entry in self.metadata_cache.iter() {
        if entry.key().name.contains(&std::any::type_name::<T>()) {
            return Ok(entry.key().clone());
        }
    }

    Err(ResourceError::unavailable("unknown",
        format!("No resource registered for type {}", std::any::type_name::<T>()), false))
}
```

**Issues**:
- `type_name` is not guaranteed to be stable across compiler versions
- String matching is fragile (what if name is substring of another?)
- `TypeId` is obtained but never used
- No compile-time type safety

#### Current Code

```rust
// This can fail at runtime in confusing ways
let db: ResourceGuard<PostgresConnection> = manager.get(&ctx).await?;
// If you have "PostgresConnection" and "PostgresConnectionPool" registered,
// which one matches? Undefined!
```

#### Proposed Solution

**Use a proper TypeId → ResourceId mapping**:

```rust
pub struct ResourceManager {
    registry: Arc<RwLock<HashMap<ResourceId, Arc<dyn ResourceFactory>>>>,
    metadata_cache: Arc<DashMap<ResourceId, ResourceMetadata>>,

    // Add: Explicit type mapping
    type_map: Arc<DashMap<TypeId, ResourceId>>,

    // ...
}

impl ResourceManager {
    /// Register a resource type with explicit type association
    pub fn register<R>(&self, resource: R) -> ResourceResult<()>
    where
        R: Resource + 'static,
        R::Instance: 'static, // Key: Instance type must be 'static
    {
        let metadata = resource.metadata();
        let resource_id = metadata.id.clone();

        // Store metadata
        self.metadata_cache.insert(resource_id.clone(), metadata);

        // Map TypeId of Instance to ResourceId
        let instance_type_id = TypeId::of::<R::Instance>();

        // Detect conflicts
        if let Some(existing) = self.type_map.get(&instance_type_id) {
            return Err(ResourceError::configuration(
                format!(
                    "Type {} already registered as {} (trying to register as {})",
                    std::any::type_name::<R::Instance>(),
                    existing.unique_key(),
                    resource_id.unique_key()
                )
            ));
        }

        self.type_map.insert(instance_type_id, resource_id.clone());

        // Create factory wrapper
        let factory = Arc::new(ResourceFactoryWrapper::new(resource));

        // Register in registry
        {
            let mut registry = self.registry.write();
            registry.insert(resource_id.clone(), factory);
        }

        Ok(())
    }

    fn find_resource_id_for_type<T>(&self) -> ResourceResult<ResourceId>
    where
        T: 'static,
    {
        let type_id = TypeId::of::<T>();

        self.type_map
            .get(&type_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| {
                ResourceError::unavailable(
                    "unknown",
                    format!(
                        "No resource registered for type {} (TypeId: {:?})",
                        std::any::type_name::<T>(),
                        type_id
                    ),
                    false,
                )
            })
    }
}
```

#### Benefits

- **Type safety**: Uses `TypeId`, not strings
- **Deterministic**: No string matching ambiguity
- **Error detection**: Catches duplicate registrations at registration time
- **Performance**: O(1) hash lookup instead of linear scan
- **Debuggability**: Clear error messages with TypeId information

#### Migration Path

1. Add `type_map: Arc<DashMap<TypeId, ResourceId>>` to `ResourceManager`
2. Update `register` to populate type_map
3. Update `find_resource_id_for_type` to use TypeId lookup
4. Add tests for duplicate type registration
5. Update error messages to be more helpful

---

### I-003: Fix ResourceGuard Drop Not Actually Releasing

**Priority**: P0 (CRITICAL)
**Effort**: 8 hours
**Impact**: Resources are never returned to pools, causing leaks and pool exhaustion.

#### Problem

The `ResourceGuard` Drop implementation calls a callback, but the callback in `create_guard` does nothing:

```rust
// manager/mod.rs
fn create_guard<T>(&self, instance: TypedResourceInstance<T>) -> ResourceGuard<T>
where
    T: Send + Sync + 'static,
{
    let instance_id = instance.instance_id();
    let resource_id = instance.resource_id().clone();

    ResourceGuard::new(instance, move |_instance| {
        // ❌ Empty callback - resource is NEVER returned to pool!
        // TODO comment acknowledges this is wrong
    })
}
```

#### Current Code

```rust
// User code
{
    let db = manager.get::<PostgresConn>(&ctx).await?;
    db.execute("SELECT 1").await?;
    // Drop happens here - but does nothing!
}
// Resource is lost, never returned to pool
```

#### Proposed Solution

```rust
fn create_guard<T>(&self, instance: TypedResourceInstance<T>, pool: Option<Arc<ResourcePool<T>>>)
    -> ResourceGuard<T>
where
    T: Send + Sync + 'static,
{
    let instance_id = instance.instance_id();
    let resource_id = instance.resource_id().clone();
    let manager_weak = Arc::downgrade(&Arc::new(self.clone())); // Weak ref to avoid cycles

    ResourceGuard::new(instance, move |instance| {
        // Spawn async cleanup task to avoid blocking Drop
        let pool = pool.clone();
        let manager_weak = manager_weak.clone();
        let instance_id = instance.instance_id();

        tokio::spawn(async move {
            if let Some(pool) = pool {
                // Return to pool
                if let Err(e) = pool.release(instance_id).await {
                    nebula_log::error!(
                        "Failed to return resource {} to pool: {}",
                        instance_id,
                        e
                    );
                }
            } else if let Some(manager) = manager_weak.upgrade() {
                // Non-pooled resource: just emit event
                manager.emit_lifecycle_event(LifecycleEvent::new(
                    resource_id.unique_key(),
                    LifecycleState::InUse,
                    LifecycleState::Idle,
                ));
            }
        });
    })
}
```

**Alternative: Use a release channel**:

```rust
pub struct ResourceManager {
    // Add: Channel for async cleanup
    release_tx: tokio::sync::mpsc::UnboundedSender<ReleaseRequest>,
    // ...
}

struct ReleaseRequest {
    instance_id: Uuid,
    resource_id: ResourceId,
    pool_key: Option<String>,
}

impl ResourceManager {
    pub fn new() -> Self {
        let (release_tx, mut release_rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn background task to handle releases
        tokio::spawn(async move {
            while let Some(req) = release_rx.recv().await {
                // Handle release asynchronously
                if let Some(pool_key) = req.pool_key {
                    // Return to pool
                } else {
                    // Update state
                }
            }
        });

        Self {
            release_tx,
            // ...
        }
    }

    fn create_guard<T>(&self, instance: TypedResourceInstance<T>) -> ResourceGuard<T> {
        let release_tx = self.release_tx.clone();
        let instance_id = instance.instance_id();
        let resource_id = instance.resource_id().clone();

        ResourceGuard::new(instance, move |_instance| {
            let _ = release_tx.send(ReleaseRequest {
                instance_id,
                resource_id,
                pool_key: None, // Set appropriately
            });
        })
    }
}
```

#### Benefits

- **Correctness**: Resources actually return to pools
- **No leaks**: Pool sizes stay stable
- **Performance**: Resource reuse works
- **Backpressure**: Pool exhaustion errors become meaningful

#### Migration Path

1. Add release channel or weak reference approach
2. Update `create_guard` to capture necessary state
3. Implement async cleanup logic
4. Add tests verifying pool return
5. Monitor pool metrics to verify reuse

---

### I-004: Add Explicit Async Drop Pattern

**Priority**: P1 (HIGH)
**Effort**: 12 hours
**Impact**: Async operations in Drop are problematic and can cause panics or blocking.

#### Problem

Both `ResourceGuard` and `PooledResource` have async cleanup needs but Drop is synchronous:

```rust
impl<T> Drop for PooledResource<T> {
    fn drop(&mut self) {
        // Note: In a real implementation, we'd need to handle the async release
        // This would typically involve sending the instance_id to a cleanup task

        // ❌ Can't call async release() from sync Drop!
    }
}
```

This is a known Rust limitation - Drop can't be async.

#### Current Code

```rust
// These patterns don't work:
impl<T> Drop for PooledResource<T> {
    fn drop(&mut self) {
        // ❌ Can't do this - Drop is not async
        // pool.release(self.instance_id).await;

        // ❌ Can't block_on in Drop - might already be in async runtime
        // tokio::runtime::Runtime::new().unwrap()
        //     .block_on(pool.release(self.instance_id));
    }
}
```

#### Proposed Solution

**Pattern 1: Explicit async cleanup** (Recommended for user-facing API):

```rust
pub struct ResourceGuard<T> {
    resource: Option<TypedResourceInstance<T>>,
    cleanup_tx: Option<tokio::sync::mpsc::UnboundedSender<CleanupRequest>>,
    instance_id: Uuid,
}

impl<T> ResourceGuard<T> {
    /// Explicitly release the resource (preferred method)
    pub async fn release(mut self) -> ResourceResult<()> {
        if let Some(resource) = self.resource.take() {
            // Async cleanup logic here
            self.do_async_cleanup(resource).await?;
        }
        Ok(())
    }

    /// Get a reference without consuming
    pub fn as_ref(&self) -> Option<&T> {
        self.resource.as_ref().map(|r| r.as_ref())
    }
}

impl<T> Drop for ResourceGuard<T> {
    fn drop(&mut self) {
        if let Some(resource) = self.resource.take() {
            // Send to background cleanup task
            if let Some(tx) = self.cleanup_tx.take() {
                let instance_id = self.instance_id;
                let _ = tx.send(CleanupRequest {
                    instance_id,
                    resource: Box::new(resource), // Type-erased
                });
            } else {
                // Log warning: resource dropped without cleanup
                nebula_log::warn!(
                    "Resource {} dropped without proper cleanup - consider using release() explicitly",
                    self.instance_id
                );
            }
        }
    }
}
```

**Pattern 2: Scoped async contexts** (For advanced users):

```rust
/// Scoped resource that must be used within a scope
pub struct ScopedResource<'scope, T> {
    resource: TypedResourceInstance<T>,
    _phantom: PhantomData<&'scope ()>,
}

impl ResourceManager {
    /// Use a resource within a scoped async context
    pub async fn with_resource<T, F, R>(&self, ctx: &ResourceContext, f: F) -> ResourceResult<R>
    where
        T: Send + Sync + 'static,
        F: FnOnce(&T) -> BoxFuture<'_, ResourceResult<R>>,
    {
        let resource = self.acquire_internal::<T>(ctx).await?;
        let result = f(resource.as_ref()).await;

        // Cleanup happens here - async is fine
        self.release_internal(resource).await?;

        result
    }
}

// Usage:
manager.with_resource::<PostgresConn, _, _>(&ctx, |db| {
    Box::pin(async move {
        db.execute("SELECT 1").await
    })
}).await?;
```

**Pattern 3: Background cleanup task** (Best for pooling):

```rust
pub struct ResourceManager {
    cleanup_tx: tokio::sync::mpsc::UnboundedSender<CleanupRequest>,
    cleanup_handle: tokio::task::JoinHandle<()>,
}

impl ResourceManager {
    pub fn new() -> Self {
        let (cleanup_tx, mut cleanup_rx) = tokio::sync::mpsc::unbounded_channel();

        let cleanup_handle = tokio::spawn(async move {
            while let Some(req) = cleanup_rx.recv().await {
                match req {
                    CleanupRequest::ReturnToPool { instance_id, pool_key } => {
                        // Async pool return
                    }
                    CleanupRequest::Destroy { instance_id } => {
                        // Async cleanup
                    }
                }
            }
        });

        Self { cleanup_tx, cleanup_handle }
    }
}
```

#### Benefits

- **Safety**: No blocking in async runtime
- **Correctness**: Cleanup actually happens
- **Ergonomics**: Users can choose explicit or automatic cleanup
- **Observability**: Can track cleanup failures

#### Migration Path

1. Add cleanup channel to ResourceManager
2. Spawn background cleanup task
3. Update Drop to send cleanup requests
4. Add explicit `release()` method for advanced users
5. Add `with_resource()` scoped pattern
6. Document best practices
7. Add tests for cleanup timing

---

### I-005: Improve ResourceContext Overhead

**Priority**: P2 (MEDIUM)
**Effort**: 10 hours
**Impact**: ResourceContext is cloned frequently but contains large nested structures.

#### Problem

`ResourceContext` is 400+ bytes and contains many `HashMap`s and `String`s. It's cloned on every resource operation:

```rust
pub struct ResourceContext {
    pub context_id: Uuid,                              // 16 bytes
    pub created_at: DateTime<Utc>,                     // 16 bytes
    pub workflow: WorkflowContext,                     // ~100 bytes
    pub execution: ExecutionContext,                   // ~80 bytes
    pub action: Option<ActionContext>,                 // ~100 bytes
    pub tracing: TracingContext,                       // ~80 bytes
    pub identity: IdentityContext,                     // ~60 bytes
    pub environment: EnvironmentContext,               // ~100 bytes
    pub scope: ResourceScope,                          // ~50 bytes
    pub metadata: HashMap<String, serde_json::Value>,  // 48+ bytes
    pub tags: HashMap<String, String>,                 // 48+ bytes
}
```

Total: ~600+ bytes per clone, and it's cloned:
- When deriving context for actions
- When passing to resource factory
- When storing in instance metadata
- When emitting events

#### Current Code

```rust
// Every one of these clones the entire context
let instance = factory.create_instance(config, context, &deps).await?;
//                                            ^^^^^^^ Clone

let metadata = ResourceInstanceMetadata {
    context: context.clone(), // Another clone
    // ...
};
```

#### Proposed Solution

**Use Arc for shared context**:

```rust
/// Lightweight context handle
#[derive(Debug, Clone)]
pub struct ResourceContext {
    inner: Arc<ResourceContextInner>,
}

/// Actual context data
struct ResourceContextInner {
    pub context_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub workflow: WorkflowContext,
    pub execution: ExecutionContext,
    pub action: Option<ActionContext>,
    pub tracing: TracingContext,
    pub identity: IdentityContext,
    pub environment: EnvironmentContext,
    pub scope: ResourceScope,
    pub metadata: Arc<DashMap<String, serde_json::Value>>, // Shared mutable map
    pub tags: Arc<DashMap<String, String>>,                // Shared mutable map
}

impl ResourceContext {
    /// Clone is now cheap - just an Arc clone
    pub fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Create a derived context (does a deep copy when needed)
    pub fn derive_for_action(&self, action_id: String, action_name: String) -> Self {
        Self {
            inner: Arc::new(ResourceContextInner {
                context_id: Uuid::new_v4(),
                action: Some(ActionContext {
                    action_id: action_id.clone(),
                    action_name,
                    action_path: vec![action_id.clone()],
                    attempt_number: 1,
                    started_at: Utc::now(),
                }),
                scope: ResourceScope::action(action_id),
                // Share immutable parts
                workflow: self.inner.workflow.clone(),
                execution: self.inner.execution.clone(),
                tracing: self.inner.tracing.clone(),
                identity: self.inner.identity.clone(),
                environment: self.inner.environment.clone(),
                // Share mutable maps
                metadata: Arc::clone(&self.inner.metadata),
                tags: Arc::clone(&self.inner.tags),
                created_at: self.inner.created_at,
            }),
        }
    }

    /// Add metadata (uses DashMap for concurrent updates)
    pub fn add_metadata<K, V>(&self, key: K, value: V)
    where
        K: Into<String>,
        V: Into<serde_json::Value>,
    {
        self.inner.metadata.insert(key.into(), value.into());
    }
}
```

#### Benefits

- **Performance**: Clone is 8 bytes (Arc pointer) instead of 600+
- **Memory**: Shared context data
- **Concurrency**: DashMap allows concurrent metadata updates
- **API**: Same interface, better performance

#### Migration Path

1. Create `ResourceContextInner` struct
2. Wrap in `Arc<ResourceContextInner>`
3. Replace `HashMap` with `Arc<DashMap>` for metadata/tags
4. Update builder to create Arc'd version
5. Benchmark clone performance
6. Update tests

---

## 2. Type System Enhancements

### I-006: Add Resource Type States

**Priority**: P2 (MEDIUM)
**Effort**: 20 hours
**Impact**: Prevent invalid state transitions at compile time using typestate pattern.

#### Problem

Currently, lifecycle states are runtime checked via an enum:

```rust
pub enum LifecycleState {
    Created,
    Initializing,
    Ready,
    InUse,
    Idle,
    // ...
}
```

Invalid transitions can happen at runtime:
- Using a resource before it's initialized
- Trying to cleanup a resource that's in use
- Re-initializing a terminated resource

#### Current Code

```rust
// Nothing prevents this at compile time:
let instance = resource.create(&config, &ctx).await?; // State: Created
db.query("SELECT 1").await?; // ❌ Should fail - not initialized!
```

#### Proposed Solution

**Use typestate pattern**:

```rust
// State types
pub struct Created;
pub struct Initialized;
pub struct Ready;
pub struct InUse;
pub struct Terminated;

/// Resource instance with state encoded in type
pub struct ResourceInstance<T, S> {
    inner: T,
    metadata: ResourceInstanceMetadata,
    _state: PhantomData<S>,
}

/// Only Created → Initialized transition is allowed
impl<T> ResourceInstance<T, Created> {
    pub async fn initialize(self) -> ResourceResult<ResourceInstance<T, Initialized>> {
        // Initialization logic
        Ok(ResourceInstance {
            inner: self.inner,
            metadata: self.metadata,
            _state: PhantomData,
        })
    }
}

/// Only Initialized → Ready transition
impl<T> ResourceInstance<T, Initialized> {
    pub async fn make_ready(self) -> ResourceResult<ResourceInstance<T, Ready>> {
        // Readiness checks
        Ok(ResourceInstance {
            inner: self.inner,
            metadata: self.metadata,
            _state: PhantomData,
        })
    }
}

/// Only Ready resources can be used
impl<T> ResourceInstance<T, Ready> {
    pub fn acquire(self) -> ResourceInstance<T, InUse> {
        ResourceInstance {
            inner: self.inner,
            metadata: self.metadata,
            _state: PhantomData,
        }
    }
}

/// Only InUse can be released
impl<T> ResourceInstance<T, InUse> {
    pub fn release(self) -> ResourceInstance<T, Ready> {
        ResourceInstance {
            inner: self.inner,
            metadata: self.metadata,
            _state: PhantomData,
        }
    }
}

/// Ready or InUse can be terminated
impl<T> ResourceInstance<T, Ready> {
    pub async fn terminate(self) -> ResourceResult<ResourceInstance<T, Terminated>> {
        // Cleanup
        Ok(ResourceInstance {
            inner: self.inner,
            metadata: self.metadata,
            _state: PhantomData,
        })
    }
}

impl<T> ResourceInstance<T, InUse> {
    pub async fn terminate(self) -> ResourceResult<ResourceInstance<T, Terminated>> {
        // Forced cleanup
        Ok(ResourceInstance {
            inner: self.inner,
            metadata: self.metadata,
            _state: PhantomData,
        })
    }
}
```

**Usage**:

```rust
// Compile-time state enforcement
let instance = resource.create(&config, &ctx).await?; // ResourceInstance<DB, Created>
let instance = instance.initialize().await?;          // ResourceInstance<DB, Initialized>
let instance = instance.make_ready().await?;          // ResourceInstance<DB, Ready>
let instance = instance.acquire();                    // ResourceInstance<DB, InUse>

// ✅ This compiles
instance.query("SELECT 1").await?;

// ❌ This doesn't compile - wrong state!
let instance = resource.create(&config, &ctx).await?;
instance.query("SELECT 1").await?; // Error: method not found for ResourceInstance<DB, Created>
```

#### Benefits

- **Compile-time safety**: Invalid transitions impossible
- **Self-documenting**: Type shows state
- **Zero runtime cost**: PhantomData is zero-sized
- **Clearer API**: Can't use resource incorrectly

#### Migration Path

1. Create state marker types
2. Add `PhantomData<S>` to ResourceInstance
3. Implement state transition methods
4. Update Resource trait to return typed states
5. Update manager to handle typestate
6. Add examples showing compile-time enforcement
7. Consider keeping runtime enum for serialization/logging

---

### I-007: Add GAT-based Resource Trait

**Priority**: P2 (MEDIUM)
**Effort**: 16 hours
**Impact**: Allow resources to have borrowed data in their instances (currently limited to 'static).

#### Problem

All resource instances must be `'static` because of the trait bounds:

```rust
pub trait Resource: Send + Sync + 'static {
    type Instance: ResourceInstance; // Implicitly 'static
}
```

This prevents resources that:
- Borrow from configuration
- Reference other resources
- Use arena allocation
- Have explicit lifetimes

#### Current Code

```rust
// Can't do this:
struct DatabaseConnection<'a> {
    config: &'a DbConfig, // ❌ Can't have lifetime
    pool: &'a ConnectionPool,
}

impl Resource for Database {
    type Instance = DatabaseConnection; // ❌ Error: lifetime required
}
```

#### Proposed Solution

**Use Generic Associated Types (GATs)**:

```rust
#[async_trait]
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;

    // GAT allows lifetime parameters
    type Instance<'a>: ResourceInstance
    where
        Self: 'a;

    fn metadata(&self) -> ResourceMetadata;

    async fn create<'a>(
        &'a self,
        config: &'a Self::Config,
        context: &'a ResourceContext,
    ) -> ResourceResult<Self::Instance<'a>>;
}

// Now this works:
struct Database {
    pool: ConnectionPool,
}

struct DatabaseConnection<'a> {
    pool: &'a ConnectionPool,
    config: &'a DbConfig,
}

impl Resource for Database {
    type Config = DbConfig;
    type Instance<'a> = DatabaseConnection<'a>;

    async fn create<'a>(
        &'a self,
        config: &'a Self::Config,
        context: &'a ResourceContext,
    ) -> ResourceResult<Self::Instance<'a>> {
        Ok(DatabaseConnection {
            pool: &self.pool,
            config,
        })
    }
}
```

#### Benefits

- **Flexibility**: Resources can borrow data
- **Performance**: Avoid cloning configuration
- **Expressiveness**: More natural API for some resources
- **Zero-cost**: Lifetimes are compile-time only

#### Migration Path

1. Check MSRV supports GATs (Rust 1.65+)
2. Add GAT to Resource trait
3. Update existing resources to use `type Instance<'a> = Foo`
4. Update manager to handle borrowed instances
5. Add example resources using lifetimes
6. Document lifetime requirements

---

## 3. Trait Hierarchy Refinements

### I-008: Split Resource Trait into Smaller Traits

**Priority**: P1 (HIGH)
**Effort**: 12 hours
**Impact**: Current Resource trait does too much; makes it hard to implement partially.

#### Problem

The `Resource` trait combines:
- Metadata (static)
- Creation (required)
- Initialization (optional)
- Validation (optional)
- Cleanup (optional)
- Dependencies (optional)

All resources must implement all methods, even if defaulted.

#### Current Code

```rust
#[async_trait]
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Instance: ResourceInstance;

    fn metadata(&self) -> ResourceMetadata;
    async fn create(&self, config: &Self::Config, context: &ResourceContext)
        -> ResourceResult<Self::Instance>;
    async fn initialize(&self, instance: &mut Self::Instance) -> ResourceResult<()> {
        Ok(()) // Most resources don't need this
    }
    async fn cleanup(&self, instance: Self::Instance) -> ResourceResult<()> {
        Ok(()) // Most resources don't need this
    }
    async fn validate_instance(&self, instance: &Self::Instance) -> ResourceResult<bool> {
        Ok(true) // Most resources don't implement
    }
    fn dependencies(&self) -> Vec<ResourceId> {
        vec![] // Most resources have no dependencies
    }
}
```

#### Proposed Solution

**Trait hierarchy with extension traits**:

```rust
/// Core trait - only metadata and creation
#[async_trait]
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Instance: Send + Sync + 'static;

    fn metadata(&self) -> ResourceMetadata;

    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> ResourceResult<Self::Instance>;
}

/// Optional: Resource needs explicit initialization
#[async_trait]
pub trait InitializableResource: Resource {
    async fn initialize(&self, instance: &mut Self::Instance) -> ResourceResult<()>;
}

/// Optional: Resource needs explicit cleanup
#[async_trait]
pub trait CleanupableResource: Resource {
    async fn cleanup(&self, instance: Self::Instance) -> ResourceResult<()>;
}

/// Optional: Resource can be validated
#[async_trait]
pub trait ValidatableResource: Resource {
    async fn validate(&self, instance: &Self::Instance) -> ResourceResult<bool>;
}

/// Optional: Resource has dependencies
pub trait DependentResource: Resource {
    fn dependencies(&self) -> Vec<ResourceId>;
}

/// Convenience: Resource with full lifecycle
#[async_trait]
pub trait ManagedResource:
    Resource + InitializableResource + CleanupableResource + ValidatableResource
{
    // Default implementations in terms of other traits
}
```

**Manager adaptation**:

```rust
impl ResourceManager {
    async fn create_instance<R>(&self, resource: &R, config: &R::Config, ctx: &ResourceContext)
        -> ResourceResult<R::Instance>
    where
        R: Resource,
    {
        // Create
        let mut instance = resource.create(config, ctx).await?;

        // Initialize if supported
        if let Some(init) = resource as &dyn InitializableResource {
            init.initialize(&mut instance).await?;
        }

        // Validate if supported
        if let Some(validator) = resource as &dyn ValidatableResource {
            if !validator.validate(&instance).await? {
                return Err(ResourceError::initialization(
                    "resource",
                    "Validation failed after creation"
                ));
            }
        }

        Ok(instance)
    }
}
```

#### Benefits

- **Simplicity**: Most resources only implement core trait
- **Clarity**: Explicit about what a resource supports
- **Composability**: Can mix and match behaviors
- **Discovery**: IDE shows only relevant methods
- **Type safety**: Can require specific traits in generics

#### Migration Path

1. Create new trait hierarchy
2. Keep old Resource trait for compatibility
3. Implement new traits for existing resources
4. Update manager to detect and use trait implementations
5. Add blanket impl for old trait in terms of new traits
6. Deprecate old trait
7. Remove old trait in next breaking release

---

### I-009: Unify Poolable and HealthCheckable with Resource

**Priority**: P2 (MEDIUM)
**Effort**: 8 hours
**Impact**: Currently separate traits, but tightly coupled to Resource lifecycle.

#### Problem

`Poolable` and `HealthCheckable` are separate traits but:
- Only make sense for resources
- Require knowledge of Resource internals
- Duplicate some Resource metadata (poolable flag in metadata vs Poolable trait)

```rust
// Redundant information
let metadata = resource.metadata();
if metadata.poolable {  // Check metadata
    // But also need to check if trait is implemented
    if let Some(poolable) = resource as &dyn Poolable {
        let config = poolable.pool_config();
    }
}
```

#### Current Code

```rust
pub trait Poolable: Send + Sync {
    fn pool_config(&self) -> PoolConfig;
    fn is_valid_for_pool(&self) -> bool;
    // ...
}

pub struct ResourceMetadata {
    pub poolable: bool, // Redundant!
    pub health_checkable: bool, // Redundant!
}
```

#### Proposed Solution

**Associate with Resource via extension traits**:

```rust
/// Resource that supports pooling
#[async_trait]
pub trait PoolableResource: Resource {
    fn pool_config(&self) -> PoolConfig {
        PoolConfig::default()
    }

    async fn prepare_for_pool(&self, instance: &mut Self::Instance) -> ResourceResult<()> {
        Ok(())
    }

    async fn prepare_for_acquisition(&self, instance: &mut Self::Instance) -> ResourceResult<()> {
        Ok(())
    }

    async fn is_valid_for_pool(&self, instance: &Self::Instance) -> bool {
        true
    }
}

/// Resource that supports health checks
#[async_trait]
pub trait HealthCheckableResource: Resource {
    async fn health_check(&self, instance: &Self::Instance) -> ResourceResult<HealthStatus>;

    fn health_check_interval(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn health_check_timeout(&self) -> Duration {
        Duration::from_secs(5)
    }
}

// Remove redundant flags from metadata
pub struct ResourceMetadata {
    pub id: ResourceId,
    pub description: String,
    // Remove: poolable, health_checkable
    pub stateful: bool,
    pub dependencies: Vec<ResourceId>,
}
```

**Automatic metadata inference**:

```rust
impl ResourceManager {
    pub fn register<R>(&self, resource: R) -> ResourceResult<()>
    where
        R: Resource + 'static,
    {
        let mut metadata = resource.metadata();

        // Infer capabilities from trait implementations
        metadata.poolable = std::any::Any::type_id(&resource)
            == std::any::TypeId::of::<dyn PoolableResource>();

        // Or use specialization when stable:
        // metadata.poolable = R::IS_POOLABLE;

        // Store...
    }
}
```

#### Benefits

- **DRY**: Single source of truth
- **Type safety**: Can't mark as poolable without implementing trait
- **Clearer**: Capabilities expressed via traits
- **Automatic**: Metadata inferred from implementations

#### Migration Path

1. Create `PoolableResource` and `HealthCheckableResource` traits
2. Update existing impls to use new traits
3. Auto-populate metadata flags from trait detection
4. Deprecate manual metadata flags
5. Remove flags in next breaking release

---

## 4. Module Structure

### I-010: Split pool/mod.rs (580 LOC)

**Priority**: P2 (MEDIUM)
**Effort**: 6 hours
**Impact**: Single file is getting large and handles multiple concerns.

#### Problem

`pool/mod.rs` contains:
- `PoolStrategy` (35 lines)
- `PoolStats` (50 lines)
- `PoolEntry<T>` (120 lines)
- `ResourcePool<T>` (240 lines)
- `PooledResource<T>` (40 lines)
- `PoolManager` (95 lines)

Total: 580 lines in one file.

#### Current Code

```rust
// pool/mod.rs - everything in one file
pub enum PoolStrategy { /* ... */ }
pub struct PoolStats { /* ... */ }
struct PoolEntry<T> { /* ... */ }
pub struct ResourcePool<T> { /* ... */ }
pub struct PooledResource<T> { /* ... */ }
pub struct PoolManager { /* ... */ }
```

#### Proposed Solution

```
pool/
├── mod.rs              # Public API and re-exports (50 lines)
├── strategy.rs         # PoolStrategy enum and impls (60 lines)
├── stats.rs            # PoolStats and HealthCheckStats (80 lines)
├── entry.rs            # PoolEntry<T> internals (100 lines)
├── pool.rs             # ResourcePool<T> implementation (200 lines)
├── pooled_resource.rs  # PooledResource<T> wrapper (50 lines)
└── manager.rs          # PoolManager implementation (120 lines)
```

```rust
// pool/mod.rs
mod strategy;
mod stats;
mod entry;
mod pool;
mod pooled_resource;
mod manager;

pub use strategy::PoolStrategy;
pub use stats::{PoolStats, HealthCheckStats};
pub use pool::ResourcePool;
pub use pooled_resource::PooledResource;
pub use manager::PoolManager;

// Re-export PoolConfig from traits
pub use crate::core::traits::PoolConfig;
```

#### Benefits

- **Readability**: Easier to navigate
- **Maintainability**: Changes are localized
- **Testing**: Can test modules independently
- **Collaboration**: Reduces merge conflicts

#### Migration Path

1. Create new files
2. Move code maintaining same API
3. Update internal visibility (pub(crate) vs pub)
4. Ensure tests still pass
5. Update documentation

---

### I-011: Extract Credentials Module

**Priority**: P2 (MEDIUM)
**Effort**: 20 hours
**Impact**: Credential integration is planned but not implemented.

#### Problem

Feature flag exists but no implementation:

```toml
credentials = ["nebula-credential"]
```

No code in `src/credentials/` directory.

Resources with credentials would need:
- Credential retrieval
- Rotation handling
- Secure storage
- Audit logging

#### Proposed Solution

```
credentials/
├── mod.rs              # Public API
├── provider.rs         # CredentialProvider trait
├── manager.rs          # CredentialManager
├── rotation.rs         # Automatic rotation
├── cache.rs            # Secure caching
└── integration.rs      # nebula-credential integration
```

```rust
// credentials/provider.rs
#[async_trait]
pub trait CredentialProvider: Send + Sync {
    /// Get a credential by ID
    async fn get(&self, credential_id: &str) -> ResourceResult<Credential>;

    /// List available credentials
    async fn list(&self, filters: &CredentialFilters) -> ResourceResult<Vec<CredentialMetadata>>;

    /// Subscribe to credential rotation events
    fn subscribe_to_rotation(&self) -> tokio::sync::broadcast::Receiver<RotationEvent>;
}

// credentials/manager.rs
pub struct CredentialManager {
    provider: Arc<dyn CredentialProvider>,
    cache: Arc<CredentialCache>,
    rotation_handler: Arc<RotationHandler>,
}

impl CredentialManager {
    pub async fn get_for_resource(
        &self,
        resource_id: &ResourceId,
        credential_id: &str,
    ) -> ResourceResult<Credential> {
        // Check cache
        if let Some(cached) = self.cache.get(credential_id).await {
            if !cached.is_expired() {
                return Ok(cached);
            }
        }

        // Fetch from provider
        let credential = self.provider.get(credential_id).await?;

        // Cache
        self.cache.set(credential_id, credential.clone()).await;

        // Set up rotation if needed
        if credential.rotation_enabled {
            self.rotation_handler.schedule_rotation(
                credential_id,
                credential.rotation_interval,
            ).await;
        }

        Ok(credential)
    }
}

// credentials/rotation.rs
pub struct RotationHandler {
    scheduler: Arc<tokio::sync::Mutex<HashMap<String, JoinHandle<()>>>>,
}

impl RotationHandler {
    pub async fn schedule_rotation(&self, credential_id: &str, interval: Duration) {
        let credential_id = credential_id.to_string();
        let provider = self.provider.clone();

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;

                // Trigger rotation
                if let Err(e) = provider.rotate(&credential_id).await {
                    nebula_log::error!(
                        "Failed to rotate credential {}: {}",
                        credential_id,
                        e
                    );
                }
            }
        });

        self.scheduler.lock().await.insert(credential_id.to_string(), handle);
    }
}
```

**Resource integration**:

```rust
pub struct ResourceMetadata {
    pub required_credentials: Vec<String>,
    // ...
}

#[async_trait]
pub trait Resource {
    async fn create_with_credentials(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
        credentials: &CredentialMap,
    ) -> ResourceResult<Self::Instance>;
}
```

#### Benefits

- **Security**: Secure credential handling
- **Automation**: Rotation happens automatically
- **Auditability**: All credential access logged
- **Integration**: Works with nebula-credential

#### Migration Path

1. Create module structure
2. Define `CredentialProvider` trait
3. Implement `CredentialManager`
4. Add rotation logic
5. Integrate with Resource trait
6. Add nebula-credential adapter
7. Document usage patterns
8. Add examples

---

## 5. Performance Optimizations

### I-012: Use Parking Lot Consistently

**Priority**: P1 (HIGH)
**Effort**: 4 hours
**Impact**: Mix of std::sync and parking_lot creates inconsistency and misses performance gains.

#### Problem

Code uses both:

```rust
use std::sync::{Arc, Mutex};
use parking_lot::{RwLock, Mutex as ParkingLotMutex};
```

`parking_lot` is faster and already a dependency, but not used consistently.

#### Current Code

```rust
// manager/mod.rs
registry: Arc<RwLock<HashMap<...>>>, // parking_lot::RwLock ✓
instances: Arc<DashMap<...>>,         // Good

// pool/mod.rs
available: Arc<Mutex<Vec<...>>>,      // std::sync::Mutex ✗
acquired: Arc<Mutex<HashMap<...>>>,   // std::sync::Mutex ✗
stats: Arc<RwLock<PoolStats>>,        // parking_lot::RwLock ✓
```

#### Proposed Solution

**Standardize on parking_lot**:

```rust
// Everywhere:
use parking_lot::{Mutex, RwLock};
use std::sync::Arc; // Keep std Arc

// Update all code:
available: Arc<Mutex<Vec<PoolEntry<T>>>>,  // Now parking_lot
acquired: Arc<Mutex<HashMap<Uuid, PoolEntry<T>>>>, // Now parking_lot
```

**Benefits of parking_lot**:
- 1.5-2x faster than std in high contention
- Smaller memory footprint
- Better fairness
- No poisoning (simpler error handling)

#### Benefits

- **Performance**: 1.5-2x faster lock operations
- **Consistency**: One lock implementation
- **Simplicity**: No lock poisoning to handle
- **Memory**: Smaller lock overhead

#### Migration Path

1. Replace `use std::sync::Mutex` with `use parking_lot::Mutex`
2. Remove `.unwrap()` calls on lock (parking_lot doesn't poison)
3. Update error handling (no more PoisonError)
4. Run tests
5. Benchmark before/after

---

### I-013: Add Lock-Free Path for Metrics

**Priority**: P2 (MEDIUM)
**Effort**: 12 hours
**Impact**: Metrics updates acquire write locks on every operation - contention hotspot.

#### Problem

Every resource operation updates metrics behind a write lock:

```rust
pub struct ResourcePool<T> {
    stats: Arc<RwLock<PoolStats>>, // Write lock on every acquire/release!
}

pub async fn acquire(&self) -> ResourceResult<PooledResource<T>> {
    // ...
    {
        let mut stats = self.stats.write(); // ❌ Write lock
        stats.total_acquisitions += 1;
        stats.active_count += 1;
        stats.avg_acquisition_time_ms = /* ... */;
    }
    // ...
}
```

Under high load, this creates contention.

#### Current Code

```rust
pub struct PoolStats {
    pub total_acquisitions: u64,
    pub total_releases: u64,
    pub active_count: usize,
    // ...
}
```

#### Proposed Solution

**Use atomics for counters**:

```rust
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub struct PoolStats {
    // Lock-free counters
    pub total_acquisitions: AtomicU64,
    pub total_releases: AtomicU64,
    pub active_count: AtomicUsize,
    pub peak_active_count: AtomicUsize,
    pub failed_acquisitions: AtomicU64,
    pub resources_created: AtomicU64,
    pub resources_destroyed: AtomicU64,

    // For complex updates, keep a lock (but update less frequently)
    detailed_stats: parking_lot::Mutex<DetailedStats>,
}

struct DetailedStats {
    avg_acquisition_time_ms: f64,
    last_acquisition: Option<DateTime<Utc>>,
    health_checks: HealthCheckStats,
}

impl PoolStats {
    pub fn record_acquisition(&self, duration_ms: f64) {
        self.total_acquisitions.fetch_add(1, Ordering::Relaxed);
        self.active_count.fetch_add(1, Ordering::Relaxed);

        // Update peak (lock-free compare-and-swap loop)
        let current_active = self.active_count.load(Ordering::Relaxed);
        let mut peak = self.peak_active_count.load(Ordering::Relaxed);
        while current_active > peak {
            match self.peak_active_count.compare_exchange_weak(
                peak,
                current_active,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(new_peak) => peak = new_peak,
            }
        }

        // Update detailed stats less frequently (1% sample rate)
        if self.total_acquisitions.load(Ordering::Relaxed) % 100 == 0 {
            let mut detailed = self.detailed_stats.lock();
            detailed.avg_acquisition_time_ms =
                (detailed.avg_acquisition_time_ms * 99.0 + duration_ms) / 100.0;
            detailed.last_acquisition = Some(Utc::now());
        }
    }

    /// Snapshot for reporting (copies atomics)
    pub fn snapshot(&self) -> PoolStatsSnapshot {
        PoolStatsSnapshot {
            total_acquisitions: self.total_acquisitions.load(Ordering::Relaxed),
            total_releases: self.total_releases.load(Ordering::Relaxed),
            active_count: self.active_count.load(Ordering::Relaxed),
            peak_active_count: self.peak_active_count.load(Ordering::Relaxed),
            detailed: self.detailed_stats.lock().clone(),
        }
    }
}

/// Snapshot of stats at a point in time
#[derive(Clone, Debug)]
pub struct PoolStatsSnapshot {
    pub total_acquisitions: u64,
    pub total_releases: u64,
    pub active_count: usize,
    pub peak_active_count: usize,
    pub detailed: DetailedStats,
}
```

#### Benefits

- **Performance**: No lock contention on hot path
- **Scalability**: Linear scaling with cores
- **Accuracy**: Counters always accurate
- **Simplicity**: Relaxed ordering is fine for metrics

#### Migration Path

1. Create new `PoolStats` with atomics
2. Add `record_acquisition()`, `record_release()` methods
3. Update pool code to use new methods
4. Add `snapshot()` for reporting
5. Benchmark before/after
6. Remove old implementation

---

### I-014: Optimize Scope Key Generation

**Priority**: P3 (LOW)
**Effort**: 4 hours
**Impact**: Scope keys are generated frequently with string allocations.

#### Problem

```rust
fn build_instance_key(&self, resource_id: &ResourceId, scope: &ResourceScope) -> String {
    format!("{}:{}", resource_id.unique_key(), scope.scope_key())
    // ^^^ Allocates new String on every call
}
```

This is called on every `get()` operation.

#### Current Code

```rust
pub fn scope_key(&self) -> String {
    match self {
        Self::Global => "global".to_string(),
        Self::Tenant { tenant_id } => format!("tenant:{}", tenant_id),
        Self::Workflow { workflow_id } => format!("workflow:{}", workflow_id),
        // ...
    }
}
```

#### Proposed Solution

**Cache scope keys**:

```rust
pub enum ResourceScope {
    Global,
    Tenant {
        tenant_id: String,
        cached_key: OnceCell<String>, // Lazy cache
    },
    Workflow {
        workflow_id: String,
        cached_key: OnceCell<String>,
    },
    // ...
}

impl ResourceScope {
    pub fn scope_key(&self) -> &str {
        match self {
            Self::Global => "global",
            Self::Tenant { tenant_id, cached_key } => {
                cached_key.get_or_init(|| format!("tenant:{}", tenant_id))
            }
            Self::Workflow { workflow_id, cached_key } => {
                cached_key.get_or_init(|| format!("workflow:{}", workflow_id))
            }
            // ...
        }
    }
}
```

**Or use Cow**:

```rust
pub fn scope_key(&self) -> Cow<'static, str> {
    match self {
        Self::Global => Cow::Borrowed("global"),
        Self::Tenant { tenant_id } => Cow::Owned(format!("tenant:{}", tenant_id)),
        // ...
    }
}
```

#### Benefits

- **Performance**: Avoid repeated allocations
- **Memory**: Reuse cached strings
- **API**: Same interface

#### Migration Path

1. Add `OnceCell` or use `Cow`
2. Update `scope_key()` to cache/borrow
3. Update callers if signature changes
4. Benchmark

---

## 6. Error Handling

### I-015: Add Error Context Chain

**Priority**: P1 (HIGH)
**Effort**: 10 hours
**Impact**: Hard to debug errors because context is lost in nested calls.

#### Problem

Errors lose context as they propagate:

```rust
// Deep in the code
async fn connect_to_database(&self) -> Result<Connection> {
    Err(io::Error::new(ErrorKind::ConnectionRefused, "Connection refused"))
}

// Bubbles up losing context
async fn create_instance(&self) -> ResourceResult<Instance> {
    let conn = self.connect_to_database().await?;
    //                                          ^^^ Where did this fail? Why?
}
```

User sees: `"Initialization failed for resource 'postgres:v1': Connection refused"`

But doesn't know:
- Which host/port failed?
- What was the configuration?
- What retry attempt was this?

#### Current Code

```rust
pub enum ResourceError {
    Initialization {
        resource_id: String,
        reason: String,
        source: Option<Box<dyn Error + Send + Sync>>,
    },
    // ...
}

// Usage loses context
resource.create(&config, &ctx)
    .await
    .map_err(|e| ResourceError::initialization("postgres", e.to_string()))?;
    //                                                     ^^^ Lost all detail!
```

#### Proposed Solution

**Add context-preserving error handling**:

```rust
use std::backtrace::Backtrace;

pub struct ResourceError {
    kind: ResourceErrorKind,
    context: Vec<ErrorContext>,
    backtrace: Option<Backtrace>,
}

pub enum ResourceErrorKind {
    Configuration { message: String },
    Initialization { resource_id: String, reason: String },
    Unavailable { resource_id: String, reason: String, retryable: bool },
    // ...
}

pub struct ErrorContext {
    location: &'static str,
    message: String,
    metadata: HashMap<String, serde_json::Value>,
}

impl ResourceError {
    /// Add context to an error
    pub fn context<S: Into<String>>(mut self, location: &'static str, message: S) -> Self {
        self.context.push(ErrorContext {
            location,
            message: message.into(),
            metadata: HashMap::new(),
        });
        self
    }

    /// Add structured metadata
    pub fn with_metadata<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<serde_json::Value>,
    {
        if let Some(ctx) = self.context.last_mut() {
            ctx.metadata.insert(key.into(), value.into());
        }
        self
    }

    /// Pretty-print error chain
    pub fn display_chain(&self) -> String {
        let mut output = format!("{}\n", self.kind);

        if !self.context.is_empty() {
            output.push_str("\nContext chain (most recent first):\n");
            for (i, ctx) in self.context.iter().rev().enumerate() {
                output.push_str(&format!(
                    "  {}. {} at {}\n",
                    i + 1,
                    ctx.message,
                    ctx.location
                ));

                if !ctx.metadata.is_empty() {
                    for (k, v) in &ctx.metadata {
                        output.push_str(&format!("     - {}: {}\n", k, v));
                    }
                }
            }
        }

        if let Some(ref bt) = self.backtrace {
            output.push_str(&format!("\nBacktrace:\n{}", bt));
        }

        output
    }
}

// Usage:
async fn create_instance(&self, config: &Config) -> ResourceResult<Instance> {
    let conn = self.connect_to_database(&config.host, config.port)
        .await
        .map_err(|e| {
            ResourceError::from(e)
                .context(
                    "resource::database::create_instance",
                    "Failed to connect to database"
                )
                .with_metadata("host", config.host.clone())
                .with_metadata("port", config.port)
                .with_metadata("attempt", self.attempt_count)
        })?;

    Ok(Instance { conn })
}
```

**Output example**:

```
Initialization failed for resource 'postgres:v1': Connection refused

Context chain (most recent first):
  1. Failed to connect to database at resource::database::create_instance
     - host: "localhost"
     - port: 5432
     - attempt: 3
  2. Pool exhausted, creating new connection at pool::acquire
     - pool_size: 10
     - max_size: 10
     - waiters: 5
  3. Resource acquisition at manager::get_by_id
     - scope: "workflow:etl-pipeline-123"
     - timeout_ms: 30000

Backtrace:
  [... detailed backtrace ...]
```

#### Benefits

- **Debuggability**: Full context preserved
- **Observability**: Can log structured context
- **User experience**: Clear error messages
- **Troubleshooting**: Know exactly what failed and why

#### Migration Path

1. Add `context` field to ResourceError
2. Add `context()` and `with_metadata()` methods
3. Update error creation sites to add context
4. Add `display_chain()` for pretty printing
5. Update logging to use error context
6. Add examples of good error handling

---

### I-016: Add Retryable Error Classification

**Priority**: P2 (MEDIUM)
**Effort**: 6 hours
**Impact**: Some errors have retryable flag, but not used consistently.

#### Problem

`is_retryable()` method exists but:
- Not all error variants are classified
- No retry metadata (how many times, backoff strategy)
- Retry logic not centralized

```rust
pub fn is_retryable(&self) -> bool {
    match self {
        Self::Unavailable { retryable, .. } => *retryable,
        Self::Timeout { .. } => true,
        Self::PoolExhausted { .. } => true,
        Self::CircuitBreakerOpen { .. } => true,
        _ => false, // Everything else is not retryable?
    }
}
```

#### Proposed Solution

**Add retry metadata and strategy**:

```rust
#[derive(Debug, Clone)]
pub struct RetryMetadata {
    /// Is this error retryable?
    pub retryable: bool,
    /// Recommended retry strategy
    pub strategy: RetryStrategy,
    /// Maximum retry attempts
    pub max_attempts: u32,
    /// Current attempt number (if known)
    pub current_attempt: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum RetryStrategy {
    /// Retry immediately
    Immediate,
    /// Fixed delay between retries
    Fixed { delay: Duration },
    /// Exponential backoff
    Exponential {
        initial_delay: Duration,
        max_delay: Duration,
        multiplier: f64,
    },
    /// Exponential with jitter
    ExponentialJitter {
        initial_delay: Duration,
        max_delay: Duration,
        multiplier: f64,
    },
    /// Custom function
    Custom(Arc<dyn Fn(u32) -> Duration + Send + Sync>),
}

impl ResourceError {
    /// Get retry metadata for this error
    pub fn retry_metadata(&self) -> RetryMetadata {
        match self {
            Self::Unavailable { retryable, .. } => RetryMetadata {
                retryable: *retryable,
                strategy: RetryStrategy::ExponentialJitter {
                    initial_delay: Duration::from_millis(100),
                    max_delay: Duration::from_secs(30),
                    multiplier: 2.0,
                },
                max_attempts: 5,
                current_attempt: None,
            },

            Self::Timeout { .. } => RetryMetadata {
                retryable: true,
                strategy: RetryStrategy::Exponential {
                    initial_delay: Duration::from_secs(1),
                    max_delay: Duration::from_secs(60),
                    multiplier: 2.0,
                },
                max_attempts: 3,
                current_attempt: None,
            },

            Self::PoolExhausted { waiters, .. } => RetryMetadata {
                retryable: true,
                strategy: if *waiters > 10 {
                    // Many waiters - backoff aggressively
                    RetryStrategy::ExponentialJitter {
                        initial_delay: Duration::from_secs(1),
                        max_delay: Duration::from_secs(30),
                        multiplier: 3.0,
                    }
                } else {
                    // Few waiters - retry quickly
                    RetryStrategy::Fixed {
                        delay: Duration::from_millis(100),
                    }
                },
                max_attempts: 10,
                current_attempt: None,
            },

            Self::CircuitBreakerOpen { retry_after_ms, .. } => RetryMetadata {
                retryable: true,
                strategy: RetryStrategy::Fixed {
                    delay: retry_after_ms
                        .map(Duration::from_millis)
                        .unwrap_or(Duration::from_secs(30)),
                },
                max_attempts: 1,
                current_attempt: None,
            },

            Self::HealthCheck { attempt, .. } => RetryMetadata {
                retryable: true,
                strategy: RetryStrategy::ExponentialJitter {
                    initial_delay: Duration::from_secs(5),
                    max_delay: Duration::from_secs(300),
                    multiplier: 2.0,
                },
                max_attempts: 5,
                current_attempt: Some(*attempt),
            },

            // Not retryable
            Self::Configuration { .. }
            | Self::CircularDependency { .. }
            | Self::InvalidStateTransition { .. } => RetryMetadata {
                retryable: false,
                strategy: RetryStrategy::Immediate,
                max_attempts: 0,
                current_attempt: None,
            },

            _ => RetryMetadata::default(),
        }
    }

    /// Calculate delay for next retry attempt
    pub fn calculate_retry_delay(&self, attempt: u32) -> Option<Duration> {
        let metadata = self.retry_metadata();

        if !metadata.retryable || attempt >= metadata.max_attempts {
            return None;
        }

        Some(match metadata.strategy {
            RetryStrategy::Immediate => Duration::ZERO,
            RetryStrategy::Fixed { delay } => delay,
            RetryStrategy::Exponential { initial_delay, max_delay, multiplier } => {
                let delay = initial_delay.as_secs_f64() * multiplier.powi(attempt as i32);
                Duration::from_secs_f64(delay.min(max_delay.as_secs_f64()))
            }
            RetryStrategy::ExponentialJitter { initial_delay, max_delay, multiplier } => {
                let delay = initial_delay.as_secs_f64() * multiplier.powi(attempt as i32);
                let jitter = rand::thread_rng().gen_range(0.0..=0.3); // 0-30% jitter
                let delay_with_jitter = delay * (1.0 + jitter);
                Duration::from_secs_f64(delay_with_jitter.min(max_delay.as_secs_f64()))
            }
            RetryStrategy::Custom(ref f) => f(attempt),
        })
    }
}
```

#### Benefits

- **Smart retries**: Context-aware backoff
- **Predictable**: Clear retry behavior
- **Configurable**: Can customize per error type
- **Observable**: Can log retry attempts with metadata

#### Migration Path

1. Add `RetryMetadata` and `RetryStrategy` types
2. Implement `retry_metadata()` for all error variants
3. Add `calculate_retry_delay()` helper
4. Update manager to use retry metadata
5. Add retry decorator/middleware
6. Document retry behavior

---

## 7. Concurrency Patterns

### I-017: Add Proper Async Drop via Cleanup Tasks

**Priority**: P0 (CRITICAL)
**Effort**: 12 hours
**Impact**: Drop handlers can't be async, leading to resource leaks and incomplete cleanup.

(See I-003 and I-004 for detailed solutions)

**Summary**: Use cleanup channels + background tasks to handle async cleanup properly.

---

### I-018: Add Timeout Wrapper for All Async Operations

**Priority**: P1 (HIGH)
**Effort**: 8 hours
**Impact**: No timeouts on resource operations can cause indefinite hangs.

#### Problem

Resource operations can hang indefinitely:

```rust
// Can hang forever if database is unresponsive
let instance = resource.create(&config, &ctx).await?;
```

No timeout enforcement at the framework level.

#### Proposed Solution

**Timeout wrapper**:

```rust
use tokio::time::{timeout, Duration};

impl ResourceManager {
    async fn create_instance_with_timeout<T>(
        &self,
        resource_id: &ResourceId,
        context: &ResourceContext,
        operation_timeout: Duration,
    ) -> ResourceResult<ResourceGuard<T>>
    where
        T: Send + Sync + 'static,
    {
        timeout(
            operation_timeout,
            self.create_instance(resource_id, context)
        )
        .await
        .map_err(|_| {
            ResourceError::timeout(
                resource_id.unique_key(),
                operation_timeout.as_millis() as u64,
                "create_instance",
            )
        })?
    }

    pub async fn get_by_id<T>(
        &self,
        resource_id: &ResourceId,
        context: &ResourceContext,
    ) -> ResourceResult<ResourceGuard<T>>
    where
        T: Send + Sync + 'static,
    {
        let operation_timeout = self.config.default_timeout;

        self.create_instance_with_timeout(resource_id, context, operation_timeout)
            .await
    }
}
```

**With cancellation token for cleanup**:

```rust
use tokio_util::sync::CancellationToken;

impl ResourceManager {
    async fn with_timeout_and_cleanup<F, T>(
        &self,
        operation: F,
        timeout_duration: Duration,
        cleanup: impl FnOnce() + Send + 'static,
    ) -> ResourceResult<T>
    where
        F: Future<Output = ResourceResult<T>>,
    {
        let cancel_token = CancellationToken::new();
        let cancel_token_clone = cancel_token.clone();

        let result = tokio::select! {
            result = operation => result,
            _ = tokio::time::sleep(timeout_duration) => {
                // Timeout occurred
                cancel_token.cancel();

                // Spawn cleanup task
                tokio::spawn(async move {
                    cleanup();
                });

                Err(ResourceError::timeout(
                    "operation",
                    timeout_duration.as_millis() as u64,
                    "resource_operation",
                ))
            }
        };

        result
    }
}
```

#### Benefits

- **Reliability**: No indefinite hangs
- **Observability**: Timeout errors are clear
- **Resource cleanup**: Partial work is cleaned up
- **User control**: Configurable timeouts

#### Migration Path

1. Add timeout helpers
2. Wrap all async resource operations
3. Add per-resource timeout configuration
4. Add cleanup for timed-out operations
5. Document timeout behavior
6. Add metrics for timeout frequency

---

## 8. Safety & Correctness

### I-019: Remove Unsafe Code in Testing

**Priority**: P0 (CRITICAL)
**Effort**: 6 hours
**Impact**: `unsafe { std::mem::zeroed() }` in tests is a critical safety violation.

#### Problem

```rust
// testing/mod.rs:70
pub async fn get_mock<T>(&self, resource_id: &str) -> ResourceResult<Arc<T>>
where
    T: Send + Sync + 'static,
{
    // ...
    Ok(Arc::new(unsafe { std::mem::zeroed() }))
    //           ^^^^^^^^^^^^^^^^^^^^^^^^^ CRITICAL BUG!
}
```

This creates undefined behavior:
- Zero-initialized types may be invalid (like NonZeroU*, references, etc.)
- Can cause immediate segfaults
- Violates memory safety guarantees

#### Current Code

```rust
// This is catastrophically wrong:
let mock: Arc<DatabaseConnection> = test_manager.get_mock("db").await?;
// mock contains zero-initialized memory - likely crashes when used!
```

#### Proposed Solution

**Option 1: Use proper mocking** (Recommended):

```rust
use std::any::Any;

pub struct TestResourceManager {
    resources: Arc<Mutex<HashMap<String, Arc<dyn Any + Send + Sync>>>>,
    call_history: Arc<Mutex<Vec<ResourceCall>>>,
}

impl TestResourceManager {
    /// Register a pre-constructed mock instance
    pub fn register_mock_instance<T>(&self, resource_id: String, instance: T)
    where
        T: Send + Sync + 'static,
    {
        let mut resources = self.resources.lock().unwrap();
        resources.insert(resource_id, Arc::new(instance));
    }

    /// Get a mock instance (must be registered first)
    pub async fn get_mock<T>(&self, resource_id: &str) -> ResourceResult<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        let resources = self.resources.lock().unwrap();
        let any_resource = resources
            .get(resource_id)
            .ok_or_else(|| {
                ResourceError::unavailable(
                    resource_id,
                    "Mock resource not registered. Call register_mock_instance() first.",
                    false
                )
            })?;

        // Safe downcast
        any_resource
            .clone()
            .downcast::<T>()
            .map_err(|_| {
                ResourceError::internal(
                    resource_id,
                    format!(
                        "Mock resource type mismatch. Expected {}, got different type.",
                        std::any::type_name::<T>()
                    )
                )
            })
    }
}

// Usage:
let test_manager = TestResourceManager::new();

// Create a real mock instance
let mock_db = MockDatabase::new();
test_manager.register_mock_instance("db", mock_db);

// Get it back (safe!)
let db: Arc<MockDatabase> = test_manager.get_mock("db").await?;
```

**Option 2: Use mockall** (if feature enabled):

```rust
#[cfg(feature = "mockall")]
use mockall::mock;

#[cfg(feature = "mockall")]
mock! {
    pub Database {}

    impl Database {
        pub async fn query(&self, sql: &str) -> Result<Vec<Row>>;
        pub async fn execute(&self, sql: &str) -> Result<u64>;
    }
}

impl TestResourceManager {
    #[cfg(feature = "mockall")]
    pub fn create_mock<T>(&self) -> T
    where
        T: mockall::Mock,
    {
        T::default()
    }
}
```

#### Benefits

- **Safety**: No undefined behavior
- **Correctness**: Mocks actually work
- **Testing**: Can verify mock behavior
- **Type safety**: Proper type checking

#### Migration Path

1. Remove unsafe code immediately
2. Add proper mock registration API
3. Update tests to register mocks
4. Consider adding mockall integration
5. Document testing patterns
6. Add test for type mismatch errors

---

### I-020: Fix ResourceInstance::touch(&self) Interior Mutability

**Priority**: P1 (HIGH)
**Effort**: 4 hours
**Impact**: `touch()` claims to use interior mutability but signature doesn't enforce it.

#### Problem

```rust
pub trait ResourceInstance: Send + Sync + Any {
    /// Update last access timestamp
    ///
    /// Uses interior mutability to update timestamp without requiring &mut self
    fn touch(&self);
    //       ^^^^^ Says it uses interior mutability, but how?
}
```

Implementations currently use `&mut self`:

```rust
impl ResourceInstance for TestInstance {
    fn touch(&mut self) {} // ❌ Requires mutable borrow, not interior mutability
    //       ^^^^^^^^
}
```

#### Proposed Solution

**Enforce interior mutability**:

```rust
use std::sync::atomic::{AtomicI64, Ordering};
use std::cell::Cell;

pub trait ResourceInstance: Send + Sync + Any {
    fn instance_id(&self) -> Uuid;
    fn resource_id(&self) -> &ResourceId;
    fn lifecycle_state(&self) -> LifecycleState;
    fn context(&self) -> &ResourceContext;
    fn created_at(&self) -> DateTime<Utc>;

    /// Get last access timestamp (atomic read)
    fn last_accessed_at(&self) -> Option<DateTime<Utc>>;

    /// Update last access timestamp (uses interior mutability)
    fn touch(&self); // ← Keep &self
}

// Example implementation with interior mutability:
pub struct DefaultResourceInstance<T> {
    instance_id: Uuid,
    resource_id: ResourceId,
    inner: Arc<T>,
    created_at: DateTime<Utc>,

    // Interior mutability for timestamp
    last_accessed_at: Arc<Mutex<Option<DateTime<Utc>>>>,

    // Or use atomic i64 for timestamp
    last_accessed_unix: AtomicI64,
}

impl<T> ResourceInstance for DefaultResourceInstance<T>
where
    T: Send + Sync + Any,
{
    fn touch(&self) {
        // Option 1: Use Mutex (simpler, works with DateTime)
        *self.last_accessed_at.lock() = Some(Utc::now());

        // Option 2: Use atomic (faster, but need to convert DateTime)
        let now = Utc::now().timestamp_millis();
        self.last_accessed_unix.store(now, Ordering::Release);
    }

    fn last_accessed_at(&self) -> Option<DateTime<Utc>> {
        // Option 1: Mutex
        *self.last_accessed_at.lock()

        // Option 2: Atomic
        let unix_ms = self.last_accessed_unix.load(Ordering::Acquire);
        if unix_ms == 0 {
            None
        } else {
            Some(DateTime::from_timestamp_millis(unix_ms).unwrap())
        }
    }
}
```

#### Benefits

- **Correctness**: Actually uses interior mutability as claimed
- **Usability**: Can touch from shared references
- **Thread-safety**: Atomic or mutex ensures correctness
- **API**: Matches documentation

#### Migration Path

1. Add interior mutability to implementations
2. Update `TypedResourceInstance` to use atomics or mutex
3. Fix test implementations
4. Verify thread safety
5. Document pattern

---

## 9. Testing Architecture

### I-021: Add Property-Based Testing

**Priority**: P2 (MEDIUM)
**Effort**: 16 hours
**Impact**: Current tests are example-based; property tests would catch edge cases.

#### Problem

Tests are all example-based:

```rust
#[tokio::test]
async fn test_pool_acquire_and_release() {
    let pool = ResourcePool::new(/* ... */);
    let resource = pool.acquire().await.unwrap();
    // Test one specific case
}
```

Missing:
- Edge cases
- Invariant checking
- Concurrency bugs
- Resource exhaustion scenarios

#### Proposed Solution

**Add proptest-based property tests**:

```rust
use proptest::prelude::*;
use tokio::test;

proptest! {
    #[test]
    fn pool_never_exceeds_max_size(
        max_size in 1usize..100,
        num_acquisitions in 0usize..200,
    ) {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let config = PoolConfig {
                min_size: 0,
                max_size,
                ..Default::default()
            };

            let pool = ResourcePool::new(config, PoolStrategy::Fifo, || async {
                Ok(TypedResourceInstance::new(Arc::new("test"), /* metadata */))
            });

            let mut handles = vec![];
            for _ in 0..num_acquisitions {
                let pool_clone = pool.clone();
                let handle = tokio::spawn(async move {
                    pool_clone.acquire().await
                });
                handles.push(handle);
            }

            // Collect results
            let results: Vec<_> = futures::future::join_all(handles).await;
            let acquired: Vec<_> = results.into_iter()
                .filter_map(|r| r.ok())
                .filter_map(|r| r.ok())
                .collect();

            // Property: Can never have more than max_size acquired
            prop_assert!(acquired.len() <= max_size);

            // Property: Stats are consistent
            let stats = pool.stats();
            prop_assert_eq!(stats.active_count, acquired.len());
        });
    }

    #[test]
    fn resource_context_serialization_roundtrip(
        workflow_id in "[a-z]{5,20}",
        execution_id in "[a-z]{5,20}",
    ) {
        let ctx = ResourceContext::new(
            workflow_id.clone(),
            "Test".to_string(),
            execution_id.clone(),
            "test".to_string(),
        );

        // Property: Serialize then deserialize = identity
        let json = serde_json::to_string(&ctx).unwrap();
        let deserialized: ResourceContext = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(ctx.workflow.workflow_id, deserialized.workflow.workflow_id);
        prop_assert_eq!(ctx.execution.execution_id, deserialized.execution.execution_id);
    }

    #[test]
    fn scope_hierarchy_is_transitive(
        tenant_id in "[a-z]{5,10}",
        workflow_id in "[a-z]{5,10}",
        execution_id in "[a-z]{5,10}",
    ) {
        let global = ResourceScope::Global;
        let tenant = ResourceScope::tenant(tenant_id);
        let workflow = ResourceScope::workflow(workflow_id);
        let execution = ResourceScope::execution(execution_id);

        // Property: Scope hierarchy is transitive
        prop_assert!(execution.contains(&execution));
        prop_assert!(workflow.contains(&execution));
        prop_assert!(tenant.contains(&workflow));
        prop_assert!(tenant.contains(&execution));
        prop_assert!(global.contains(&tenant));
        prop_assert!(global.contains(&execution));
    }
}
```

#### Benefits

- **Coverage**: Tests edge cases automatically
- **Invariants**: Verifies properties always hold
- **Regression**: Generates failing cases for bugs
- **Confidence**: Higher assurance of correctness

#### Migration Path

1. Add proptest dependency
2. Identify key invariants to test
3. Write property tests for core modules
4. Run with CI (may be slow)
5. Document properties being tested

---

## 10. Dependencies Cleanup

### I-022: Remove Unused Dependencies

**Priority**: P2 (MEDIUM)
**Effort**: 2 hours
**Impact**: Cleaner Cargo.toml, faster builds, smaller binaries.

#### Problem

Several dependencies are imported but never used:

```toml
deadpool = { version = "0.10", optional = true }  # ❌ Never used
bb8 = { version = "0.8", optional = true }        # ❌ Never used
arc-swap = "1.6"                                  # ❌ Never used
tokio-util = { version = "0.7", optional = true } # ❌ Never used
```

These add:
- Build time
- Binary size
- Dependency audit surface
- Confusion about whether they're actually used

#### Proposed Solution

**Remove unused dependencies**:

```toml
# Remove these:
# deadpool = { version = "0.10", optional = true }
# bb8 = { version = "0.8", optional = true }
# arc-swap = "1.6"
# tokio-util = { version = "0.7", optional = true }

# Remove from features:
[features]
# pooling = ["dep:deadpool", "dep:bb8"]  # Remove - we have our own pool
```

**If we want to use them later**:

Add them when actually implementing:
- `deadpool` - only if we decide to use it as pool backend
- `bb8` - only if we decide to use it as pool backend
- `arc-swap` - only if we implement lock-free updates (see I-013)

#### Benefits

- **Build speed**: Fewer dependencies to compile
- **Binary size**: Smaller output
- **Clarity**: Only shows what's actually used
- **Security**: Smaller attack surface

#### Migration Path

1. Search codebase for usage
2. Remove if unused
3. Update features
4. Verify build works
5. Update docs

---

## 11. Implementation Order

### Phase 1: Critical Fixes (Week 1-2)

**Must fix before any production use**:

1. **I-019**: Remove unsafe code in testing (6h) - **Safety**
2. **I-001**: Fix Manager-Pool integration (24h) - **Correctness**
3. **I-003**: Fix ResourceGuard Drop (8h) - **Correctness**
4. **I-004**: Add async drop pattern (12h) - **Correctness**

**Total**: ~50 hours, ~1.5 weeks

---

### Phase 2: High-Impact Improvements (Week 3-5)

**Significant improvements to usability and performance**:

5. **I-002**: Remove type erasure smell (16h)
6. **I-008**: Split Resource trait (12h)
7. **I-012**: Use parking_lot consistently (4h)
8. **I-015**: Add error context chain (10h)
9. **I-018**: Add timeout wrapper (8h)
10. **I-020**: Fix touch() interior mutability (4h)

**Total**: ~54 hours, ~1.5 weeks

---

### Phase 3: Polish & Optimization (Week 6-8)

**Quality of life and performance**:

11. **I-005**: Improve ResourceContext overhead (10h)
12. **I-009**: Unify traits (8h)
13. **I-013**: Lock-free metrics (12h)
14. **I-016**: Retryable error classification (6h)
15. **I-010**: Split pool module (6h)
16. **I-022**: Remove unused deps (2h)

**Total**: ~44 hours, ~1.5 weeks

---

### Phase 4: Advanced Features (Week 9-12)

**New capabilities**:

17. **I-011**: Extract credentials module (20h)
18. **I-006**: Add resource type states (20h)
19. **I-007**: Add GAT-based trait (16h)
20. **I-021**: Property-based testing (16h)
21. **I-014**: Optimize scope keys (4h)

**Total**: ~76 hours, ~2.5 weeks

---

## Appendix: Design Alternatives Considered

### A1: Manager-Pool Integration Alternatives

**Option A: Manager delegates to pools** ✅ **CHOSEN**
- Pros: Clean separation, pools are authoritative
- Cons: More indirection

**Option B: Manager owns instances, pools are cache**
- Pros: Simpler ownership
- Cons: Defeats pooling purpose, confusing semantics

**Option C: Remove Manager, pools are primary API**
- Pros: Simpler architecture
- Cons: Loses metadata, registration, dependency resolution

**Decision**: Option A preserves best of both worlds.

---

### A2: Type Erasure Solutions

**Option A: TypeId → ResourceId map** ✅ **CHOSEN**
- Pros: Type-safe, O(1), detects conflicts
- Cons: Requires 'static instances

**Option B: Keep string matching, add validation**
- Pros: Simple
- Cons: Still fragile, runtime errors

**Option C: Macro-generated registry**
- Pros: Compile-time checking
- Cons: Complex, inflexible

**Decision**: Option A balances safety and simplicity.

---

### A3: Async Drop Patterns

**Option A: Cleanup channel + background task** ✅ **CHOSEN**
- Pros: No blocking, reliable
- Cons: Slight delay before cleanup

**Option B: Explicit release() only**
- Pros: Clear semantics
- Cons: Easy to forget, leaks resources

**Option C: block_on in Drop**
- Pros: Immediate cleanup
- Cons: Can panic, blocks executor

**Decision**: Option A is safest and most reliable.

---

### A4: Error Context Approaches

**Option A: Context chain with metadata** ✅ **CHOSEN**
- Pros: Rich context, structured
- Cons: More complex error type

**Option B: anyhow-style simple context**
- Pros: Simple API
- Cons: Less structured

**Option C: Keep current approach**
- Pros: No changes needed
- Cons: Poor debuggability

**Decision**: Option A significantly improves debuggability.

---

## Summary

This improvement plan addresses **28 architectural issues** across 10 categories, prioritized by impact and effort. The most critical insight is that the Manager-Pool integration is fundamentally broken and must be fixed before production use.

**Top 5 Critical Improvements**:
1. Fix Manager-Pool integration (I-001)
2. Remove unsafe code (I-019)
3. Fix async drop pattern (I-003, I-004)
4. Add proper error context (I-015)
5. Fix type erasure (I-002)

**Top 5 Quick Wins** (< 8 hours each):
1. Use parking_lot consistently (I-012) - 4h
2. Remove unused deps (I-022) - 2h
3. Fix touch() signature (I-020) - 4h
4. Optimize scope keys (I-014) - 4h
5. Remove unsafe testing code (I-019) - 6h

**Implementation Timeline**: ~224 hours total (~8 weeks with one developer)

The crate has excellent architectural foundations but requires these improvements to reach production quality. Most changes are backward-compatible or can be phased in gradually.
