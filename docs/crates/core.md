# nebula-core

Foundational crate. Defines identifiers, the scope system, and shared traits used by every
other crate. Contains no business logic — only the primitives that prevent cyclic
dependencies.

## Identifiers

All IDs are strongly typed newtypes to prevent accidental mixing.

```rust
pub struct ExecutionId(Uuid);    // Unique per workflow run
pub struct WorkflowId(String);   // Slug or UUID string
pub struct NodeId(String);       // Node identifier within a workflow
pub struct UserId(String);
pub struct TenantId(String);
```

## Scope System

Resources, memory, and credentials are allocated within a scope. The engine cleans up
scope-bound allocations automatically when the scope ends.

```rust
pub enum ScopeLevel {
    Global,
    Workflow(WorkflowId),
    Execution(ExecutionId),
    Action(ExecutionId, NodeId),
}

pub trait Scoped {
    fn scope(&self) -> &ScopeLevel;
}

pub trait HasContext {
    fn execution_id(&self) -> Option<&ExecutionId>;
    fn workflow_id(&self) -> Option<&WorkflowId>;
    fn tenant_id(&self) -> Option<&TenantId>;
}
```

## Parameter Values

`ParamValue` separates expressions and templates from literal data in workflow definitions.
After resolution at the Execution layer everything becomes `serde_json::Value`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    /// "$nodes.user_lookup.result.email"
    Expression(Expression),
    /// "Order #{order_id} for {name}"
    Template(TemplateString),
    /// Plain JSON literal
    Literal(serde_json::Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expression {
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateString {
    pub template: String,
    pub bindings: Vec<String>,
}
```

Resolution happens in `nebula-execution` before the value reaches an `Action`:

```rust
// In ExecutionContext:
let resolved: serde_json::Value = context.resolve_param_value(&param_value).await?;
```

## `ActionMetadata`

Describes a node so the engine and UI can discover and display it.

```rust
pub struct ActionMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub version: semver::Version,
    pub author: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub tags: Vec<String>,
    pub capabilities: CapabilityRequirements,
    pub resource_requirements: ResourceRequirements,
}

pub struct CapabilityRequirements {
    pub network_access: bool,
    pub filesystem_access: bool,
    pub system_commands: bool,
    pub custom_capabilities: Vec<String>,
}

pub struct ResourceRequirements {
    pub memory_mb: Option<u64>,
    pub cpu_cores: Option<f32>,
    pub execution_timeout: Option<Duration>,
}
```

## Error Pattern

`nebula-core` does not define a shared error type. Each crate defines its own `Error` enum.
The convention used across the workspace:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("action execution failed: {0}")]
    Execution(String),

    #[error("resource unavailable: {id}")]
    ResourceUnavailable { id: String },

    #[error(transparent)]
    Storage(#[from] nebula_storage::Error),
}
```

## Dependency Rule

`nebula-core` has **no internal workspace dependencies**. It only depends on external crates
(`serde`, `uuid`, `thiserror`, `semver`). This enforces the no-cycle guarantee.

Every other crate may depend on `nebula-core`, but `nebula-core` depends on nothing in the
workspace.

## Archived Ideas (from `docs/archive/architecture-v2.md`)

These are intentionally kept as historical design notes, not current implementation.

### Workflow graph model

```rust
pub struct Workflow {
    pub id: WorkflowId,
    pub name: String,
    pub version: Version,
    pub graph: WorkflowGraph,
    pub metadata: WorkflowMetadata,
}

pub struct WorkflowGraph {
    pub nodes: HashMap<NodeId, Node>,
    pub edges: Vec<Edge>,
    pub subgraphs: HashMap<SubgraphId, WorkflowGraph>,
}
```

### Type-safe connections idea

- `TypedConnection<From, To>` with `PhantomData` was proposed for compile-time port compatibility.
- `TypedPort` trait was proposed to validate connection rules by data type.
- This is still a design direction; current runtime graph compatibility checks remain the source of truth.
