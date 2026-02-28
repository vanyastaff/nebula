# Archived From "docs/archive/business-cross.md"

### nebula-system
**Назначение:** Мониторинг системных ресурсов.

```rust
pub struct SystemMonitor {
    system: Arc<RwLock<System>>,
    collectors: Vec<Box<dyn MetricsCollector>>,
}

// Системные метрики
let metrics = monitor.get_current_metrics().await;
println!("CPU: {:.1}%", metrics.cpu.usage_percent);
println!("Memory: {:.1}%", metrics.memory.usage_percent);

// Health checks
let health = HealthChecker::new()
    .add_check(DatabaseHealthCheck)
    .add_check(DiskSpaceHealthCheck { threshold: 90 })
    .add_check(MemoryHealthCheck { threshold: 80 });

let status = health.check_all().await;

// Resource pressure detection
if detector.detect_pressure(&metrics).contains(&ResourcePressure::MemoryCritical) {
    // Trigger cleanup or scale-out
}
```
