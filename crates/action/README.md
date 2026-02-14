# nebula-action

Action execution framework for the Nebula workflow engine.

## Overview

`nebula-action` provides the core infrastructure for executing workflow actions/nodes. It defines the action lifecycle, parameter handling, and execution context.

## Status

⚠️ **Work in Progress** - This crate is currently under development.

## Planned Features

- **Action Trait** - Base trait for all workflow actions
- **Parameter Binding** - Automatic parameter injection
- **Execution Context** - Action execution environment
- **Error Handling** - Standardized action error handling
- **Lifecycle Hooks** - Before/after execution hooks
- **Timeout Management** - Action execution timeouts
- **Retry Logic** - Automatic retry for transient failures

## Architecture

```
nebula-action/
├── core/              # Core traits and types
├── execution/         # Execution engine (planned)
├── context/           # Execution context (planned)
└── process/           # Process management (partial)
```

## Usage (Planned)

```rust
use nebula_action::prelude::*;

#[derive(Action)]
pub struct HttpRequestAction;

#[async_trait]
impl Action for HttpRequestAction {
    type Input = HttpRequestParams;
    type Output = HttpResponse;

    async fn execute(&self, input: Self::Input, ctx: &ActionContext)
        -> ActionResult<Self::Output> {
        // Execute HTTP request
        todo!()
    }
}
```

## Integration

This crate will integrate with:
- `nebula-parameter` - Parameter validation and binding
- `nebula-resource` - Resource access (DB, HTTP, etc.)
- `nebula-credential` - Authentication

## License

Licensed under the same terms as the Nebula project.
