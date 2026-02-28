# Archived From "docs/archive/layers-interaction.md"

### 7. nebula-resilience обертывает другие крейты

**Паттерн:** Resilience patterns оборачивают вызовы других крейтов

```rust
// nebula-action использует resilience
impl ProcessAction for ExternalApiAction {
    async fn execute(&self, input: Input, ctx: &ActionContext) -> Result<Output> {
        // Получаем resilience executor из контекста
        let resilience = ctx.get_resilience_executor();
        
        // Оборачиваем внешний вызов в resilience patterns
        let result = resilience
            .with_circuit_breaker()
            .with_retry(RetryPolicy::exponential())
            .with_timeout(Duration::from_secs(30))
            .execute(async {
                // Реальный вызов API
                let client = ctx.get_resource::<HttpClient>().await?;
                client.post(&input.url, &input.body).await
            })
            .await?;
        
        Ok(result)
    }
}

// nebula-resource использует resilience для health checks
impl ResourceInstance for DatabaseInstance {
    async fn health_check(&self) -> HealthStatus {
        let resilience = ResilientExecutor::new()
            .with_timeout(Duration::from_secs(5))
            .with_retry(RetryPolicy::fixed(3, Duration::from_millis(100)));
        
        match resilience.execute(async {
            self.pool.acquire().await?.ping().await
        }).await {
            Ok(_) => HealthStatus::Healthy,
            Err(e) => HealthStatus::Unhealthy { reason: e.to_string() },
        }
    }
}
```

