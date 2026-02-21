# ResourceRef and ResourceProvider

## Overview

Added type-safe resource references and a provider trait to `nebula-resource` for decoupling resource acquisition from the concrete `Manager` implementation.

## New Types

### `ResourceRef`

Type-safe reference to a resource that wraps a `TypeId`.

```rust
use nebula_resource::ResourceRef;
use std::any::TypeId;

struct PostgresConnection;

// Create a type-safe reference
const DB_REF: ResourceRef = ResourceRef::of::<PostgresConnection>();

// Check type ID
assert_eq!(DB_REF.type_id(), TypeId::of::<PostgresConnection>());
```

**Key Features:**
- **Pure TypeId**: Just wraps `TypeId` - no string IDs
- **Type-safe**: Ensures compile-time and runtime type checking
- **Const-friendly**: Can be used in const contexts
- **Zero-cost**: Single `TypeId` field (no overhead)
- **Implements**: `Clone`, `Copy`, `PartialEq`, `Eq`, `Hash`

**Design Philosophy:**
- No string IDs - type is the identifier
- Simple newtype wrapper around `TypeId`
- Enables both type-based and ID-based resource acquisition strategies

### `ResourceProvider`

Trait for acquiring resources with two methods: type-safe and string-based.

```rust
pub trait ResourceProvider: Send + Sync {
    // Type-safe: acquire by Resource type
    async fn resource<R: Resource>(&self, ctx: &Context) -> Result<Guard<R::Instance>>;

    // Dynamic: acquire by string ID
    async fn acquire(&self, id: &str, ctx: &Context) -> Result<Box<dyn Any + Send>>;

    // Convenience methods
    async fn has_resource<R: Resource>(&self, ctx: &Context) -> bool;
    async fn has(&self, id: &str, ctx: &Context) -> bool;
}
```

**Two Acquisition Strategies:**

1. **Type-based (preferred):**
```rust
// Define resource
struct PostgresResource;
impl Resource for PostgresResource {
    type Instance = PgConnection;
    fn id() -> &'static str { "postgres" }
}

// Acquire it
let conn = provider.resource::<PostgresResource>(&ctx).await?;
```

2. **ID-based (dynamic):**
```rust
let any = provider.acquire("postgres", &ctx).await?;
let guard = any.downcast::<Guard<PgConnection>>().unwrap();
```

## Usage in Triggers

Trigger contexts can now hold a `ResourceProvider`:

```rust
use nebula_resource::{ResourceProvider, Resource};

pub struct TriggerContext {
    pub trigger_id: Uuid,
    pub resource_context: nebula_resource::Context,
    resource_provider: Arc<dyn ResourceProvider>,
}

impl TriggerContext {
    // Type-safe acquisition
    pub async fn get_resource<R: Resource>(&self) -> Result<Guard<R::Instance>> {
        self.resource_provider
            .resource::<R>(&self.resource_context)
            .await
    }
    
    // Dynamic acquisition
    pub async fn get_by_id(&self, id: &str) -> Result<Box<dyn Any + Send>> {
        self.resource_provider
            .acquire(id, &self.resource_context)
            .await
    }
}
```

## Benefits

1. **Dual API**: Type-safe (`resource<R>()`) + dynamic (`acquire(id)`)
2. **No Circular Dependencies**: Clean separation between crates
3. **Type Safety**: `ResourceRef` enforces type correctness
4. **Flexibility**: Manager can map TypeId → Pool or ID → Pool
5. **Modern Rust**: Native async fn in traits (Rust 1.75+)

## Implementation Strategy

Manager can support both approaches:

```rust
struct Manager {
    pools_by_id: DashMap<String, Arc<Pool>>,        // String-based
    pools_by_type: DashMap<TypeId, Arc<Pool>>,      // Type-based
}

impl ResourceProvider for Manager {
    async fn resource<R: Resource>(&self, ctx: &Context) -> Result<Guard<R::Instance>> {
        let type_id = TypeId::of::<R>();
        let pool = self.pools_by_type.get(&type_id)?;
        pool.acquire(ctx).await
    }

    async fn acquire(&self, id: &str, ctx: &Context) -> Result<Box<dyn Any + Send>> {
        let pool = self.pools_by_id.get(id)?;
        Ok(Box::new(pool.acquire(ctx).await?))
    }
}
```

## File Structure

```
crates/resource/src/
├── lib.rs           # Exports ResourceRef and ResourceProvider
├── reference.rs     # ResourceRef + ResourceProvider trait
├── manager.rs       # Manager implements ResourceProvider
└── context.rs       # Context used by provider
```

## Migration Notes

- `ResourceRef` is now just `ResourceRef::of::<T>()` - no string ID needed
- Use `resource<R>()` for type-safe acquisition
- Use `acquire(id)` for dynamic acquisition when type unknown at compile time
- No `async-trait` dependency - native async fn in traits

## Future Work

- Implement `ResourceProvider` for `Manager` with dual registry (TypeId + String)
- Consider registration API: `register_typed::<R>(config)` vs `register(id, config)`
- Add resource discovery: `list_resources() -> Vec<ResourceInfo>`
