# nebula-core

Core types and traits for the Nebula workflow engine.

## Overview

`nebula-core` provides the fundamental building blocks used by all other Nebula crates. It defines common identifiers, scope management, base traits, and shared types that form the foundation of the Nebula ecosystem.

## Key Components

### Identifiers
- **ExecutionId** - Unique identifier for workflow executions
- **WorkflowId** - Identifier for workflow definitions
- **NodeId** - Identifier for workflow nodes/steps
- **UserId** - User identification
- **TenantId** - Multi-tenancy support
- **CredentialId** - Credential reference

### Scope System
Resource lifecycle management with different scope levels:
- `Global` - System-wide resources
- `Tenant` - Tenant-specific resources
- `User` - User-specific resources
- `Workflow` - Workflow-level resources
- `Execution` - Execution-specific resources

### Base Traits
- **Scoped** - For resources with lifecycle management
- **HasContext** - For types that carry execution context
- **Identifiable** - For types with unique identifiers

### Common Types
Constants, utilities, and shared types used throughout the system.

## Usage

```rust
use nebula_core::{
    ExecutionId, WorkflowId, NodeId,
    ScopeLevel, Scoped, HasContext
};

// Create identifiers
let execution_id = ExecutionId::new();
let workflow_id = WorkflowId::new("my-workflow");
let node_id = NodeId::new("process-data");

// Define resource scope
let scope = ScopeLevel::Execution(execution_id.clone());
```

## Using the Prelude

```rust
use nebula_core::prelude::*;

// All core types are now available
let execution_id = ExecutionId::new();
let result: Result<()> = Ok(());
```

## Features

- Type-safe identifiers with compile-time guarantees
- Flexible scope system for resource management
- Comprehensive error handling with `CoreError`
- Serde support for all types
- Async-ready traits with `async-trait`

## Dependencies

See [Cargo.toml](./Cargo.toml) for the full list of dependencies.

## License

Licensed under the same terms as the Nebula project.
