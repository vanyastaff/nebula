# API Reference (Human-Oriented)

This file documents the current public surface based on `crates/core/src/*`.

## Public Surface

- **Stable APIs:** All re-exported types, traits, constants. No deprecations.
- **Experimental APIs:** None. Core is conservative.
- **Hidden/internal APIs:** `error`, `keys` modules are public but implementation details (e.g., `collapse_underscores`) are private.

## Re-exports

`lib.rs` re-exports:
- all items from `constants`, `id`, `keys`, `scope`, `traits`, `types`
- error items from `error` (`CoreError`, related aliases)
- `Result<T> = std::result::Result<T, CoreError>`
- `prelude` module with commonly used core types

## Usage Patterns

- **ID creation:** Prefer `Id::v4()` for new entities; `Id::parse(s)` for deserialization; `Id::nil()` for sentinel/default.
- **Scope construction:** Use `ScopeLevel::Action(exec_id, node_id)` for action-scoped resources; `ScopedId::execution(exec_id, "key")` for execution-scoped keys.
- **Context propagation:** Build `OperationContext` with builder pattern; pass by reference into execution paths.
- **Error handling:** Use `CoreError::validation()`, `CoreError::not_found()`, etc.; check `is_retryable()` for retry logic.

## IDs (`id.rs`)

Typed UUID wrappers:
- `UserId`
- `TenantId`
- `ExecutionId`
- `WorkflowId`
- `NodeId`
- `ActionId`
- `ResourceId`
- `CredentialId`
- `ProjectId`
- `RoleId`
- `OrganizationId`

Key properties:
- distinct types (can’t accidentally pass `NodeId` where `WorkflowId` is required)
- support generation/parsing/serde/display
- cheap copy semantics

## Scoping (`scope.rs`)

`ScopeLevel` variants:
- `Global`
- `Organization(OrganizationId)`
- `Project(ProjectId)`
- `Workflow(WorkflowId)`
- `Execution(ExecutionId)`
- `Action(ExecutionId, NodeId)`

Important APIs:
- classification helpers (`is_global`, `is_project`, ...)
- context accessors (`organization_id`, `project_id`, `execution_id`, `node_id`)
- containment checks (`is_contained_in`)
- parent/child helpers (`parent`, `child`)
- `ScopedId` constructor helpers (`global`, `workflow`, `execution`, `action`)

## Traits (`traits.rs`)

Primary traits:
- `Scoped`
- `HasContext`
- `Identifiable`
- `Validatable`
- `Serializable`
- `HasMetadata`

Supporting traits:
- `Cloneable`, `Comparable`, `Hashable`, `Displayable`, `Debuggable`, `StringConvertible`

## Shared Types (`types.rs`)

Core value types:
- `Version`
- `InterfaceVersion`
- `Status`
- `Priority`
- `ProjectType`
- `RoleScope`
- `OperationResult<T>`
- `OperationContext`

Helpers:
- `utils::generate_operation_id`
- `utils::format_duration`
- `utils::parse_version`
- `utils::is_valid_identifier`

## Errors (`error.rs`)

`CoreError` includes:
- validation and input errors
- auth/authz and access errors
- state/config/internal/dependency/network/storage errors
- workflow/node/expression/resource/cluster/tenant domain errors

Methods:
- `is_retryable`
- `is_client_error`
- `is_server_error`
- `error_code`
- `user_message`

## Keys (`keys.rs`)

- `ParameterKey`
- `CredentialKey`
- `PluginKey` with normalization + validation

`PluginKey` normalization:
- trim
- lowercase
- whitespace/hyphen -> underscore
- collapse repeated underscores
- strip leading/trailing underscores
- max length 64

## Constants (`constants.rs`)

Platform defaults and limits:
- timeouts/retries/circuit-breaker defaults
- size and queue limits
- env var names
- status/error codes
- feature flags
- regex patterns and validation limits

## Minimal Example

```rust
use nebula_core::{ExecutionId, WorkflowId, NodeId, ScopeLevel, Scoped};

let exec_id = ExecutionId::v4();
let workflow_id = WorkflowId::v4();
let node_id = NodeId::v4();

let scope = ScopeLevel::Action(exec_id, node_id);
assert!(scope.is_action());
```

## Advanced Example

```rust
use nebula_core::{
    ExecutionId, WorkflowId, NodeId, ScopeLevel, OperationContext, Priority,
    OperationResult, CoreError, PluginKey,
};

let ctx = OperationContext::new("op_run_node")
    .with_execution_id(ExecutionId::v4())
    .with_workflow_id(WorkflowId::v4())
    .with_node_id(NodeId::v4())
    .with_priority(Priority::High)
    .with_metadata("source", "api");

let plugin_key: PluginKey = "HTTP Request".parse().unwrap();
assert_eq!(plugin_key.as_str(), "http_request");

let result: OperationResult<String> = OperationResult::success(
    "ok".to_string(),
    std::time::Duration::from_millis(50),
);
assert!(result.is_success());
```

## Error Semantics

- **Retryable errors:** `Timeout`, `RateLimitExceeded`, `ServiceUnavailable`, `Network { retryable: true }`, `NodeExecution { retryable: true }`, `Storage`. Use `CoreError::is_retryable()`.
- **Fatal errors:** `Validation`, `NotFound`, `PermissionDenied`, `Authentication`, `Authorization`, `InvalidInput`, `InvalidState`. Do not retry.
- **Validation errors:** `CoreError::validation()`, `CoreError::validation_with_details()`; `is_client_error()` true.

## Compatibility Rules

- **Major version bump:** New `CoreError` variants that consumers may match on; enum variant renames; serde schema changes for public types; ID/scope semantic changes.
- **Deprecation policy:** Minimum 6 months; deprecated items documented in MIGRATION.md; `#[deprecated]` attribute with replacement path.
