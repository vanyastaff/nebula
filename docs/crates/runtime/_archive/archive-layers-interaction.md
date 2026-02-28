# Archived From "docs/archive/layers-interaction.md"

### 8. nebula-sandbox ↔ nebula-runtime ↔ nebula-action

**Паттерн:** Runtime делегирует выполнение Action в Sandbox, который контролирует доступ через capabilities

```rust
// nebula-runtime использует sandbox для выполнения actions
impl ActionRuntime {
    pub async fn execute_action(
        &self,
        action_id: &ActionId,
        context: ActionContext,
    ) -> Result<ActionResult> {
        let action = self.action_registry.get(action_id)?;
        let metadata = action.metadata();
        
        // Определяем уровень изоляции из metadata или config
        let isolation = self.resolve_isolation_level(metadata);
        
        match isolation {
            IsolationLevel::None => {
                // Builtin actions — выполняем напрямую
                action.execute(context).await
            }
            _ => {
                // Создаем sandboxed context — проксирует вызовы через capability checks
                let sandboxed = SandboxedContext::new(
                    context,
                    metadata.capabilities.clone(),
                );
                self.sandbox.execute(action.as_ref(), sandboxed).await
            }
        }
    }
}
// ... (see full content in original)
```
