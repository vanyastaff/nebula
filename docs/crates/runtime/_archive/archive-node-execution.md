# Archived From "docs/archive/node-execution.md"

### nebula-runtime
**Назначение:** Runtime окружение для выполнения Actions и управления ресурсами.

**Ключевые компоненты:**
- ActionRuntime - выполнение actions через Sandbox
- ResourceRuntime - управление ресурсами
- Memory management
- Error handling

```rust
pub struct Runtime {
    action_runtime: ActionRuntime,
    resource_runtime: ResourceRuntime,
    memory_manager: Arc<MemoryManager>,
    metrics: Arc<RuntimeMetrics>,
}

pub struct ActionRuntime {
    action_registry: Arc<ActionRegistry>,
    executor: Arc<ActionExecutor>,
    sandbox: Arc<dyn Sandbox>,  // Pluggable sandbox
}

impl ActionRuntime {
    pub async fn execute_action(
        &self,
        action_id: &ActionId,
        context: ActionContext,
    ) -> Result<ActionResult> {
        // ... (see full content in original)
    }
}
```

---
