# Archived From "docs/archive/layers-interaction.md"

### 9. nebula-memory управляет памятью для других крейтов

**Паттерн:** Memory manager предоставляет scoped allocation

```rust
// nebula-execution использует memory для кеширования
impl ExecutionContext {
    pub async fn cache_node_result(&self, node_id: NodeId, result: serde_json::Value) -> Result<()> {
        // Allocate в execution-scoped memory
        let cached = self.memory_manager
            .allocate_scoped(result, MemoryScope::Execution(self.execution_id))
            .await?;
        
        self.node_cache.insert(node_id, cached);
        Ok(())
    }
}

// nebula-expression кеширует compiled expressions
impl ExpressionEngine {
    pub async fn compile_and_cache(&self, expr: &str) -> Result<CompiledExpression> {
        // Проверяем кеш
        if let Some(compiled) = self.memory_manager.get_cached(expr).await {
            return Ok(compiled);
        }
        
        // Компилируем
        let compiled = self.compile(expr)?;
        
        // Кешируем в global scope (т.к. expressions переиспользуются)
        self.memory_manager
            .cache(expr, compiled.clone(), MemoryScope::Global)
            .await?;
        
        Ok(compiled)
    }
}
```

