# Nebula Observability Migration Guide

**Version**: 1.0
**Last Updated**: October 2025
**Target Audience**: Maintainers of existing nebula crates

## Table of Contents

1. [Overview](#overview)
2. [Migration Path](#migration-path)
3. [Crate-Specific Guides](#crate-specific-guides)
4. [Breaking Changes](#breaking-changes)
5. [Testing Migration](#testing-migration)
6. [Rollback Plan](#rollback-plan)

---

## Overview

This guide helps migrate existing nebula crates from custom observability solutions to the unified system in `nebula-log`.

### Why Migrate?

**Benefits**:
- ✅ **Standardization** - Consistent observability across all crates
- ✅ **Zero-cost** - Lock-free, high-performance design
- ✅ **Panic-safe** - Hooks cannot crash your application
- ✅ **Composable** - Mix logging, metrics, and custom hooks
- ✅ **Future-proof** - Built for extensibility

**Migration Effort**: 2-4 hours per crate

---

## Migration Path

### Phase 1: Assessment (30 minutes)

**Checklist**:
- [ ] Identify all custom metrics in your crate
- [ ] List all logging/tracing calls
- [ ] Document custom observability hooks
- [ ] Check dependencies on `metrics` crate version

**Tools**:
```bash
# Find all metrics usage
rg "counter!|gauge!|histogram!" crates/your-crate/

# Find custom observability code
rg "struct.*Hook|impl.*Hook" crates/your-crate/
```

### Phase 2: Add Dependencies (15 minutes)

Update `Cargo.toml`:

```toml
[dependencies]
# Add unified observability
nebula-log = { path = "../nebula-log", features = ["observability"] }

# Update metrics version (if using)
metrics = "0.23"  # Was 0.21

# Remove if present (now in nebula-log)
# OLD: custom-metrics = "0.1"
```

### Phase 3: Migrate Events (1-2 hours)

**Before** (custom events):
```rust
// OLD: Custom event system
struct MyCustomEvent {
    data: String,
}

impl MyCustomEvent {
    fn emit(&self) {
        // Custom emission logic
        custom_logger::log(self);
    }
}
```

**After** (unified events):
```rust
// NEW: Implement ObservabilityEvent
use nebula_log::observability::ObservabilityEvent;

struct MyCustomEvent {
    data: String,
}

impl ObservabilityEvent for MyCustomEvent {
    fn name(&self) -> &str {
        "my_custom_event"
    }

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "data": self.data,
        }))
    }
}

// Usage
use nebula_log::observability::emit_event;
emit_event(&MyCustomEvent { data: "test".to_string() });
```

### Phase 4: Migrate Metrics (30 minutes)

Metrics API is **unchanged**, just update version:

```toml
[dependencies]
metrics = "0.23"  # Update from 0.21
```

**Naming Convention Change**:
```rust
// OLD: Inconsistent naming
counter!("requests").increment(1);
gauge!("memory").set(1024.0);

// NEW: Follow convention
counter!("nebula.mycrate.requests_total").increment(1);
gauge!("nebula.mycrate.memory_bytes").set(1024.0);
```

### Phase 5: Register Hooks (15 minutes)

**Before** (custom initialization):
```rust
// OLD: Custom hook registration
fn init_observability() {
    my_custom_logger::init();
    my_metrics_exporter::start();
}
```

**After** (unified registration):
```rust
// NEW: Use unified system
use nebula_log::observability::{register_hook, LoggingHook};
use std::sync::Arc;

fn init_observability() {
    register_hook(Arc::new(LoggingHook::default()));
    // Add more hooks as needed
}
```

### Phase 6: Testing (30-60 minutes)

```bash
# Run tests
cargo test -p your-crate

# Check metrics output
cargo run --example observability_demo

# Verify no regressions
cargo bench
```

---

## Crate-Specific Guides

### nebula-resource

**Current State**:
- Uses `metrics` 0.21
- Has custom `ResourceEvent` types
- Exports metrics for pool stats

**Migration Steps**:

1. **Update Dependencies**:
```toml
[dependencies]
nebula-log = { path = "../nebula-log", features = ["observability"] }
metrics = "0.23"
```

2. **Migrate Resource Events**:
```rust
// OLD
struct ResourceAcquired {
    pool_id: String,
}

impl ResourceAcquired {
    fn log(&self) {
        tracing::info!("Resource acquired: {}", self.pool_id);
    }
}

// NEW
use nebula_log::observability::ObservabilityEvent;

impl ObservabilityEvent for ResourceAcquired {
    fn name(&self) -> &str { "resource.acquired" }

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "pool_id": self.pool_id,
        }))
    }
}
```

3. **Update Metric Names**:
```rust
// OLD
counter!("resources_acquired").increment(1);
gauge!("pool_size").set(size as f64);

// NEW
counter!("nebula.resource.acquired_total",
    "pool_id" => pool_id
).increment(1);

gauge!("nebula.resource.pool.size",
    "pool_id" => pool_id
).set(size as f64);
```

### nebula-memory

**Current State**:
- Custom `AllocationStats` struct
- Manual metric export
- No event system

**Migration Steps**:

1. **Keep Existing Stats** (no breaking change):
```rust
// Keep this - it's still useful
pub struct AllocationStats {
    pub bytes_allocated: usize,
    pub bytes_freed: usize,
    pub active_allocations: usize,
}
```

2. **Add Observability Events**:
```rust
use nebula_log::observability::{emit_event, ObservabilityEvent};

struct AllocationEvent {
    size: usize,
    allocator: &'static str,
}

impl ObservabilityEvent for AllocationEvent {
    fn name(&self) -> &str { "memory.allocated" }

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "size": self.size,
            "allocator": self.allocator,
        }))
    }
}

// Emit on allocation
pub fn allocate(&mut self, size: usize) -> Result<*mut u8> {
    let ptr = self.do_allocate(size)?;

    emit_event(&AllocationEvent {
        size,
        allocator: "bump",
    });

    Ok(ptr)
}
```

3. **Export Metrics** (optional, for Prometheus):
```rust
pub fn update_metrics(&self) {
    gauge!("nebula.memory.bytes_allocated")
        .set(self.stats.bytes_allocated as f64);

    gauge!("nebula.memory.allocations_active")
        .set(self.stats.active_allocations as f64);
}
```

### nebula-validator

**Current State**:
- No observability
- Performance-critical code

**Migration Steps**:

1. **Add Optional Events** (feature-gated):
```rust
#[cfg(feature = "observability")]
use nebula_log::observability::emit_event;

pub fn validate(&self, input: &T) -> Result<()> {
    #[cfg(feature = "observability")]
    emit_event(&ValidationStarted {
        validator: std::any::type_name::<Self>(),
    });

    let result = self.do_validate(input);

    #[cfg(feature = "observability")]
    if result.is_err() {
        emit_event(&ValidationFailed {
            validator: std::any::type_name::<Self>(),
        });
    }

    result
}
```

2. **Add Metrics for Cache** (already performance-tracked):
```rust
impl CachedValidator {
    fn get_cached(&self, key: &Key) -> Option<Result> {
        let result = self.cache.get(key);

        counter!("nebula.validator.cache.requests",
            "result" => if result.is_some() { "hit" } else { "miss" }
        ).increment(1);

        result
    }
}
```

### nebula-resilience

**Current State**:
- Already migrated in Issue #38!
- Uses unified observability

**No Migration Needed** ✅

---

## Breaking Changes

### API Changes

**None** - This is an **additive migration**. Existing code continues to work.

### Dependency Changes

| Crate | Old Version | New Version | Breaking? |
|-------|-------------|-------------|-----------|
| `metrics` | 0.21 | 0.23 | No (compatible) |
| `tracing` | Any | Any | No |
| `nebula-log` | N/A | 0.1 | No (new dep) |

### Metric Name Changes

Recommended (but **not required**):

| Old Pattern | New Pattern |
|-------------|-------------|
| `requests` | `nebula.{crate}.requests_total` |
| `errors` | `nebula.{crate}.errors_total` |
| `memory` | `nebula.{crate}.memory_bytes` |

**Migration Timeline**: Can be gradual, no breaking changes.

---

## Testing Migration

### Unit Tests

Existing tests **should not break**. Add new tests for observability:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_log::observability::*;
    use std::sync::{Arc, Mutex};

    struct TestHook {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl ObservabilityHook for TestHook {
        fn on_event(&self, event: &dyn ObservabilityEvent) {
            self.events.lock().unwrap()
                .push(event.name().to_string());
        }
    }

    #[test]
    fn test_emits_events() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let hook = Arc::new(TestHook { events: events.clone() });

        register_hook(hook);

        // Your code that emits events
        my_function();

        // Verify events
        let captured = events.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0], "my_event");

        shutdown_hooks();
    }
}
```

### Integration Tests

Run existing integration tests - they should pass without changes:

```bash
# Run all tests
cargo test -p your-crate

# Run with observability enabled
cargo test -p your-crate --features observability

# Check examples
cargo run --example observability_demo
```

### Performance Tests

Verify no performance regression:

```bash
# Before migration
cargo bench -p your-crate > before.txt

# After migration
cargo bench -p your-crate > after.txt

# Compare
diff before.txt after.txt
```

**Expected overhead**: < 1% for most crates.

---

## Rollback Plan

If migration causes issues:

### Quick Rollback

1. **Revert Cargo.toml**:
```toml
[dependencies]
# Remove
# nebula-log = { path = "../nebula-log", features = ["observability"] }

# Restore old metrics version
metrics = "0.21"
```

2. **Revert Code Changes**:
```bash
git revert <migration-commit-hash>
```

3. **Verify Tests**:
```bash
cargo test -p your-crate
```

### Partial Rollback

Keep metrics update, remove events:

```rust
// Comment out event emissions
// emit_event(&MyEvent);

// Keep metrics
counter!("nebula.mycrate.requests").increment(1);
```

---

## Migration Checklist

### Pre-Migration
- [ ] Read this guide
- [ ] Review [OBSERVABILITY_GUIDE.md](./OBSERVABILITY_GUIDE.md)
- [ ] Review [OBSERVABILITY_BEST_PRACTICES.md](./OBSERVABILITY_BEST_PRACTICES.md)
- [ ] Identify all metrics and logging in your crate
- [ ] Create migration branch

### During Migration
- [ ] Update `Cargo.toml` dependencies
- [ ] Add `use nebula_log::observability::*;`
- [ ] Implement `ObservabilityEvent` for custom events
- [ ] Replace custom event emission with `emit_event()`
- [ ] Update metric names to follow convention
- [ ] Register hooks in initialization code
- [ ] Add observability to public API (if appropriate)

### Post-Migration
- [ ] Run all tests (`cargo test`)
- [ ] Run benchmarks (`cargo bench`)
- [ ] Test examples (`cargo run --example`)
- [ ] Update crate documentation
- [ ] Update CHANGELOG
- [ ] Create PR with migration
- [ ] Review with team

### Documentation
- [ ] Update crate README with observability section
- [ ] Document emitted events
- [ ] Document exported metrics
- [ ] Add example usage
- [ ] Update API docs

---

## Support

Need help with migration?

- **GitHub Issues**: https://github.com/vanyastaff/nebula/issues
- **Discussions**: https://github.com/vanyastaff/nebula/discussions
- **Slack**: #nebula-observability (if available)

---

## Examples

See fully migrated crates:
- ✅ `nebula-resilience` - Complete migration (Issue #38)
- ✅ `nebula-log` - Reference implementation
- 🚧 `nebula-resource` - In progress
- 🚧 `nebula-memory` - Planned

---

**Last Updated**: October 2025
**Maintainers**: Nebula Team
