# Archived From "docs/archive/business-cross.md"

### nebula-metrics
**Назначение:** Сбор и экспорт метрик.

**Ключевые компоненты:**
- System metrics
- Business metrics
- Custom metrics
- Export backends

```rust
pub struct MetricsManager {
    registry: Registry,
    collectors: Vec<Box<dyn MetricsCollector>>,
}

// Автоматический сбор метрик
#[derive(Metrics)]
pub struct WorkflowMetrics {
    #[metric(type = "counter")]
    pub executions_total: Counter,
    
    #[metric(type = "histogram", buckets = [0.1, 0.5, 1.0, 5.0, 10.0])]
    pub execution_duration: Histogram,
    
    #[metric(type = "gauge")]
    pub active_executions: Gauge,
}

// Использование
impl ActionContext {
    pub async fn measure<F, T>(&self, name: &str, f: F) -> T 
    where F: Future<Output = T> {
        let start = Instant::now();
        let result = f.await;
        self.metrics.record(name, start.elapsed());
        result
    }
}

// В Action
let user = ctx.measure("database.query", async {
    db.get_user(user_id).await
}).await?;
```

---

