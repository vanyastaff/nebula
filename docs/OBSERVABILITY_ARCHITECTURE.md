# Nebula Observability Architecture

## Problem Statement

Currently, observability is fragmented across the nebula ecosystem:
- Each crate might need its own logging, metrics, and tracing
- Duplication of `LogLevel`, event types, and hook patterns
- No unified way to collect metrics across crates
- Manual instrumentation required in every crate

## Goals

1. **Unified API**: Single source of truth for observability primitives
2. **Zero Boilerplate**: Crates shouldn't repeat observability code
3. **Flexibility**: Support multiple backends (Prometheus, OTEL, custom)
4. **Performance**: Zero-cost when not used, minimal overhead when enabled
5. **Composability**: Easy to add observability to any nebula crate

## Proposed Architecture

### Option 1: Extend nebula-log ✅ RECOMMENDED

**Rationale**: nebula-log already has:
- OpenTelemetry integration (feature `telemetry`)
- Tracing infrastructure
- Configuration system
- Context propagation

**Changes needed**:
```
nebula-log/
├── src/
│   ├── lib.rs
│   ├── core/
│   ├── telemetry/
│   ├── metrics/          # NEW: Metrics collection
│   │   ├── mod.rs
│   │   ├── collector.rs  # MetricsCollector
│   │   ├── registry.rs   # Global metrics registry
│   │   └── export.rs     # Prometheus/OTEL export
│   └── observability/    # NEW: Unified hooks
│       ├── mod.rs
│       ├── hooks.rs      # ObservabilityHook trait
│       ├── events.rs     # Common event types
│       └── context.rs    # Event context
```

**Benefits**:
- Leverage existing OpenTelemetry integration
- Reuse tracing infrastructure
- Single dependency for all observability
- Already widely used across nebula crates

### Option 2: Create nebula-metric (separate crate)

**Structure**:
```
nebula-metric/
├── src/
│   ├── collector.rs      # Metrics collection
│   ├── registry.rs       # Global registry
│   ├── hooks.rs          # Hook system
│   └── export/
│       ├── prometheus.rs
│       └── otel.rs
```

**Drawbacks**:
- Another dependency to manage
- Duplication with nebula-log telemetry
- Need to coordinate between nebula-log and nebula-metric
- More complex integration story

## Recommended Solution: Enhanced nebula-log

### 1. Add Metrics Module to nebula-log

```rust
// nebula-log/src/metrics/mod.rs
pub mod collector;
pub mod registry;
pub mod export;

pub use collector::{MetricsCollector, Metric, MetricSnapshot};
pub use registry::{GlobalRegistry, register, collect_all};

// Optional exports based on features
#[cfg(feature = "prometheus")]
pub use export::prometheus::PrometheusExporter;

#[cfg(feature = "telemetry")]
pub use export::otel::OtelExporter;
```

### 2. Add Observability Hooks Module

```rust
// nebula-log/src/observability/mod.rs
pub trait ObservabilityEvent: Send + Sync {
    fn name(&self) -> &str;
    fn timestamp(&self) -> SystemTime;
    fn context(&self) -> &EventContext;
}

pub trait ObservabilityHook: Send + Sync {
    fn on_event(&self, event: &dyn ObservabilityEvent);
}

pub struct ObservabilityRegistry {
    hooks: Vec<Arc<dyn ObservabilityHook>>,
}

impl ObservabilityRegistry {
    pub fn register_hook(&mut self, hook: Arc<dyn ObservabilityHook>);
    pub fn emit<E: ObservabilityEvent>(&self, event: E);
}
```

### 3. Update Cargo.toml Features

```toml
[features]
default = ["ansi", "async"]
metrics = []  # NEW: Enable metrics collection
observability = ["metrics"]  # NEW: Full observability
prometheus = ["metrics"]  # NEW: Prometheus export
telemetry = ["opentelemetry", "opentelemetry_sdk", "tracing-opentelemetry"]
full = ["ansi", "async", "file", "telemetry", "observability", "prometheus"]
```

### 4. Unified Public API

```rust
// nebula-log exports
pub use metrics::{MetricsCollector, register_metric, collect_metrics};
pub use observability::{
    ObservabilityEvent,
    ObservabilityHook,
    ObservabilityRegistry,
    emit_event,
};

// Common event types
pub mod events {
    pub struct OperationStarted { ... }
    pub struct OperationCompleted { ... }
    pub struct OperationFailed { ... }
}
```

## Migration Plan for nebula-resilience

### Before (Current):

```rust
// nebula-resilience/src/observability/hooks.rs
pub enum LogLevel { Error, Warn, Info, Debug }
pub enum PatternEvent { Started, Succeeded, Failed, ... }
pub trait ObservabilityHook { ... }
pub struct LoggingHook { ... }
pub struct MetricsHook { ... }
```

### After (Using nebula-log):

```rust
// nebula-resilience/src/observability/mod.rs
use nebula_log::{ObservabilityEvent, ObservabilityHook, Level};
use nebula_log::events::{OperationStarted, OperationCompleted, OperationFailed};

// Resilience-specific events
pub enum PatternEvent {
    RetryAttempt { ... },
    CircuitBreakerStateChanged { ... },
    // etc - domain-specific events
}

impl ObservabilityEvent for PatternEvent { ... }

// No need to reimplement LoggingHook, MetricsHook - provided by nebula-log!
```

## Usage Across Nebula Crates

### Example: nebula-validator

```rust
use nebula_log::{emit_event, events::OperationStarted};
use nebula_log::metrics::register_metric;

pub fn validate<T>(&self, value: &T) -> Result<(), ValidationError> {
    emit_event(OperationStarted {
        operation: "validation",
        context: ...,
    });

    register_metric("validation.total", 1.0);

    // ... validation logic ...
}
```

### Example: nebula-memory

```rust
use nebula_log::metrics::{register_metric, collect_metrics};

impl BumpAllocator {
    pub fn allocate(&self, size: usize) -> *mut u8 {
        register_metric("memory.allocations", 1.0);
        register_metric("memory.bytes_allocated", size as f64);

        // ... allocation logic ...
    }
}
```

## Implementation Phases

### Phase 1: Add Metrics to nebula-log ✅
- [ ] Create `src/metrics/` module
- [ ] Implement `MetricsCollector` (from nebula-resilience)
- [ ] Add global registry
- [ ] Add basic Prometheus export

### Phase 2: Add Observability Hooks ✅
- [ ] Create `src/observability/` module
- [ ] Define `ObservabilityEvent` trait
- [ ] Define `ObservabilityHook` trait
- [ ] Implement global registry
- [ ] Add built-in hooks (logging, metrics)

### Phase 3: Migrate nebula-resilience ✅
- [ ] Remove local observability code
- [ ] Use nebula-log primitives
- [ ] Update examples
- [ ] Test integration

### Phase 4: Add to Other Crates
- [ ] nebula-validator: validation events
- [ ] nebula-memory: allocation metrics
- [ ] nebula-resource: pool metrics
- [ ] nebula-expression: evaluation metrics

## Benefits

1. **Single Source of Truth**: All observability in nebula-log
2. **No Duplication**: Crates use common primitives
3. **Easy Integration**: Just add hooks, emit events
4. **Unified Export**: One Prometheus endpoint for all metrics
5. **OpenTelemetry Ready**: Built-in OTEL integration
6. **Performance**: Global registry with lock-free access
7. **Flexibility**: Custom events + built-in common events

## API Design

### For Library Authors (nebula crates)

```rust
// 1. Define domain events
pub struct ValidationEvent { ... }
impl ObservabilityEvent for ValidationEvent { ... }

// 2. Emit events
emit_event(ValidationEvent { ... });

// 3. Record metrics
register_metric("validator.checks", 1.0);
```

### For Application Developers

```rust
use nebula_log::{init_with, Config, Level};
use nebula_log::observability::hooks::{LoggingHook, MetricsHook};

fn main() {
    // Initialize with observability
    let config = Config::production()
        .with_observability()
        .with_prometheus_exporter("0.0.0.0:9090");

    nebula_log::init_with(config)?;

    // All nebula crates now emit to unified system
    // Access metrics at http://localhost:9090/metrics
}
```

## Open Questions

1. **Should metrics be always-on or feature-gated?**
   - Proposal: Always-on but with minimal overhead when not collected

2. **Global vs per-crate registries?**
   - Proposal: Global with namespacing (e.g., "resilience.*", "memory.*")

3. **Event buffer size?**
   - Proposal: Configurable with sensible defaults (1000 events)

4. **Prometheus vs OTEL as primary?**
   - Proposal: Support both, Prometheus simpler for start

## Next Steps

1. Review this design with team
2. Start Phase 1: Add metrics module to nebula-log
3. Implement Phase 2: Observability hooks
4. Migrate nebula-resilience as proof of concept
5. Document integration guide for other crates

---

**Status**: 🔄 Design Review
**Owner**: @vanyastaff
**Last Updated**: 2025-10-12
