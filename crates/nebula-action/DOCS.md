# nebula-action Documentation

## For AI Agents

## Crate Purpose

`nebula-action` will be the **action execution framework** for Nebula workflows. Defines how workflow nodes/actions are executed.

## Status

⚠️ **Work in Progress** - Crate is currently minimal/under development.

## Planned Architecture

### Action Trait
```rust
#[async_trait]
pub trait Action: Send + Sync {
    type Input;
    type Output;

    async fn execute(&self, input: Self::Input, ctx: &ActionContext)
        -> ActionResult<Self::Output>;
}
```

### ActionContext
Provides execution environment:
- Parameters
- Resources (DB, HTTP, etc.)
- Credentials
- Workflow metadata
- Trace/span context

### Lifecycle
1. **Prepare** - Validate parameters, acquire resources
2. **Execute** - Run action logic
3. **Cleanup** - Release resources, finalize
4. **Error Handling** - Handle failures, retry if needed

## Module Structure (Planned)

```
nebula-action/
├── core/
│   ├── action.rs          # Action trait
│   ├── context.rs         # ActionContext
│   ├── error.rs           # ActionError
│   └── result.rs          # ActionResult
├── execution/
│   ├── engine.rs          # Execution engine
│   ├── lifecycle.rs       # Lifecycle management
│   └── timeout.rs         # Timeout handling
├── parameters/
│   └── binding.rs         # Parameter injection
└── process/
    └── mod.rs             # Process management
```

## Integration Points (Planned)

**Will use**:
- `nebula-parameter` - Parameter validation
- `nebula-resource` - Resource access
- `nebula-credential` - Authentication
- `nebula-error` - Error handling
- `nebula-core` - Core types (IDs, scopes)

**Will be used by**: Workflow execution engine

## When to Use

✅ When implementing workflow actions/nodes
✅ When executing user-defined logic
✅ When integrating with external services

## Current State

- Mostly empty placeholder
- Some core module structure exists
- Process module has partial implementation
- Not ready for production use

## Development Status

Check the repository for latest updates on implementation progress.

## Version

See [Cargo.toml](./Cargo.toml)
