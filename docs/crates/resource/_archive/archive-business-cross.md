# Archived From "docs/archive/business-cross.md"

### nebula-resource
**Назначение:** Управление жизненным циклом долгоживущих ресурсов с учетом scopes.

**Ключевые концепции:**
- Scoped resources (Global/Workflow/Execution/Action)
- Connection pooling
- Health monitoring
- Automatic cleanup

```rust
// Определение ресурса
#[derive(Resource)]
#[resource(
    id = "database",
    name = "Database Connection Pool",
    lifecycle = "global"  // Один экземпляр на все приложение
)]
pub struct DatabaseResource;

pub struct DatabaseInstance {
    pool: sqlx::Pool<Postgres>,
    metrics: DatabaseMetrics,
}

impl ResourceInstance for DatabaseInstance {
    async fn health_check(&self) -> HealthStatus {
        match self.pool.acquire().await {
            Ok(_) => HealthStatus::Healthy,
            Err(e) => HealthStatus::Unhealthy { reason: e.to_string() },
        }
    }
    
    async fn cleanup(&mut self) -> Result<()> {
        self.pool.close().await;
        Ok(())
    }
}

// Различные scopes ресурсов
#[derive(Resource)]
#[resource(lifecycle = "execution")]
pub struct LoggerResource;  // Новый logger для каждого execution

#[derive(Resource)]
#[resource(lifecycle = "workflow")]
pub struct MetricsCollectorResource;  // Один collector на workflow

// Использование в Action
impl ProcessAction for DatabaseQueryAction {
    async fn execute(&self, input: Input, ctx: &ActionContext) -> Result<Output> {
        // Получаем ресурс с правильным scope
        let db = ctx.get_resource::<DatabaseResource>().await?;  // Global
        let logger = ctx.get_resource::<LoggerResource>().await?;  // Per execution
        let metrics = ctx.get_resource::<MetricsCollectorResource>().await?;  // Per workflow
        
        logger.info("Executing query");
        let start = Instant::now();
        
        let result = db.query(&input.sql).await?;
        
        metrics.record_query_duration(start.elapsed());
        Ok(result)
    }
}
```

---

