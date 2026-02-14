# nebula-core Documentation

## For AI Agents

This document provides structured information about the `nebula-core` crate for AI agents and automated tools.

## Crate Purpose

`nebula-core` is the **foundation library** for the Nebula workflow engine. It provides:
1. Core type definitions (IDs, scopes, errors)
2. Base traits for common functionality
3. Shared constants and utilities
4. Type-safe abstractions

## Module Structure

```
nebula-core/
├── src/
│   ├── lib.rs          # Main exports and prelude
│   ├── id.rs           # ID types (ExecutionId, WorkflowId, etc.)
│   ├── scope.rs        # Scope system for resource lifecycle
│   ├── traits.rs       # Base traits (Scoped, HasContext, etc.)
│   ├── types.rs        # Common types and utilities
│   ├── constants.rs    # System constants
│   ├── error.rs        # CoreError type
│   └── keys.rs         # Key types for data access
└── Cargo.toml
```

## Key Types

### Identifiers (id.rs)
- `ExecutionId` - UUID-based execution identifier
- `WorkflowId` - Domain key-based workflow identifier
- `NodeId` - Domain key-based node identifier
- `UserId` - UUID-based user identifier
- `TenantId` - UUID-based tenant identifier
- `CredentialId` - UUID-based credential identifier

All IDs implement:
- `Clone`, `Debug`, `PartialEq`, `Eq`
- `Serialize`, `Deserialize`
- `Display`

### Scope System (scope.rs)

```rust
pub enum ScopeLevel {
    Global,
    Tenant(TenantId),
    User(UserId),
    Workflow(WorkflowId),
    Execution(ExecutionId),
}
```

**Usage**: Define resource lifecycle and visibility boundaries.

### Traits (traits.rs)

#### `Scoped`
```rust
pub trait Scoped {
    fn scope(&self) -> ScopeLevel;
}
```
For resources with lifecycle management.

#### `HasContext`
```rust
pub trait HasContext {
    fn execution_id(&self) -> Option<&ExecutionId>;
    fn workflow_id(&self) -> Option<&WorkflowId>;
    fn node_id(&self) -> Option<&NodeId>;
    fn user_id(&self) -> Option<&UserId>;
    fn tenant_id(&self) -> Option<&TenantId>;
}
```
For types that carry execution context.

#### `Identifiable`
```rust
pub trait Identifiable {
    type Id;
    fn id(&self) -> &Self::Id;
}
```
For types with unique identifiers.

## Error Handling

```rust
pub enum CoreError {
    InvalidId(String),
    SerializationError(String),
    DeserializationError(String),
    // ... other variants
}

pub type Result<T> = std::result::Result<T, CoreError>;
```

## Common Patterns

### Creating Identifiers

```rust
// UUID-based IDs
let execution_id = ExecutionId::new();

// Domain key-based IDs
let workflow_id = WorkflowId::new("my-workflow");
let node_id = NodeId::new("process-step");
```

### Working with Scopes

```rust
let scope = ScopeLevel::Execution(execution_id);

match scope {
    ScopeLevel::Global => { /* system-wide */ },
    ScopeLevel::Tenant(tid) => { /* tenant-specific */ },
    ScopeLevel::Execution(eid) => { /* execution-specific */ },
    // ...
}
```

### Using Traits

```rust
struct MyResource {
    scope: ScopeLevel,
}

impl Scoped for MyResource {
    fn scope(&self) -> ScopeLevel {
        self.scope.clone()
    }
}
```

## Dependencies

**Core**:
- `thiserror` - Error derive macros
- `serde`, `serde_json` - Serialization
- `async-trait` - Async trait support

**IDs**:
- `uuid` - UUID generation
- `domain-key` - Domain-based keys

**Data**:
- `chrono` - Date/time handling
- `dashmap` - Concurrent maps
- `bincode` - Binary serialization

## Integration Points

### Used By
- `nebula-parameter` - Parameter system
- `nebula-expression` - Expression engine
- `nebula-value` - Value system
- `nebula-resource` - Resource management
- `nebula-credential` - Credential management
- All other Nebula crates

### Uses
- `nebula-log` - Logging infrastructure

## When to Use

Use `nebula-core` when you need:
1. ✅ Type-safe identifiers for workflows/executions
2. ✅ Scope-based resource management
3. ✅ Base traits for Nebula types
4. ✅ Consistent error handling
5. ✅ Common constants and utilities

## When NOT to Use

❌ Don't use `nebula-core` for:
- Business logic (use domain-specific crates)
- Data storage (use `nebula-resource` or `nebula-memory`)
- Expression evaluation (use `nebula-expression`)
- Value types (use `nebula-value`)

## Testing

```bash
# Run tests
cargo test -p nebula-core

# Run with all features
cargo test -p nebula-core --all-features

# Check documentation
cargo doc -p nebula-core --open
```

## API Stability

This crate provides **stable core APIs**. Breaking changes are rare and follow semantic versioning.

## Performance Considerations

- ID creation is very fast (microseconds)
- All IDs are stack-allocated (no heap)
- Traits are zero-cost abstractions
- Scope checks are O(1)

## Thread Safety

- All ID types are `Send + Sync`
- Scope types are immutable and thread-safe
- No global mutable state

## Version

Current version: See [Cargo.toml](./Cargo.toml)
