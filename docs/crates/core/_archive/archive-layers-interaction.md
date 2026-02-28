# Archived From "docs/archive/layers-interaction.md"

# Взаимодействие слоев и крейтов в Nebula

## Принципы взаимодействия

---

### Правила зависимостей
1. **Однонаправленные зависимости** - слои могут зависеть только от слоев ниже
2. **Через интерфейсы** - взаимодействие через трейты из `nebula-core`
3. **Event-driven** - loose coupling через `nebula-eventbus`
4. **Shared types** - общие типы только в `nebula-core`

## Детальные примеры взаимодействия

---

### 10. Cross-cutting concerns через middleware pattern

```rust
// nebula-tenant, nebula-log, nebula-metrics работают через middleware
pub struct ExecutionPipeline {
    middlewares: Vec<Box<dyn ExecutionMiddleware>>,
}

#[async_trait]
pub trait ExecutionMiddleware: Send + Sync {
    async fn before_execution(&self, ctx: &mut ExecutionContext) -> Result<()>;
    async fn after_execution(&self, ctx: &ExecutionContext, result: &ExecutionResult) -> Result<()>;
}

// Tenant middleware
pub struct TenantMiddleware;

impl ExecutionMiddleware for TenantMiddleware {
    async fn before_execution(&self, ctx: &mut ExecutionContext) -> Result<()> {
        // Извлекаем tenant context
        let tenant_id = ctx.extract_tenant_id()?;
        let tenant = TenantManager::get(tenant_id).await?;
        
        // Проверяем квоты
        tenant.check_quota(ResourceType::Execution).await?;
        
        // Инжектируем в контекст
        ctx.set_tenant_context(tenant);
        Ok(())
    }
}

// Logging middleware
pub struct LoggingMiddleware;

impl ExecutionMiddleware for LoggingMiddleware {
    async fn before_execution(&self, ctx: &mut ExecutionContext) -> Result<()> {
        log::info!("Starting execution {}", ctx.execution_id);
        Ok(())
    }
    
    async fn after_execution(&self, ctx: &ExecutionContext, result: &ExecutionResult) -> Result<()> {
        match result {
            Ok(output) => log::info!("Execution {} completed", ctx.execution_id),
            Err(e) => log::error!("Execution {} failed: {}", ctx.execution_id, e),
        }
        Ok(())
    }
}

// Metrics middleware
pub struct MetricsMiddleware;

impl ExecutionMiddleware for MetricsMiddleware {
    async fn before_execution(&self, ctx: &mut ExecutionContext) -> Result<()> {
        ctx.set_metric_start_time(Instant::now());
        metrics::increment_gauge!("executions_active", 1.0);
        Ok(())
    }
    
    async fn after_execution(&self, ctx: &ExecutionContext, result: &ExecutionResult) -> Result<()> {
        let duration = ctx.get_metric_start_time().elapsed();
        metrics::histogram!("execution_duration", duration);
        metrics::decrement_gauge!("executions_active", 1.0);
        
        if result.is_err() {
            metrics::increment_counter!("executions_failed");
        }
        Ok(())
    }
}
```

## Диаграмма взаимодействия слоев

```
┌─────────────────────────────────────────────────┐
│              Presentation Layer                  │
│                                                  │
│  API/CLI/UI вызывают Engine через SDK           │
└────────────────────┬────────────────────────────┘
                     │ использует
┌────────────────────▼────────────────────────────┐
│              Developer Tools                     │
│                                                  │
│  SDK реэкспортирует публичное API крейтов       │
└────────────────────┬────────────────────────────┘
                     │ зависит от
┌────────────────────▼────────────────────────────┐
│           Multi-tenancy & Clustering             │
│                                                  │
│  Оборачивает Engine для распределенной работы   │
└────────────────────┬────────────────────────────┘
                     │ использует
┌────────────────────▼────────────────────────────┐
│              Business Logic                      │
│                                                  │
│  Registry предоставляет Actions для Engine      │
│  Resources управляются через ResourceManager     │
└────────────────────┬────────────────────────────┘
                     │ координирует
┌────────────────────▼────────────────────────────┐
│              Execution Layer                     │
│                                                  │
│  Engine orchestrates Workers                     │
│  Runtime executes Actions through Sandbox        │
└────────────────────┬────────────────────────────┘
                     │ выполняет
┌────────────────────▼────────────────────────────┐
│                Node Layer                        │
│                                                  │
│  Actions используют Parameters и Credentials     │
└────────────────────┬────────────────────────────┘
                     │ базируется на
┌────────────────────▼────────────────────────────┐
│                Core Layer                        │
│                                                  │
│  Workflow definitions, Expressions, ParamValue    │
│  EventBus для loose coupling                     │
└────────────────────┬────────────────────────────┘
                     │ использует
┌────────────────────▼────────────────────────────┐
│          Cross-Cutting Concerns                  │
│                                                  │
│  Config, Log, Metrics, Errors, Resilience       │
│  Validator, Locale, System monitoring            │
└────────────────────┬────────────────────────────┘
                     │ хранит в
┌────────────────────▼────────────────────────────┐
│            Infrastructure Layer                  │
│                                                  │
│  Storage abstractions, Binary serialization      │
└─────────────────────────────────────────────────┘
```

## Ключевые паттерны взаимодействия

1. **Dependency Injection** - конфигурация и ресурсы инжектируются сверху вниз
2. **Event-driven** - слои общаются через события для loose coupling
3. **Middleware chain** - cross-cutting concerns через цепочку middleware
4. **serde_json::Value everywhere** - единый тип данных через все слои, expressions резолвятся на уровне Execution через `ParamValue`
5. **Context propagation** - `ExecutionContext` несет информацию через все вызовы
6. **Resource scoping** - автоматическое управление жизненным циклом на разных уровнях

