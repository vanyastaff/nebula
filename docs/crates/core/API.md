# API Reference (Human-Oriented)

This file documents the current public surface based on `crates/core/src/*`.

## Re-exports

`lib.rs` re-exports:
- all items from `constants`, `id`, `keys`, `scope`, `traits`, `types`
- error items from `error` (`CoreError`, related aliases)
- `Result<T> = std::result::Result<T, CoreError>`
- `prelude` module with commonly used core types

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
