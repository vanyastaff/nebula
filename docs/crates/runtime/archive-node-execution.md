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
        // Получаем action
        let action = self.action_registry.get(action_id)?;
        let metadata = action.metadata();

        // Определяем уровень изоляции
        let isolation = self.resolve_isolation_level(metadata);

        // Выполняем через sandbox
        let result = match isolation {
            IsolationLevel::None => {
                // Доверенный код — выполняем напрямую
                let start = Instant::now();
                let result = action.execute(context).await;
                self.metrics.record_execution(action_id, start.elapsed(), result.is_ok());
                result
            }
            IsolationLevel::Lightweight | IsolationLevel::Full => {
                // Создаем sandboxed context с проверкой capabilities
                let sandboxed_ctx = SandboxedContext::new(
                    context,
                    metadata.capabilities.clone(),
                );
                let start = Instant::now();
                let result = self.sandbox.execute(action.as_ref(), sandboxed_ctx).await;
                self.metrics.record_execution(action_id, start.elapsed(), result.is_ok());
                result
            }
        };

        result
    }

    fn resolve_isolation_level(&self, metadata: &ActionMetadata) -> IsolationLevel {
        // Из атрибута Action, или из конфигурации по паттерну id
        metadata.isolation_level
            .unwrap_or_else(|| self.config.default_isolation_for(&metadata.id))
    }
}
```

---

