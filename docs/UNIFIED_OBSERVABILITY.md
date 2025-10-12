# Unified Nebula Observability Strategy

## Current State (Проблема)

### Анализ текущих подходов:

1. **nebula-memory**
   - ✅ Мощная stats система (`src/stats/` ~280KB кода)
   - ✅ collector, aggregator, histogram, real-time monitoring
   - ✅ Predictive analytics, profiling
   - ❌ Изолированная, только для memory

2. **nebula-resource**
   - ✅ Использует стандартный `metrics = "0.21"` крейт
   - ✅ Prometheus exporter из коробки
   - ✅ Counter, Gauge, Histogram
   - ✅ Индустриальный стандарт

3. **nebula-resilience** (только что добавили)
   - ❌ Самописная система (MetricsCollector, hooks)
   - ❌ Дублирование LogLevel
   - ❌ Не переиспользуется

### Проблемы:
- 3 разных подхода к metrics
- Нет единого способа собрать метрики со всех крейтов
- Дублирование кода и концепций
- Сложно добавлять observability в новые крейты

## Решение: Двухуровневая Архитектура

### Уровень 1: Стандартный `metrics` Крейт (Foundation)

**Почему `metrics`, а не custom решение?**
- ✅ Индустриальный стандарт Rust
- ✅ Уже используется в nebula-resource
- ✅ Backend-agnostic (Prometheus, StatsD, OpenTelemetry)
- ✅ Zero-cost abstractions
- ✅ Отличная экосистема exporters

```toml
# Добавить во все крейты как optional dependency
[dependencies]
metrics = { version = "0.21", optional = true }

[features]
observability = ["metrics"]
```

### Уровень 2: nebula-log как Facade

**nebula-log** становится единым фасадом для:
- Logging (уже есть)
- Tracing (уже есть через OpenTelemetry)
- Metrics (новое - re-export `metrics` + helpers)
- Events (новое - observability hooks)

```rust
// nebula-log/src/lib.rs
pub use metrics::{counter, gauge, histogram}; // re-export

pub mod observability {
    pub use super::hooks::{ObservabilityHook, ObservabilityEvent};
    pub use super::metrics::helpers; // convenience wrappers
}
```

## Архитектура

```
┌─────────────────────────────────────────────────────────┐
│                     Application                         │
└──────────────────┬──────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────┐
│                    nebula-log                           │
│  ┌──────────────┬──────────────┬──────────────┐        │
│  │   Logging    │   Tracing    │   Metrics    │        │
│  │  (tracing)   │    (OTEL)    │ (re-export)  │        │
│  └──────────────┴──────────────┴──────────────┘        │
│  ┌──────────────────────────────────────────┐          │
│  │       Observability Hooks                │          │
│  │  (event system, context propagation)     │          │
│  └──────────────────────────────────────────┘          │
└──────────────────┬──────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────┐
│              Standard `metrics` Crate                   │
│  ┌───────────┬───────────┬────────────┐                │
│  │  Counter  │   Gauge   │ Histogram  │                │
│  └───────────┴───────────┴────────────┘                │
└──────────────────┬──────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────┐
│                  Metric Exporters                       │
│  ┌──────────────┬──────────────┬──────────────┐        │
│  │  Prometheus  │     OTEL     │   StatsD     │        │
│  └──────────────┴──────────────┴──────────────┘        │
└─────────────────────────────────────────────────────────┘
```

## Предлагаемая Структура

### nebula-log (facade + coordination)

```
nebula-log/
├── src/
│   ├── lib.rs              # Re-exports, facade
│   ├── observability/      # NEW: Event system
│   │   ├── mod.rs
│   │   ├── hooks.rs        # ObservabilityHook trait
│   │   ├── events.rs       # Common event types
│   │   └── registry.rs     # Hook registration
│   └── metrics/            # NEW: Helpers для metrics crate
│       ├── mod.rs          # Re-export + convenience
│       ├── helpers.rs      # measure!, timed! macros
│       └── labels.rs       # Label management
```

### Каждый nebula crate

```rust
// Cargo.toml
[dependencies]
nebula-log = { path = "../nebula-log", optional = true }
metrics = { version = "0.21", optional = true }

[features]
observability = ["nebula-log/observability", "metrics"]
```

```rust
// src/lib.rs
#[cfg(feature = "observability")]
use metrics::{counter, gauge};

pub fn my_function() {
    #[cfg(feature = "observability")]
    counter!("nebula.validator.checks", 1);

    // ... logic ...
}
```

## API Дизайн

### Для Library Authors (nebula crates)

```rust
use nebula_log::observability::{emit_event, ObservabilityEvent};
use metrics::{counter, gauge, histogram};

// 1. Метрики (прямо через `metrics` crate)
counter!("resilience.retry.attempts", 1, "service" => "api");
gauge!("resilience.circuit_breaker.open", 1.0);
histogram!("resilience.operation.duration_ms", duration.as_millis() as f64);

// 2. Структурированные события (через nebula-log)
struct PatternEvent {
    pattern: String,
    operation: String,
    // ...
}

impl ObservabilityEvent for PatternEvent { ... }

emit_event(PatternEvent { ... });

// 3. Хелперы из nebula-log
use nebula_log::metrics::timed;

#[timed("operation.duration")]  // автоматический histogram
async fn my_operation() {
    // ...
}
```

### Для Application Developers

```rust
use nebula_log::{init_with, Config};

fn main() {
    // 1. Инициализация nebula-log
    nebula_log::init_with(
        Config::production()
            .with_observability()
    )?;

    // 2. Настройка Prometheus exporter
    metrics_exporter_prometheus::PrometheusBuilder::new()
        .install()
        .expect("failed to install Prometheus recorder");

    // Все nebula крейты теперь экспортируют метрики!
    // GET http://localhost:9000/metrics
}
```

## Migration Plan

### Phase 1: Add to nebula-log ✅

1. Добавить `metrics` как optional dependency
2. Создать `src/observability/` модуль
3. Создать `src/metrics/` с helpers
4. Обновить documentation

### Phase 2: Migrate nebula-resilience ✅

1. Удалить custom MetricsCollector
2. Использовать `metrics` crate
3. Переместить ObservabilityHook trait в nebula-log
4. Обновить примеры

### Phase 3: Enhance nebula-resource

1. Оставить `metrics` crate (уже есть!)
2. Добавить nebula-log observability events
3. Унифицировать naming conventions

### Phase 4: Optional для других crates

1. nebula-validator: validation metrics
2. nebula-memory: унифицировать с существующей stats системой
3. nebula-expression: evaluation metrics

## Naming Conventions (Важно!)

Единые правила именования метрик:

```rust
// Pattern: nebula.{crate}.{component}.{metric}

// Counters (total count)
counter!("nebula.resilience.retry.attempts_total");
counter!("nebula.validator.checks_total");
counter!("nebula.memory.allocations_total");

// Gauges (current value)
gauge!("nebula.resilience.circuit_breaker.open");
gauge!("nebula.memory.bytes_allocated");
gauge!("nebula.resource.pool.active_connections");

// Histograms (distribution)
histogram!("nebula.resilience.operation.duration_seconds");
histogram!("nebula.validator.check.duration_seconds");

// Labels
counter!("nebula.resilience.retry.attempts_total",
    "service" => "api",
    "pattern" => "retry"
);
```

## Benefits

### Для Разработчиков Nebula

1. ✅ **Не нужно изобретать велосипед** - используем стандарт
2. ✅ **Минимум кода** - просто `counter!()`, `gauge!()`
3. ✅ **Единый подход** - все крейты одинаково
4. ✅ **Optional** - feature-gated, zero overhead если не нужно

### Для Пользователей Nebula

1. ✅ **Одна точка экспорта** - все метрики в Prometheus
2. ✅ **Стандартный формат** - работает с Grafana, Datadog, etc.
3. ✅ **Минимальная настройка** - просто install exporter
4. ✅ **Production-ready** - проверенное решение

## Special Case: nebula-memory Stats

**Вопрос**: Что делать с мощной stats системой в nebula-memory?

**Ответ**: Hybrid approach

```rust
// nebula-memory продолжает иметь детальную stats систему
use nebula_memory::stats::{MemoryTracker, Aggregator};

// НО также экспортирует ключевые метрики через `metrics`
#[cfg(feature = "observability")]
{
    gauge!("nebula.memory.bytes_allocated", tracker.total_allocated());
    counter!("nebula.memory.allocations_total", tracker.allocation_count());
}
```

**Преимущества**:
- Детальная аналитика внутри nebula-memory (existing code)
- Ключевые метрики доступны через Prometheus (new)
- Лучшее из двух миров

## Implementation Details

### nebula-log/Cargo.toml

```toml
[dependencies]
# Existing
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = [...] }

# NEW: Metrics support
metrics = { version = "0.21", optional = true }

[features]
default = ["ansi", "async"]
observability = ["metrics"]
telemetry = ["opentelemetry", "opentelemetry_sdk", "tracing-opentelemetry", "observability"]
full = ["ansi", "async", "file", "log-compat", "telemetry", "observability"]
```

### nebula-log/src/observability/hooks.rs

```rust
use std::sync::Arc;
use std::time::SystemTime;

/// Event that can be emitted through observability system
pub trait ObservabilityEvent: Send + Sync {
    fn name(&self) -> &str;
    fn timestamp(&self) -> SystemTime;
}

/// Hook that receives observability events
pub trait ObservabilityHook: Send + Sync {
    fn on_event(&self, event: &dyn ObservabilityEvent);
}

/// Global registry for observability hooks
pub struct ObservabilityRegistry {
    hooks: Vec<Arc<dyn ObservabilityHook>>,
}

impl ObservabilityRegistry {
    pub fn register(&mut self, hook: Arc<dyn ObservabilityHook>) {
        self.hooks.push(hook);
    }

    pub fn emit(&self, event: &dyn ObservabilityEvent) {
        for hook in &self.hooks {
            hook.on_event(event);
        }
    }
}

// Global instance
static REGISTRY: once_cell::sync::Lazy<parking_lot::RwLock<ObservabilityRegistry>> =
    once_cell::sync::Lazy::new(|| parking_lot::RwLock::new(ObservabilityRegistry::default()));

pub fn register_hook(hook: Arc<dyn ObservabilityHook>) {
    REGISTRY.write().register(hook);
}

pub fn emit_event(event: &dyn ObservabilityEvent) {
    REGISTRY.read().emit(event);
}
```

### nebula-log/src/metrics/helpers.rs

```rust
#[cfg(feature = "observability")]
#[macro_export]
macro_rules! timed {
    ($name:expr, $block:expr) => {{
        let start = std::time::Instant::now();
        let result = $block;
        let duration = start.elapsed();
        metrics::histogram!($name, duration.as_secs_f64());
        result
    }};
}

#[cfg(feature = "observability")]
#[macro_export]
macro_rules! measure {
    ($name:expr, $value:expr) => {
        metrics::gauge!($name, $value);
    };
}
```

## Decision: nebula-log or separate nebula-metric?

### ✅ RECOMMENDATION: Extend nebula-log

**Причины**:
1. Один dependency для всей observability
2. Логичное место для coordination
3. Уже есть телеметрия и контекст
4. nebula-log используется везде
5. Меньше fragmentation

**НЕ создаем nebula-metric** потому что:
- Добавит еще одну зависимость
- Дублирование с nebula-log
- Усложнит интеграцию

## Next Steps

1. ✅ Согласовать архитектуру
2. Add `metrics` to nebula-log
3. Create `observability` module in nebula-log
4. Migrate nebula-resilience
5. Document integration guide
6. Add examples

---

**Status**: 🔄 Design Review
**Decision Needed**: Approve architecture before implementation
**Owner**: @vanyastaff
**Date**: 2025-10-12
