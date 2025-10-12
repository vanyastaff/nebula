# nebula-log Technical Debt & Refactoring Plan

**Status**: 📋 Planning Phase
**Created**: 2025-10-12
**Last Updated**: 2025-10-12

## Executive Summary

After implementing observability features (Sprint 1-2), the codebase has accumulated technical debt that needs addressing. This document outlines issues, proposes solutions, and defines a refactoring roadmap.

**Current State**:
- 📊 ~2,247 lines of code (22 files)
- 📦 20 dependencies (19 optional)
- 🎯 7 feature flags
- 🧪 17 tests passing
- ⚠️ 3 major areas of concern

---

## 1. Critical Issues

### 1.1 Module Organization & Visibility

**Problem**: Inconsistent module visibility and organization

**Issues**:
- ❌ `observability` module is always public (line 40, lib.rs) but should be feature-gated
- ❌ `metrics` module is feature-gated but `observability` depends on it conceptually
- ❌ Prelude exports observability types unconditionally (lines 65-71, lib.rs)
- ❌ No clear separation between core logging and observability features

**Current Code**:
```rust
// lib.rs line 35-40
#[cfg(feature = "observability")]
pub mod metrics;

// Observability module
pub mod observability;  // ❌ Always visible!
```

**Impact**:
- Users without `observability` feature get unused APIs
- Confusion about which features require which flags
- Larger API surface than necessary

**Recommended Fix**:
```rust
// Option A: Make observability conditional
#[cfg(feature = "observability")]
pub mod metrics;

#[cfg(feature = "observability")]
pub mod observability;

// Option B: Keep base observability, gate metrics integration
pub mod observability;  // Basic events/hooks
#[cfg(feature = "observability")]
mod observability_metrics;  // Metrics integration
```

**Priority**: 🔴 High
**Effort**: 2-3 hours

---

### 1.2 Builder Complexity

**Problem**: `builder.rs` is 537 lines with duplicated code patterns

**Issues**:
- ❌ Separate methods for each format × reloadable combination (8 methods!)
- ❌ Code duplication in `build_reloadable_*` and `build_static_*` methods
- ❌ Complex match statement with 8 branches (lines 94-110)
- ❌ Telemetry setup mixed with layer building
- ❌ Poor separation of concerns

**Current Structure**:
```rust
impl LoggerBuilder {
    build_reloadable_pretty()    // ~60 lines
    build_reloadable_compact()   // ~60 lines
    build_reloadable_json()      // ~60 lines
    build_static_pretty()        // ~60 lines
    build_static_compact()       // ~60 lines
    build_static_json()          // ~60 lines
    // + telemetry, sentry setup...
}
```

**Impact**:
- Difficult to maintain and test
- High cyclomatic complexity
- Adding new formats requires 2 new methods

**Recommended Fix**:
1. Extract format layer creation to separate functions
2. Extract reload handling to a trait/strategy pattern
3. Use builder pattern for telemetry setup

**Priority**: 🟡 Medium
**Effort**: 4-6 hours

---

### 1.3 Config Module Size

**Problem**: `config.rs` is 334 lines mixing concerns

**Issues**:
- ❌ Contains config structs, builders, defaults, and presets
- ❌ Multiple responsibilities in one file
- ❌ Telemetry config tightly coupled
- ❌ Hard to find specific configuration types

**Structure**:
```
config.rs (334 lines)
├── Config struct
├── Format enum
├── Level enum
├── WriterConfig enum
├── DisplayConfig struct
├── Fields struct
├── TelemetryConfig struct
├── SentryConfig struct
├── OpenTelemetryConfig struct
├── Config::default()
├── Config::development()
├── Config::production()
├── Config::test()
└── Config::from_env()
```

**Recommended Structure**:
```
config/
├── mod.rs           - Re-exports
├── base.rs          - Config, Format, Level
├── writer.rs        - WriterConfig, DisplayConfig
├── fields.rs        - Fields
├── telemetry.rs     - TelemetryConfig
├── presets.rs       - development(), production(), test()
└── builder.rs       - Config builder pattern
```

**Priority**: 🟡 Medium
**Effort**: 3-4 hours

---

## 2. Code Quality Issues

### 2.1 Missing Documentation

**Issues**:
- ⚠️ `observability` module lacks module-level docs explaining architecture
- ⚠️ No examples in metrics module beyond basic usage
- ⚠️ Missing "when to use X vs Y" guidance
- ⚠️ No migration guide from old logging patterns

**Files Needing Docs**:
- `src/metrics/mod.rs` - missing architecture overview
- `src/observability/mod.rs` - has basic docs but needs more context
- `src/lib.rs` - prelude exports need better docs

**Priority**: 🟢 Low
**Effort**: 2-3 hours

---

### 2.2 Test Coverage Gaps

**Issues**:
- ⚠️ No integration tests for observability + metrics together
- ⚠️ No tests for feature flag combinations
- ⚠️ No tests for concurrent hook registration
- ⚠️ Missing tests for edge cases (empty events, null data, etc.)

**Missing Coverage**:
```
- [ ] Observability + Metrics integration
- [ ] Multiple hooks firing on same event
- [ ] Hook errors/panics don't crash system
- [ ] Memory leaks in registry
- [ ] Feature flag combinations
```

**Priority**: 🟡 Medium
**Effort**: 3-4 hours

---

### 2.3 Naming Consistency

**Issues**:
- ⚠️ `timed_block` vs `TimingGuard` (inconsistent naming style)
- ⚠️ `emit_event` vs `register_hook` (verb vs verb+noun)
- ⚠️ `MetricsHook` vs `LoggingHook` (both are built-in hooks)
- ⚠️ `ObservabilityEvent` is verbose (maybe just `Event`?)

**Recommended Naming**:
```rust
// Current
timed_block() + TimingGuard
emit_event() + register_hook()

// Proposed
time_block() + TimingGuard  OR  timed_block() + TimedGuard
emit() + register()  OR  emit_event() + register_hook() ✓ (keep)
```

**Priority**: 🟢 Low
**Effort**: 1-2 hours

---

## 3. Architecture Issues

### 3.1 Feature Flag Dependencies

**Problem**: Complex feature flag relationships not clearly documented

**Current State**:
```toml
observability = ["metrics"]
telemetry = ["opentelemetry", "...", "observability"]
full = ["ansi", "async", "file", "log-compat", "telemetry", "sentry", "observability"]
```

**Issues**:
- ❌ `telemetry` implicitly enables `observability` (not obvious)
- ❌ No clear documentation of what each flag provides
- ❌ Users might enable `observability` without understanding metrics dependency

**Recommended**:
1. Document feature flags in README
2. Create feature flag diagram showing dependencies
3. Consider splitting `observability` into sub-features:
   ```toml
   observability-hooks = []  # Just events/hooks
   observability-metrics = ["metrics", "observability-hooks"]
   observability = ["observability-hooks", "observability-metrics"]  # Convenience
   ```

**Priority**: 🟡 Medium
**Effort**: 2-3 hours

---

### 3.2 Global State Management

**Problem**: Multiple global statics without coordination

**Current Globals**:
```rust
// lib.rs
static TEST_INIT: std::sync::OnceLock<()>

// observability/registry.rs
static REGISTRY: Lazy<RwLock<ObservabilityRegistry>>
```

**Issues**:
- ⚠️ No clear ownership model
- ⚠️ Potential for initialization races
- ⚠️ Testing is harder with global state

**Recommended**:
1. Document global state in module docs
2. Add `reset()` methods for testing (feature-gated)
3. Consider dependency injection alternative for registry

**Priority**: 🟢 Low
**Effort**: 2-3 hours

---

### 3.3 Error Handling Inconsistency

**Problem**: Mix of error handling styles

**Issues**:
- ⚠️ Some functions return `Result<T, LogError>`
- ⚠️ Some functions panic
- ⚠️ Hook errors are silently ignored
- ⚠️ No error callbacks for hooks

**Example**:
```rust
// registry.rs - silently continues on hook error
pub fn emit(&self, event: &dyn ObservabilityEvent) {
    for hook in &self.hooks {
        hook.on_event(event);  // What if this panics?
    }
}
```

**Recommended**:
1. Document panic vs error boundaries
2. Add `catch_unwind` for hook callbacks
3. Add optional error hook for debugging

**Priority**: 🟡 Medium
**Effort**: 2-3 hours

---

## 4. Performance Concerns

### 4.1 Allocation in Hot Paths

**Issues**:
- ⚠️ `MetricsHook` allocates String on every event (line 204, hooks.rs)
- ⚠️ `format!()` in hot path for metric names
- ⚠️ Events pass data via `Option<serde_json::Value>` (allocates)

**Current Code**:
```rust
// hooks.rs line 202-206
fn on_event(&self, event: &dyn ObservabilityEvent) {
    let metric_name = format!("nebula.events.{}", event.name());  // ❌ Allocates
    crate::metrics::counter!(metric_name).increment(1);
}
```

**Recommended**:
```rust
// Option A: Use const metric names
fn on_event(&self, event: &dyn ObservabilityEvent) {
    crate::metrics::counter!("nebula.events", "type" => event.name()).increment(1);
}

// Option B: Cache metric handles
struct MetricsHook {
    cache: DashMap<&'static str, Counter>,
}
```

**Priority**: 🟢 Low (premature optimization?)
**Effort**: 2-3 hours

---

### 4.2 Lock Contention

**Issue**: Global registry uses `RwLock` which could contend under high load

**Current**:
```rust
static REGISTRY: Lazy<RwLock<ObservabilityRegistry>>

pub fn emit_event(event: &dyn ObservabilityEvent) {
    REGISTRY.read().emit(event);  // Read lock on every event
}
```

**Analysis**:
- ✅ Read locks are generally fast
- ✅ Hook list rarely changes after init
- ⚠️ Could be issue with 1000s of events/sec

**Recommended** (if needed):
1. Use `Arc<Vec<Arc<dyn Hook>>>` for lock-free reads
2. Only lock during registration
3. Benchmark before optimizing

**Priority**: 🟢 Low
**Effort**: 3-4 hours

---

## 5. Technical Debt Summary

### By Priority

**🔴 High Priority** (Do First):
1. Module Organization & Visibility (2-3h)

**🟡 Medium Priority** (Do Next):
1. Builder Complexity (4-6h)
2. Config Module Size (3-4h)
3. Test Coverage Gaps (3-4h)
4. Feature Flag Dependencies (2-3h)
5. Error Handling Inconsistency (2-3h)

**🟢 Low Priority** (Nice to Have):
1. Missing Documentation (2-3h)
2. Naming Consistency (1-2h)
3. Global State Management (2-3h)
4. Allocation in Hot Paths (2-3h)
5. Lock Contention (3-4h)

### Total Effort Estimate
- High: 2-3 hours
- Medium: 18-23 hours
- Low: 13-17 hours
- **Total: 33-43 hours (~1 week)**

---

## 6. Refactoring Roadmap

### Phase 1: Quick Wins (Week 1, Days 1-2)
**Goal**: Fix critical visibility issues and improve structure

**Tasks**:
- [ ] Fix module visibility (observability feature-gating)
- [ ] Update prelude exports to be conditional
- [ ] Add feature flag documentation to README
- [ ] Add module-level docs to observability

**Deliverables**:
- Clean API surface
- Clear feature flag semantics
- Better documentation

**Effort**: 6-8 hours

---

### Phase 2: Builder Refactoring (Week 1, Days 3-4)
**Goal**: Simplify builder.rs and improve maintainability

**Tasks**:
- [ ] Extract format layer creation to separate module
- [ ] Create `FormatLayerBuilder` trait
- [ ] Consolidate reloadable vs static logic
- [ ] Extract telemetry setup to separate module
- [ ] Add builder tests

**Deliverables**:
- `builder.rs` < 300 lines
- Easier to add new formats
- Better test coverage

**Effort**: 8-10 hours

---

### Phase 3: Config Restructuring (Week 1, Day 5)
**Goal**: Split config.rs into logical submodules

**Tasks**:
- [ ] Create `config/` directory
- [ ] Split into base, writer, fields, telemetry, presets
- [ ] Update imports across codebase
- [ ] Add builder pattern for Config
- [ ] Update examples

**Deliverables**:
- Modular config system
- Easier to extend
- Better organization

**Effort**: 6-8 hours

---

### Phase 4: Error Handling & Testing (Week 2, Days 1-2)
**Goal**: Improve error handling and test coverage

**Tasks**:
- [ ] Add `catch_unwind` for hook callbacks
- [ ] Add error hook for debugging
- [ ] Write integration tests for observability + metrics
- [ ] Test feature flag combinations
- [ ] Add concurrent hook registration tests
- [ ] Document error boundaries

**Deliverables**:
- Robust error handling
- 80%+ test coverage
- Clear error semantics

**Effort**: 8-10 hours

---

### Phase 5: Polish & Documentation (Week 2, Days 3-4)
**Goal**: Final cleanup and documentation

**Tasks**:
- [ ] Review and fix naming inconsistencies
- [ ] Add comprehensive module docs
- [ ] Create migration guide
- [ ] Add architecture diagrams
- [ ] Benchmark hot paths
- [ ] Performance optimization (if needed)

**Deliverables**:
- Complete documentation
- Performance baseline
- Clean, maintainable codebase

**Effort**: 8-10 hours

---

## 7. Sprint Breakdown

### Sprint 1: Critical Fixes & Module Organization (2 days)
**Goal**: Fix visibility issues and clean up module structure

**Tasks**:
1. Feature-gate observability module properly
2. Fix prelude exports
3. Document feature flags
4. Add module-level documentation
5. Update examples

**Definition of Done**:
- ✅ `observability` module only available with feature flag
- ✅ Prelude exports are conditional
- ✅ Feature flags documented in README
- ✅ All tests passing
- ✅ No breaking changes to public API

**Estimated Effort**: 6-8 hours

---

### Sprint 2: Builder Refactoring (3-4 days)
**Goal**: Simplify and modularize builder.rs

**Tasks**:
1. Create `builder/` submodule
2. Extract format layer builders
3. Create `FormatLayerBuilder` abstraction
4. Consolidate reload logic
5. Extract telemetry setup
6. Add builder tests

**Definition of Done**:
- ✅ `builder.rs` < 300 lines (currently 537)
- ✅ Each format has dedicated builder
- ✅ No code duplication between reloadable/static
- ✅ 100% test coverage for builder logic
- ✅ All existing tests passing

**Estimated Effort**: 8-10 hours

---

### Sprint 3: Config Restructuring (2 days)
**Goal**: Split config into logical submodules

**Tasks**:
1. Create `config/` directory structure
2. Split into 5 files (base, writer, fields, telemetry, presets)
3. Update all imports
4. Add Config builder pattern
5. Update examples and tests

**Definition of Done**:
- ✅ Config in `config/` submodule
- ✅ Each file < 150 lines
- ✅ Builder pattern for Config
- ✅ All tests passing
- ✅ Examples updated

**Estimated Effort**: 6-8 hours

---

### Sprint 4: Error Handling & Testing (3 days)
**Goal**: Improve error handling and test coverage

**Tasks**:
1. Add panic safety to hook callbacks
2. Add error hook for debugging
3. Write 10+ integration tests
4. Test all feature flag combinations
5. Document error boundaries

**Definition of Done**:
- ✅ Hooks can't crash the system
- ✅ Error hook available for debugging
- ✅ 80%+ test coverage
- ✅ All feature combinations tested
- ✅ Error handling documented

**Estimated Effort**: 8-10 hours

---

### Sprint 5: Polish & Performance (2 days)
**Goal**: Final cleanup and optimization

**Tasks**:
1. Fix naming inconsistencies
2. Add comprehensive docs
3. Create migration guide
4. Benchmark and optimize hot paths
5. Final code review

**Definition of Done**:
- ✅ Consistent naming throughout
- ✅ All public APIs documented
- ✅ Migration guide complete
- ✅ Performance benchmarks established
- ✅ No clippy warnings

**Estimated Effort**: 8-10 hours

---

## 8. Migration Strategy

### Breaking vs Non-Breaking Changes

**Non-Breaking** (Can do immediately):
- ✅ Add feature-gated modules
- ✅ Add documentation
- ✅ Add tests
- ✅ Internal refactoring
- ✅ Add new APIs

**Breaking** (Need major version):
- ⚠️ Remove unconditional observability exports
- ⚠️ Change prelude exports
- ⚠️ Rename public types
- ⚠️ Change error types

**Recommendation**: Do non-breaking work first, batch breaking changes for v0.2.0

---

## 9. Success Metrics

### Code Quality Metrics

**Before**:
- 📊 2,247 lines of code
- 📁 22 files
- 📏 Largest file: 537 lines (builder.rs)
- 🧪 17 tests
- 📚 ~50% documented

**Target After**:
- 📊 ~2,500 lines (may increase due to splitting)
- 📁 ~30 files (better organized)
- 📏 Largest file: < 300 lines
- 🧪 30+ tests (80%+ coverage)
- 📚 100% public API documented

### Maintainability Metrics

**Before**:
- Cyclomatic complexity: High (builder.rs)
- Code duplication: ~30% (builder methods)
- Module cohesion: Medium

**Target After**:
- Cyclomatic complexity: Low-Medium
- Code duplication: < 10%
- Module cohesion: High

---

## 10. Risks & Mitigations

### Risk 1: Breaking Public API
**Impact**: High
**Likelihood**: Medium
**Mitigation**:
- Use feature flags for new APIs
- Deprecate old APIs before removing
- Provide migration guide
- Version as 0.2.0 if breaking

### Risk 2: Performance Regression
**Impact**: Medium
**Likelihood**: Low
**Mitigation**:
- Benchmark before refactoring
- Benchmark after each sprint
- Keep hot paths simple
- Profile under load

### Risk 3: Scope Creep
**Impact**: Medium
**Likelihood**: High
**Mitigation**:
- Stick to defined sprints
- Track hours spent
- Defer non-critical items
- Review progress weekly

---

## 11. Next Steps

1. **Review this plan** - Get feedback from team/maintainers
2. **Prioritize sprints** - Decide which to do first
3. **Create GitHub issues** - One issue per sprint
4. **Start Sprint 1** - Fix critical visibility issues
5. **Iterate** - Review and adjust as needed

---

## Appendix A: File Structure (Proposed)

```
crates/nebula-log/
├── src/
│   ├── lib.rs                 (< 150 lines, clean exports)
│   ├── builder/
│   │   ├── mod.rs
│   │   ├── format.rs          (format layer builders)
│   │   ├── reload.rs          (reload logic)
│   │   └── telemetry.rs       (telemetry setup)
│   ├── config/
│   │   ├── mod.rs
│   │   ├── base.rs            (Config, Format, Level)
│   │   ├── writer.rs          (WriterConfig, DisplayConfig)
│   │   ├── fields.rs          (Fields)
│   │   ├── telemetry.rs       (TelemetryConfig)
│   │   ├── presets.rs         (development(), production())
│   │   └── builder.rs         (Config builder)
│   ├── core/                  (unchanged)
│   ├── layer/                 (unchanged)
│   ├── metrics/               (unchanged)
│   ├── observability/         (feature-gated)
│   ├── telemetry/             (unchanged)
│   ├── format.rs              (unchanged)
│   ├── macros.rs              (unchanged)
│   ├── timing.rs              (unchanged)
│   └── writer.rs              (unchanged)
├── examples/                  (unchanged)
├── tests/
│   ├── integration/           (NEW)
│   └── features.rs            (NEW - feature flag tests)
├── TECHNICAL_DEBT.md          (this file)
├── MIGRATION_GUIDE.md         (NEW)
└── Cargo.toml
```

---

**Status**: 📋 Ready for Review
**Owner**: TBD
**Estimated Total Effort**: 36-46 hours (~1-1.5 weeks)
