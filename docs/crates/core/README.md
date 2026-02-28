# nebula-core

`nebula-core` is the foundation crate for the Nebula automation platform (n8n-like, but in Rust).

It defines the shared language that every other crate uses:
- typed IDs (`ExecutionId`, `WorkflowId`, `NodeId`, ...)
- scope model (`ScopeLevel`)
- base traits (`Scoped`, `HasContext`, `Identifiable`, ...)
- common domain types (`Status`, `Priority`, `OperationContext`, ...)
- common error model (`CoreError`)
- validated keys (`PluginKey`, `ParameterKey`, `CredentialKey`)
- system defaults and limits (`constants`)

## Current Reality (from code)

- Crate: `crates/core`
- Rust edition: `2024`
- Workspace minimum Rust: `1.92` (see root `Cargo.toml`)
- Main modules:
  - `id`
  - `scope`
  - `traits`
  - `types`
  - `error`
  - `keys`
  - `constants`

## Why It Exists

For a workflow platform with many crates (`engine`, `runtime`, `storage`, `api`, `sdk`, ...), this crate prevents cyclic dependencies by keeping shared primitives in one place.

Every higher-level crate should depend on `nebula-core`, and almost never depend on each other for fundamental types.

## Quick Example

```rust
use nebula_core::{
    ExecutionId, WorkflowId, NodeId, ScopeLevel, OperationContext, Priority,
};

let execution_id = ExecutionId::v4();
let workflow_id = WorkflowId::v4();
let node_id = NodeId::v4();

let scope = ScopeLevel::Action(execution_id, node_id);

let ctx = OperationContext::new("op_run_node")
    .with_execution_id(execution_id)
    .with_workflow_id(workflow_id)
    .with_node_id(node_id)
    .with_priority(Priority::High);

assert!(ctx.has_execution_context());
assert!(scope.is_action());
```

## Document Set

- [ARCHITECTURE.md](ARCHITECTURE.md)
- [API.md](API.md)
- [DECISIONS.md](DECISIONS.md)
- [ROADMAP.md](ROADMAP.md)
- [PROPOSALS.md](PROPOSALS.md)
