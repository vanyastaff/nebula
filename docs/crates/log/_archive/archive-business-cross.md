# Archived From "docs/archive/business-cross.md"

### nebula-log
**Назначение:** Структурированное логирование с контекстом.

**Ключевые возможности:**
- Structured logging
- Context propagation
- Multiple backends
- Async buffering

```rust
pub struct Logger {
    backend: Box<dyn LogBackend>,
    context: LogContext,
    filters: Vec<LogFilter>,
}

// Контекст автоматически добавляется
impl ExecutionContext {
    pub fn log_info(&self, msg: &str) {
        info!(
            execution_id = %self.execution_id,
            workflow_id = %self.workflow_id,
            node_id = ?self.current_node_id,
            "{}", msg
        );
    }
}

// Различные backends
let console = ConsoleBackend::new().with_colors();
let file = FileBackend::new("app.log").with_rotation(RotationPolicy::Daily);
let elastic = ElasticsearchBackend::new("http://localhost:9200");

// Использование в Action
ctx.log_info("Starting user lookup");
ctx.log_error("Database connection failed", &error);
```

---

