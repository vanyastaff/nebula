# nebula-core

Foundation crate for the Nebula workflow automation platform (n8n-class). Defines the shared vocabulary used by every other crate.

## Scope

- **In scope:** **id** — UserId, TenantId, ExecutionId, WorkflowId, NodeId, ActionId, ResourceId, CredentialId, ProjectId, RoleId, OrganizationId (domain_key); **scope** — ScopeLevel (Global, Organization, Project, Workflow, Execution, Action), containment; **keys** — ParameterKey, CredentialKey, PluginKey, PluginKeyError; **traits** — Scoped, HasContext, Identifiable, OperationContext, OperationResult, Priority; **types** — Version, InterfaceVersion, EntityMetadata, ProjectType, RoleScope; **constants** — SYSTEM_*, DEFAULT_*; **error** — CoreError, Result&lt;T&gt;. Zero business logic, orchestration, storage, or transport.
- **Out of scope:** Workflow execution, node scheduling, persistence, API/CLI/UI, plugin runtime.

## Current State

- **Maturity:** Stable foundation; APIs aligned with workspace usage.
- **Key strengths:** Zero cyclic dependencies; type-safe IDs; serde-first types; comprehensive error taxonomy.
- **Key risks:** `CoreError` mixes foundation and domain variants (see PROPOSALS); scope containment semantics simplified in some cases.

## Target State

- **Production criteria:** Explicit compatibility policy; stable serialized schemas; strict scope containment where security matters.
- **Compatibility guarantees:** Patch/minor preserve public API and serialized forms; breaking changes via major version and MIGRATION.md.

## Quick Example

```rust
use nebula_core::{
    ExecutionId, WorkflowId, NodeId, ScopeLevel, OperationContext, Priority,
};

let execution_id = ExecutionId::new();
let workflow_id = WorkflowId::new();
let node_id = NodeId::new();

let scope = ScopeLevel::Action(execution_id, node_id);

let ctx = OperationContext::new("op_run_node")
    .with_execution_id(execution_id)
    .with_workflow_id(workflow_id)
    .with_node_id(node_id)
    .with_priority(Priority::High);

assert!(ctx.has_execution_context());
assert!(scope.is_action());
```

## Document Map

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [INTERACTIONS.md](./INTERACTIONS.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)
- [SECURITY.md](./SECURITY.md)
- [RELIABILITY.md](./RELIABILITY.md)
- [TEST_STRATEGY.md](./TEST_STRATEGY.md)
- [MIGRATION.md](./MIGRATION.md)
- [COMPATIBILITY.md](./COMPATIBILITY.md)

## Archive

Legacy material:
- [`_archive/`](./_archive/)
