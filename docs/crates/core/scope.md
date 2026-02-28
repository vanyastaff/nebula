# Scope System

`ScopeLevel` models the resource lifecycle hierarchy in Nebula. Every managed resource
(memory arena, credential, cache entry, etc.) lives at exactly one scope level. When a
scope ends, all resources at that level are automatically cleaned up.

---

## ScopeLevel Enum

```rust
pub enum ScopeLevel {
    Global,
    Organization(OrganizationId),
    Project(ProjectId),
    Workflow(WorkflowId),
    Execution(ExecutionId),
    Action(ExecutionId, NodeId),
}
```

### Hierarchy (broadest → narrowest)

```
Global
  └─ Organization(id)
       └─ Project(id)
            └─ Workflow(id)
                 └─ Execution(id)
                      └─ Action(execution_id, node_id)
```

A scope `A` contains scope `B` if `A` is an ancestor of `B` in this hierarchy.

---

## Containment

```rust
// Is `child` inside `parent`?
let yes = ScopeLevel::Global.is_contained_in(&ScopeLevel::Global); // true (same)
let yes = ScopeLevel::Execution(exec_id)
    .is_contained_in(&ScopeLevel::Workflow(wf_id)); // true — Execution ⊂ Workflow

// Only for Execution/Action where IDs are available
let yes = ScopeLevel::Action(exec_id, node_id)
    .is_contained_in(&ScopeLevel::Execution(exec_id)); // true
```

---

## Navigation

```rust
// Parent scope (one level up)
let parent: Option<ScopeLevel> = scope.parent();

// Child scope (one level down — requires the child-level ID)
enum ChildScopeType {
    Organization(OrganizationId),
    Project(ProjectId),
    Workflow(WorkflowId),
    Execution(ExecutionId),
    Action(ExecutionId, NodeId),
}
let child = scope.child(ChildScopeType::Workflow(wf_id));
```

---

## ScopedId

```rust
pub struct ScopedId {
    pub scope: ScopeLevel,
    pub id: String,
}
```

A scope-aware resource identifier. Combines a scope with an arbitrary string ID, e.g.
to identify a specific resource within an execution scope.

**Display format:**

| Scope | Output |
|---|---|
| `Global` | `"global"` |
| `Organization(id)` | `"organization:{id}"` |
| `Project(id)` | `"project:{id}"` |
| `Workflow(id)` | `"workflow:{id}"` |
| `Execution(id)` | `"execution:{id}"` |
| `Action(exec, node)` | `"action:{exec}:{node}"` |

---

## Usage Pattern

Scopes drive automatic cleanup in `nebula-memory` and `nebula-resource`. When an
execution ends, the engine drops the `Execution` scope, which cascades cleanup of
all `Action`-scoped sub-resources.

```rust
// nebula-memory (conceptual)
let data = memory.allocate_scoped(large_value, ScopeLevel::Execution(exec_id));
// data is freed when the Execution scope is dropped

// nebula-resource (conceptual)
let conn = resource_manager.get_scoped::<DbConnection>(ScopeLevel::Workflow(wf_id));
// connection returned to pool when the Workflow scope ends
```

---

## Archive Note

The original design had 4 scope levels: `Global / Workflow / Execution / Action`.
The current implementation adds `Organization` and `Project` between `Global` and
`Workflow` to support multi-tenant and project isolation without additional wrapper
structs.
