# Nebula Observability Implementation Plan

## Overview

Унифицировать observability через nebula-log, используя стандартный `metrics` крейт как foundation.

## Dependencies Analysis

### New Dependencies для nebula-log

```toml
[dependencies]
# Existing (keep as is)
tracing = "0.1.41"
tracing-subscriber = { version = "0.3.20", features = [...] }
opentelemetry = { version = "0.30.0", optional = true }
# ... other existing deps

# NEW: Metrics support
metrics = { version = "0.23", optional = true }  # Latest stable
once_cell = "1.20"  # Already present

[dev-dependencies]
# NEW: For testing exporters
metrics-exporter-prometheus = "0.15"
```

### Why metrics 0.23?

- **Latest stable version** (0.21 в nebula-resource можно обновить)
- Backend-agnostic architecture
- Zero-cost abstractions
- Excellent ecosystem support

### Optional: Prometheus Exporter (для примеров)

```toml
[dev-dependencies]
metrics-exporter-prometheus = "0.15"  # Для examples
```

## Sprint Planning

### Sprint 1: Foundation (3-4 дня)

**Goal**: Add metrics infrastructure to nebula-log

**Tasks**:
1. Add `metrics` dependency to nebula-log
2. Create `src/metrics/` module structure
3. Add re-exports and basic helpers
4. Add feature flags
5. Update documentation
6. Create basic example

**Deliverables**:
- ✅ `nebula-log/src/metrics/mod.rs`
- ✅ `nebula-log/src/metrics/helpers.rs`
- ✅ Feature `observability` added
- ✅ Example: `examples/metrics_basic.rs`
- ✅ Tests

**Estimated effort**: 4-6 hours

---

### Sprint 2: Observability Hooks (3-4 дня)

**Goal**: Add event system and hooks to nebula-log

**Tasks**:
1. Create `src/observability/` module
2. Define `ObservabilityEvent` trait
3. Define `ObservabilityHook` trait
4. Implement global registry
5. Add built-in hooks (logging, metrics)
6. Add tests and examples

**Deliverables**:
- ✅ `nebula-log/src/observability/mod.rs`
- ✅ `nebula-log/src/observability/hooks.rs`
- ✅ `nebula-log/src/observability/events.rs`
- ✅ `nebula-log/src/observability/registry.rs`
- ✅ Example: `examples/observability_hooks.rs`
- ✅ Tests

**Estimated effort**: 6-8 hours

---

### Sprint 3: Migrate nebula-resilience (2-3 дня)

**Goal**: Remove custom observability, use nebula-log

**Tasks**:
1. Remove `src/observability/hooks.rs` custom code
2. Use `metrics` crate instead of MetricsCollector
3. Use nebula-log ObservabilityHook trait
4. Update examples
5. Update tests
6. Update documentation

**Deliverables**:
- ✅ Removed custom MetricsCollector
- ✅ Using `metrics!()` macros
- ✅ Using nebula-log hooks
- ✅ Updated `observability_demo.rs`
- ✅ All tests passing

**Estimated effort**: 4-6 hours

---

### Sprint 4: Documentation & Polish (1-2 дня)

**Goal**: Complete documentation and examples

**Tasks**:
1. Write integration guide
2. Add migration guide for other crates
3. Create comprehensive examples
4. Add Prometheus dashboard examples
5. Update CHANGELOG

**Deliverables**:
- ✅ `docs/OBSERVABILITY_GUIDE.md`
- ✅ `docs/MIGRATION_GUIDE.md`
- ✅ Example: `examples/prometheus_integration.rs`
- ✅ Example: `examples/multi_crate_observability.rs`

**Estimated effort**: 3-4 hours

---

## Total Timeline

**Overall**: ~10-12 дней (спринты могут идти параллельно частично)

**Critical Path**:
Sprint 1 → Sprint 2 → Sprint 3 → Sprint 4

## Detailed Task Breakdown

### Sprint 1: Foundation

#### Task 1.1: Add Dependencies
```toml
# nebula-log/Cargo.toml
[dependencies]
metrics = { version = "0.23", optional = true }

[features]
observability = ["metrics"]
full = [..., "observability"]
```

**Effort**: 15 min

#### Task 1.2: Create Metrics Module

**File**: `nebula-log/src/metrics/mod.rs`
```rust
//! Metrics collection using standard metrics crate

#[cfg(feature = "observability")]
pub use metrics::{counter, gauge, histogram, describe_counter, describe_gauge, describe_histogram};

pub mod helpers;

#[cfg(feature = "observability")]
pub use helpers::{measure, timed_block};
```

**Effort**: 30 min

#### Task 1.3: Create Helpers

**File**: `nebula-log/src/metrics/helpers.rs`
```rust
/// Measure a value with automatic labeling
#[macro_export]
macro_rules! measure {
    ($name:expr, $value:expr) => {
        #[cfg(feature = "observability")]
        metrics::gauge!($name, $value);
    };
    ($name:expr, $value:expr, $($labels:tt)*) => {
        #[cfg(feature = "observability")]
        metrics::gauge!($name, $value, $($labels)*);
    };
}

/// Time a block of code
pub fn timed_block<F, R>(name: &str, f: F) -> R
where
    F: FnOnce() -> R,
{
    #[cfg(feature = "observability")]
    let _guard = TimingGuard::new(name);

    f()
}

#[cfg(feature = "observability")]
struct TimingGuard {
    name: String,
    start: std::time::Instant,
}

#[cfg(feature = "observability")]
impl TimingGuard {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            start: std::time::Instant::now(),
        }
    }
}

#[cfg(feature = "observability")]
impl Drop for TimingGuard {
    fn drop(&mut self) {
        let duration = self.start.elapsed();
        metrics::histogram!(&self.name, duration.as_secs_f64());
    }
}
```

**Effort**: 1 hour

#### Task 1.4: Update lib.rs

**File**: `nebula-log/src/lib.rs`
```rust
// Add after existing modules
#[cfg(feature = "observability")]
pub mod metrics;

// Add to prelude
pub mod prelude {
    // ... existing
    #[cfg(feature = "observability")]
    pub use crate::metrics::{counter, gauge, histogram, measure, timed_block};
}
```

**Effort**: 15 min

#### Task 1.5: Create Example

**File**: `nebula-log/examples/metrics_basic.rs`
```rust
use nebula_log::metrics::{counter, gauge, histogram};

fn main() {
    // Setup Prometheus exporter
    metrics_exporter_prometheus::PrometheusBuilder::new()
        .install()
        .expect("failed to install Prometheus recorder");

    // Use metrics
    counter!("requests_total", 1);
    gauge!("temperature_celsius", 23.5);
    histogram!("request_duration_seconds", 0.42);

    // With labels
    counter!("http_requests_total", 1,
        "method" => "GET",
        "status" => "200"
    );

    println!("Metrics available at http://localhost:9000/metrics");
}
```

**Effort**: 30 min

#### Task 1.6: Tests

**File**: `nebula-log/src/metrics/helpers.rs` (add tests)
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timed_block() {
        let result = timed_block("test_operation", || {
            std::thread::sleep(std::time::Duration::from_millis(10));
            42
        });
        assert_eq!(result, 42);
    }

    #[test]
    fn test_measure_macro() {
        measure!("test_metric", 123.45);
        measure!("test_metric_with_labels", 123.45, "label" => "value");
    }
}
```

**Effort**: 30 min

---

### Sprint 2: Observability Hooks

#### Task 2.1: Create Module Structure

**Files to create**:
- `nebula-log/src/observability/mod.rs`
- `nebula-log/src/observability/hooks.rs`
- `nebula-log/src/observability/events.rs`
- `nebula-log/src/observability/registry.rs`

**Effort**: 15 min

#### Task 2.2: Define Traits

**File**: `nebula-log/src/observability/hooks.rs`
```rust
use std::sync::Arc;
use std::time::SystemTime;

/// Event that can be emitted through observability system
pub trait ObservabilityEvent: Send + Sync {
    /// Event name for identification
    fn name(&self) -> &str;

    /// When the event occurred
    fn timestamp(&self) -> SystemTime {
        SystemTime::now()
    }

    /// Optional: serialize event data for structured logging
    fn data(&self) -> Option<serde_json::Value> {
        None
    }
}

/// Hook that receives observability events
pub trait ObservabilityHook: Send + Sync {
    /// Called when an event occurs
    fn on_event(&self, event: &dyn ObservabilityEvent);

    /// Optional: initialize hook
    fn initialize(&self) {}

    /// Optional: shutdown hook
    fn shutdown(&self) {}
}
```

**Effort**: 1 hour

#### Task 2.3: Implement Registry

**File**: `nebula-log/src/observability/registry.rs`
```rust
use std::sync::Arc;
use parking_lot::RwLock;
use once_cell::sync::Lazy;
use super::hooks::{ObservabilityEvent, ObservabilityHook};

/// Global registry for observability hooks
pub struct ObservabilityRegistry {
    hooks: Vec<Arc<dyn ObservabilityHook>>,
}

impl ObservabilityRegistry {
    fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Register a new hook
    pub fn register(&mut self, hook: Arc<dyn ObservabilityHook>) {
        hook.initialize();
        self.hooks.push(hook);
    }

    /// Emit an event to all registered hooks
    pub fn emit(&self, event: &dyn ObservabilityEvent) {
        for hook in &self.hooks {
            hook.on_event(event);
        }
    }

    /// Shutdown all hooks
    pub fn shutdown(&mut self) {
        for hook in &self.hooks {
            hook.shutdown();
        }
        self.hooks.clear();
    }
}

static REGISTRY: Lazy<RwLock<ObservabilityRegistry>> =
    Lazy::new(|| RwLock::new(ObservabilityRegistry::new()));

/// Register a global observability hook
pub fn register_hook(hook: Arc<dyn ObservabilityHook>) {
    REGISTRY.write().register(hook);
}

/// Emit an event to all registered hooks
pub fn emit_event(event: &dyn ObservabilityEvent) {
    REGISTRY.read().emit(event);
}

/// Shutdown all registered hooks
pub fn shutdown_hooks() {
    REGISTRY.write().shutdown();
}
```

**Effort**: 1.5 hours

#### Task 2.4: Built-in Hooks

**File**: `nebula-log/src/observability/hooks.rs` (add built-in hooks)
```rust
/// Built-in hook that logs events using tracing
pub struct LoggingHook {
    level: tracing::Level,
}

impl LoggingHook {
    pub fn new(level: tracing::Level) -> Self {
        Self { level }
    }
}

impl ObservabilityHook for LoggingHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        match self.level {
            tracing::Level::ERROR => tracing::error!(event = event.name(), "observability event"),
            tracing::Level::WARN => tracing::warn!(event = event.name(), "observability event"),
            tracing::Level::INFO => tracing::info!(event = event.name(), "observability event"),
            tracing::Level::DEBUG => tracing::debug!(event = event.name(), "observability event"),
            tracing::Level::TRACE => tracing::trace!(event = event.name(), "observability event"),
        }
    }
}

/// Built-in hook that records events as metrics
#[cfg(feature = "observability")]
pub struct MetricsHook;

#[cfg(feature = "observability")]
impl MetricsHook {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "observability")]
impl ObservabilityHook for MetricsHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        metrics::counter!(format!("nebula.events.{}", event.name()), 1);
    }
}
```

**Effort**: 1 hour

#### Task 2.5: Common Events

**File**: `nebula-log/src/observability/events.rs`
```rust
use super::hooks::ObservabilityEvent;
use std::time::{Duration, SystemTime};

/// Operation started event
#[derive(Debug, Clone)]
pub struct OperationStarted {
    pub operation: String,
    pub context: String,
}

impl ObservabilityEvent for OperationStarted {
    fn name(&self) -> &str {
        "operation_started"
    }
}

/// Operation completed successfully
#[derive(Debug, Clone)]
pub struct OperationCompleted {
    pub operation: String,
    pub duration: Duration,
}

impl ObservabilityEvent for OperationCompleted {
    fn name(&self) -> &str {
        "operation_completed"
    }
}

/// Operation failed
#[derive(Debug, Clone)]
pub struct OperationFailed {
    pub operation: String,
    pub error: String,
    pub duration: Duration,
}

impl ObservabilityEvent for OperationFailed {
    fn name(&self) -> &str {
        "operation_failed"
    }
}
```

**Effort**: 30 min

#### Task 2.6: Tests & Example

**Tests**: Add to each module
**Example**: `nebula-log/examples/observability_hooks.rs`

**Effort**: 1.5 hours

---

### Sprint 3: Migrate nebula-resilience

#### Task 3.1: Update Dependencies

**File**: `nebula-resilience/Cargo.toml`
```toml
[dependencies]
nebula-log = { path = "../nebula-log", features = ["observability"] }
metrics = "0.23"

# Remove if exists:
# - Custom metrics code
```

**Effort**: 15 min

#### Task 3.2: Remove Custom Code

**Files to modify/delete**:
- Delete: `src/observability/hooks.rs` (custom MetricsCollector)
- Keep: `src/observability/spans.rs` (still useful)
- Modify: `src/observability/mod.rs`

**Effort**: 30 min

#### Task 3.3: Use nebula-log Primitives

**File**: `nebula-resilience/src/observability/mod.rs`
```rust
//! Resilience pattern observability

// Re-export from nebula-log
pub use nebula_log::observability::{
    ObservabilityEvent,
    ObservabilityHook,
    emit_event,
    register_hook,
};

// Domain-specific events
pub mod events;
pub use events::PatternEvent;

// Keep useful utilities
pub mod spans;
pub use spans::{SpanGuard, create_span};
```

**Effort**: 1 hour

#### Task 3.4: Update Examples

**File**: `nebula-resilience/examples/observability_demo.rs`
- Use `metrics::counter!()` instead of MetricsHook
- Use nebula-log hooks
- Show Prometheus integration

**Effort**: 1 hour

#### Task 3.5: Update Tests

Run all tests, fix any breakage.

**Effort**: 1 hour

---

### Sprint 4: Documentation

#### Task 4.1: Integration Guide

**File**: `docs/OBSERVABILITY_GUIDE.md`
- How to add observability to nebula crates
- Examples for each crate type
- Best practices
- Naming conventions

**Effort**: 2 hours

#### Task 4.2: Migration Guide

**File**: `docs/OBSERVABILITY_MIGRATION.md`
- For nebula-resource (update metrics version)
- For nebula-memory (add metrics export)
- For other crates

**Effort**: 1 hour

#### Task 4.3: Examples

Create comprehensive examples showing:
- Basic metrics
- Observability hooks
- Prometheus integration
- Multi-crate setup

**Effort**: 1.5 hours

---

## GitHub Issues Structure

### Issue #1: [OBSERVABILITY] Add metrics support to nebula-log
**Labels**: enhancement, observability, nebula-log
**Sprint**: 1
**Tasks**:
- [ ] Add `metrics` dependency
- [ ] Create `src/metrics/` module
- [ ] Add helpers and macros
- [ ] Add tests
- [ ] Create example

### Issue #2: [OBSERVABILITY] Add observability hooks to nebula-log
**Labels**: enhancement, observability, nebula-log
**Sprint**: 2
**Tasks**:
- [ ] Create `src/observability/` module
- [ ] Define ObservabilityEvent trait
- [ ] Define ObservabilityHook trait
- [ ] Implement global registry
- [ ] Add built-in hooks
- [ ] Add tests and examples

### Issue #3: [OBSERVABILITY] Migrate nebula-resilience to use nebula-log observability
**Labels**: refactor, observability, nebula-resilience
**Sprint**: 3
**Dependencies**: #1, #2
**Tasks**:
- [ ] Remove custom MetricsCollector
- [ ] Use metrics crate
- [ ] Use nebula-log hooks
- [ ] Update examples
- [ ] Update tests

### Issue #4: [OBSERVABILITY] Documentation and examples
**Labels**: documentation, observability
**Sprint**: 4
**Dependencies**: #1, #2, #3
**Tasks**:
- [ ] Write integration guide
- [ ] Write migration guide
- [ ] Create comprehensive examples
- [ ] Update CHANGELOG

---

## Risk Assessment

### Low Risk
- Adding metrics dependency (mature crate)
- Creating new modules (additive changes)

### Medium Risk
- Migrating nebula-resilience (breaking changes)
  - **Mitigation**: Keep old code temporarily, feature flag

### High Risk
- None identified

---

## Success Criteria

✅ Sprint 1:
- `metrics` crate integrated
- Feature `observability` works
- Example compiles and runs

✅ Sprint 2:
- ObservabilityHook trait defined
- Registry implemented
- Built-in hooks work

✅ Sprint 3:
- nebula-resilience uses nebula-log
- All tests pass
- Examples updated

✅ Sprint 4:
- Documentation complete
- Migration guide ready
- Examples work

---

## Next Steps

1. **Review this plan** - согласовать подход
2. **Create GitHub issues** - создать 4 issue
3. **Start Sprint 1** - начать с metrics module
4. **Iterate** - по мере реализации уточнять

---

**Status**: 📋 Ready for Review
**Owner**: @vanyastaff
**Created**: 2025-10-12
**Estimated Total Effort**: 18-24 hours
