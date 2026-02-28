# Base Traits

`nebula-core::traits` defines the shared behavioral contracts that span the workspace.
These traits enable loose coupling: a crate can accept `impl HasContext` without
importing the concrete type that provides the context.

---

## Scoping Traits

### `Scoped`

```rust
pub trait Scoped {
    fn scope(&self) -> &ScopeLevel;

    // Convenience helpers (default implementations)
    fn is_in_scope(&self, scope: &ScopeLevel) -> bool;
    fn is_global(&self) -> bool;
    fn is_workflow(&self) -> bool;
    fn is_execution(&self) -> bool;
    fn is_action(&self) -> bool;
}
```

Implemented by anything that lives at a specific `ScopeLevel` (memory arenas, resource
handles, subscriptions).

### `HasContext`

```rust
pub trait HasContext {
    fn execution_id(&self) -> Option<&ExecutionId>;
    fn workflow_id(&self) -> Option<&WorkflowId>;
    fn node_id(&self) -> Option<&NodeId>;
    fn user_id(&self) -> Option<&UserId>;
    fn tenant_id(&self) -> Option<&TenantId>;

    // Convenience helpers
    fn has_execution_context(&self) -> bool;
    fn has_workflow_context(&self) -> bool;
    fn has_node_context(&self) -> bool;
    fn has_user_context(&self) -> bool;
    fn has_tenant_context(&self) -> bool;
}
```

Implemented by context types (`ActionContext`, `TriggerContext`, `ExecutionContext`).
Used by logging, metrics, and authorization code to extract identifiers without knowing
the concrete context type.

---

## Identity Traits

### `Identifiable`

```rust
pub trait Identifiable {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn description(&self) -> Option<&str>;
    fn version(&self) -> Option<&str>;

    fn has_name(&self) -> bool;
    fn has_description(&self) -> bool;
    fn has_version(&self) -> bool;
}
```

Implemented by plugins, actions, workflows — anything with a stable identity.

---

## Validation Trait

### `Validatable<Error>`

```rust
pub trait Validatable<Error> {
    fn validate(&self) -> Result<(), Vec<Error>>;
    fn is_valid(&self) -> bool { self.validate().is_ok() }
}
```

Generic over the error type so each crate can return its own error. Used by
`ParameterCollection`, workflow definitions, and configuration structs.

---

## Serialization Trait

### `Serializable`

```rust
pub trait Serializable: Serialize + for<'de> Deserialize<'de> {
    fn to_json(&self) -> Result<String, CoreError>;
    fn to_json_pretty(&self) -> Result<String, CoreError>;
    fn from_json(json: &str) -> Result<Self, CoreError>;

    // Binary (bincode)
    fn to_binary(&self) -> Result<Vec<u8>, CoreError>;
    fn from_binary(bytes: &[u8]) -> Result<Self, CoreError>;
}
```

Blanket-implementable for any `Serialize + DeserializeOwned` type. Provides
consistent error wrapping over `serde_json` and `bincode`.

---

## Value Traits

| Trait | Requires |
|---|---|
| `Cloneable` | `Clone` |
| `Comparable` | `PartialEq + Eq + PartialOrd + Ord` |
| `Hashable` | `Hash` |
| `Displayable` | `Display` |
| `Debuggable` | `Debug` |
| `StringConvertible` | `ToString + FromStr` |

These are marker/alias traits used in trait bounds to express combined requirements
concisely.

---

## Metadata Trait

### `HasMetadata`

```rust
pub trait HasMetadata {
    fn metadata(&self) -> &EntityMetadata;
    fn metadata_mut(&mut self) -> &mut EntityMetadata;
}

pub struct EntityMetadata {
    pub created_at: DateTime<Utc>,
    pub modified_at: DateTime<Utc>,
    pub tags: Vec<String>,
    pub custom: HashMap<String, String>,
}
```

Builder:

```rust
let meta = EntityMetadata::new()
    .with_tag("production")
    .with_custom("team", "platform")
    .build();
```

---

## Usage Pattern

```rust
// A logging helper doesn't need to know the concrete context type:
fn log_node_start(ctx: &impl HasContext) {
    if let (Some(exec), Some(node)) = (ctx.execution_id(), ctx.node_id()) {
        tracing::info!(execution_id = %exec, node_id = %node, "node started");
    }
}

// Works with any future context type: ActionContext, TriggerContext, etc.
```
