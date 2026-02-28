# Archived From "docs/archive/crates-architecture.md"

## 4. nebula-runtime

**Purpose**: Workflow execution engine and scheduling.

```rust
// nebula-runtime/src/lib.rs
pub mod engine;
pub mod executor;
pub mod scheduler;
pub mod context;

// nebula-runtime/src/engine.rs
pub struct WorkflowEngine {
    scheduler: Arc<Scheduler>,
    executor: Arc<Executor>,
    state_manager: Arc<StateManager>,
    resource_pool: Arc<ResourcePool>,
}
// ... (see full content in original)
```
