# nebula-core

Core types and traits for the Nebula workflow engine.

## Overview

`nebula-core` provides the fundamental building blocks used by all other Nebula crates. It defines common identifiers (via [domain-key](https://crates.io/crates/domain-key)), scope management, keys, base traits, and shared types that form the foundation of the Nebula ecosystem.

## Key Components

### Identifiers

Type-safe UUID identifiers (powered by **domain-key**; no direct `uuid` dependency):

- **ExecutionId**, **WorkflowId**, **NodeId** — workflow execution, definition, and node (step)
- **UserId**, **TenantId** — identity and multi-tenancy
- **ProjectId**, **OrganizationId** — projects and organizations
- **ResourceId**, **CredentialId**, **RoleId** — resources, credentials, and roles

All ID types are `Copy`, support `new()`, `nil()`, `parse(&str)`, serde, and re-export **UuidParseError** for parse error handling.

### Keys

- **PluginKey** — plugin type (e.g. `telegram_bot`); **ActionKey** is an alias for plugin/action identification
- **ParameterKey**, **CredentialKey** — parameter and credential keys with normalization and validation

### Scope System

Resource lifecycle and scope levels:

- **Global** — system-wide
- **Organization** — organization-scoped
- **Project** — project-scoped
- **Workflow** — workflow-level
- **Execution** — execution-specific
- **Action** — action/node within an execution

### Base Traits

- **Scoped** — resources with scope and lifecycle
- **HasContext** — types that carry execution context
- **Identifiable** — types with unique identifiers

### Common Types

Constants, **CoreError**, **InterfaceVersion**, **Status**, **Priority**, **ProjectType**, **RoleScope**, and utilities used across the system.

## Usage

```rust
use nebula_core::{
    ExecutionId, WorkflowId, NodeId,
    ScopeLevel, Scoped, HasContext
};

// Create identifiers (domain-key UUIDs)
let execution_id = ExecutionId::new();
let workflow_id = WorkflowId::new();
let node_id = NodeId::new();

// Define resource scope
let scope = ScopeLevel::Execution(execution_id);
```

## Prelude

```rust
use nebula_core::prelude::*;

let execution_id = ExecutionId::new();
let result: Result<()> = Ok(());
```

## Features

- Type-safe identifiers with compile-time guarantees (domain-key)
- Flexible scope system (Global → Organization → Project → Workflow → Execution → Action)
- Comprehensive error handling with **CoreError**
- Serde support for core types
- Normalized keys (PluginKey, ParameterKey, CredentialKey)

## Dependencies

See [Cargo.toml](./Cargo.toml). Main: **domain-key**, chrono, serde, thiserror, postcard.

## License

Licensed under the same terms as the Nebula project.
