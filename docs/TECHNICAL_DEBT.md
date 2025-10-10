# Technical Debt Tracker

> Last updated: 2025-10-10
> Total TODOs: 41

This document tracks all technical debt items (TODO, FIXME, HACK comments) in the Nebula project codebase. Items are categorized by priority and type to facilitate systematic resolution.

## Table of Contents

- [High Priority](#high-priority)
- [Medium Priority](#medium-priority)
- [Low Priority](#low-priority)
- [By Category](#by-category)
  - [Features](#features)
  - [Refactoring](#refactoring)
  - [Optimization](#optimization)
  - [Performance](#performance)
  - [Fixes](#fixes)
  - [Tests](#tests)
  - [Dependencies](#dependencies)

---

## High Priority

### Critical Functionality Gaps

#### 1. nebula-parameter: Display System Rewrite
**Priority:** 🔴 HIGH
**Category:** refactor
**Files:**
- `crates/nebula-parameter/src/core/mod.rs:9`
- `crates/nebula-parameter/src/core/mod.rs:12`
- `crates/nebula-parameter/src/core/display.rs:39`

**Description:**
Display system is temporarily disabled and needs complete rewrite to work with new nebula-validator API.

**Impact:** Core parameter display functionality is unavailable.

**Action Items:**
- [ ] Design new display system compatible with nebula-validator
- [ ] Implement display condition evaluation
- [ ] Replace display_stub with production implementation
- [ ] Add comprehensive tests

---

#### 2. nebula-memory: Cache Policy Integration Issues
**Priority:** 🔴 HIGH
**Category:** fix
**Files:**
- `crates/nebula-memory/src/cache/policies/ttl.rs:129`
- `crates/nebula-memory/src/cache/policies/ttl.rs:149`
- `crates/nebula-memory/src/cache/policies/ttl.rs:239`
- `crates/nebula-memory/src/cache/policies/mod.rs:3`

**Description:**
Multiple cache policy integration issues with type mismatches and disabled LFU module.

**Impact:** Reduced cache policy flexibility and potential runtime errors.

**Action Items:**
- [ ] Fix fallback policy type mismatch with CacheEntry<()>
- [ ] Fix LFU module after migration
- [ ] Re-enable fallback policy support
- [ ] Implement TTL cache clear() method

---

#### 3. nebula-resource: Pool Management Implementation
**Priority:** 🔴 HIGH
**Category:** feature
**Files:**
- `crates/nebula-resource/src/pool/mod.rs:1043`
- `crates/nebula-resource/src/pool/mod.rs:1049`

**Description:**
Missing maintenance and shutdown implementation for resource pools.

**Impact:** Resource pools cannot be properly maintained or gracefully shut down.

**Action Items:**
- [ ] Implement maintenance for all pools
- [ ] Implement shutdown for all pools
- [ ] Add lifecycle tests

---

#### 4. nebula-memory: Arena Scope and Guard
**Priority:** 🔴 HIGH
**Category:** feature
**Files:**
- `crates/nebula-memory/src/arena/scope.rs:79`
- `crates/nebula-memory/src/arena/scope.rs:132`

**Description:**
ArenaGuard functionality requires Arena::current_position() and Arena::reset_to_position() methods.

**Impact:** RAII-based arena scope management unavailable.

**Action Items:**
- [ ] Implement Arena::current_position() method
- [ ] Implement Arena::reset_to_position() method
- [ ] Enable ArenaGuard implementation
- [ ] Re-enable related tests

---

## Medium Priority

### Feature Enhancements

#### 5. nebula-log: Multi-Writer and Field Injection
**Priority:** 🟡 MEDIUM
**Category:** feature
**Files:**
- `crates/nebula-log/src/writer.rs:82`
- `crates/nebula-log/src/layer/fields.rs:55`
- `crates/nebula-log/src/layer/fields.rs:60`
- `crates/nebula-log/src/format.rs:8`
- `crates/nebula-log/src/builder.rs:89`

**Description:**
Missing features for multi-writer fanout, field injection for spans/events, and custom logfmt formatter.

**Impact:** Limited logging flexibility and custom formatting options.

**Action Items:**
- [ ] Implement proper multi-writer with fanout or tee functionality
- [ ] Implement field injection for spans
- [ ] Implement field injection for events
- [ ] Implement dedicated logfmt formatter (currently using Compact)
- [ ] Implement custom formatting when tracing-subscriber API allows

---

#### 6. nebula-error: Retry and Circuit Breaker Features
**Priority:** 🟡 MEDIUM
**Category:** feature
**Files:**
- `crates/nebula-error/src/core/retry.rs:12`
- `crates/nebula-error/src/core/retry.rs:13`
- `crates/nebula-error/src/kinds/mod.rs:81`

**Description:**
Missing advanced retry strategies and HTTP status code mapping.

**Impact:** Limited resilience patterns and web API integration.

**Action Items:**
- [ ] Add support for custom backoff strategies (jittered, decorrelated)
- [ ] Add circuit breaker pattern integration
- [ ] Add HTTP status code mapping for web API integration

---

#### 7. nebula-resilience: Metrics Collection
**Priority:** 🟡 MEDIUM
**Category:** feature
**Files:**
- `crates/nebula-resilience/src/manager.rs:371`
- `crates/nebula-resilience/src/manager.rs:408`
- `crates/nebula-resilience/src/manager.rs:409`

**Description:**
Metrics collection for circuit breaker and bulkhead patterns not implemented.

**Impact:** No observability into resilience pattern behavior.

**Action Items:**
- [ ] Implement metrics collection once patterns support it
- [ ] Replace Option<()> placeholder with actual metrics types
- [ ] Add metrics export functionality

---

#### 8. nebula-validator: LRU Cache Support
**Priority:** 🟡 MEDIUM
**Category:** dependency
**Files:**
- `crates/nebula-validator/src/combinators/cached.rs:365`
- `crates/nebula-validator/src/combinators/cached.rs:378`
- `crates/nebula-validator/src/combinators/mod.rs:117`
- `crates/nebula-validator/src/combinators/mod.rs:143`

**Description:**
LRU cache code disabled pending dependency addition.

**Impact:** LRU caching strategy unavailable for validators.

**Action Items:**
- [ ] Add lru crate as dependency
- [ ] Re-enable LruCached implementation
- [ ] Re-enable lru_cached function exports
- [ ] Add LRU cache tests

---

## Low Priority

### Optimizations and Refactoring

#### 9. nebula-error: Error Type Optimization
**Priority:** 🟢 LOW
**Category:** optimization, performance
**Files:**
- `crates/nebula-error/src/core/error.rs:31`
- `crates/nebula-error/src/core/error.rs:32`
- `crates/nebula-error/src/core/retry.rs:14`

**Description:**
Potential optimizations for error message storage and extensibility.

**Impact:** Minor performance improvement and API flexibility.

**Action Items:**
- [ ] Consider using `Cow<'static, str>` for static error messages
- [ ] Add benchmarks to measure error creation overhead
- [ ] Consider making retry strategy a trait for extensibility

---

#### 10. nebula-error: Error Hierarchy Refactoring
**Priority:** 🟢 LOW
**Category:** refactor
**Files:**
- `crates/nebula-error/src/kinds/mod.rs:80`

**Description:**
Current error hierarchy could be split into more granular types.

**Impact:** Improved type safety and error handling precision.

**Action Items:**
- [ ] Consider splitting into more granular error hierarchies
- [ ] Maintain backward compatibility
- [ ] Update error handling patterns across codebase

---

#### 11. nebula-parameter: Trait System Update
**Priority:** 🟢 LOW
**Category:** refactor
**Files:**
- `crates/nebula-parameter/src/types/mod.rs:55`
- `crates/nebula-parameter/src/types/object.rs:402`

**Description:**
Parameter types could be updated to use new trait system and display conditions.

**Impact:** Improved type system consistency.

**Action Items:**
- [ ] Update parameter types to use new trait system (when needed)
- [ ] Implement display condition evaluation based on current values

---

#### 12. nebula-derive: Smart Default Validators
**Priority:** 🟢 LOW
**Category:** feature
**Files:**
- `crates/nebula-derive/src/validator/generate.rs:149`

**Description:**
Use type_category for smarter default validator selection.

**Impact:** Better out-of-the-box validation experience.

**Action Items:**
- [ ] Use type_category for smarter default validators
- [ ] Add configuration options for default behavior

---

#### 13. nebula-value: Shallow Object Merge
**Priority:** 🟢 LOW
**Category:** feature
**Files:**
- `crates/nebula-value/src/core/ops.rs:298`

**Description:**
Add shallow merge variant for Object values.

**Impact:** Performance optimization for large object merges.

**Action Items:**
- [ ] Add shallow variant to Object merge operation
- [ ] Document differences between deep and shallow merge

---

#### 14. nebula-memory: Multi-level Cache Cleanup
**Priority:** 🟢 LOW
**Category:** feature
**Files:**
- `crates/nebula-memory/src/cache/multi_level.rs:475`

**Description:**
Proper cleanup thread implementation with shared references.

**Impact:** Better resource management for multi-level caches.

**Action Items:**
- [ ] Implement proper cleanup thread with shared references
- [ ] Add configurable cleanup intervals

---

#### 15. nebula-memory: Arena Config Mapping
**Priority:** 🟢 LOW
**Category:** refactor
**Files:**
- `crates/nebula-memory/src/arena/mod.rs:301`

**Description:**
Proper mapping needed between core::config::ArenaConfig and arena::ArenaConfig.

**Impact:** Configuration consistency and type safety.

**Action Items:**
- [ ] Implement proper mapping between config types
- [ ] Add conversion tests

---

### Tests

#### 16. nebula-validator: Optional with ?Sized Types
**Priority:** 🟢 LOW
**Category:** test, fix
**Files:**
- `crates/nebula-validator/src/combinators/optional.rs:408`

**Description:**
Test disabled - requires fixing Optional to work with ?Sized types.

**Impact:** Reduced test coverage for edge cases.

**Action Items:**
- [ ] Fix Optional to work with ?Sized types
- [ ] Re-enable test

---

## By Category

### Features (12 items)
- nebula-log: Multi-writer fanout
- nebula-log: Field injection for spans/events
- nebula-log: Custom logfmt formatter
- nebula-log: Custom formatting API
- nebula-error: Custom backoff strategies
- nebula-error: Circuit breaker integration
- nebula-error: HTTP status code mapping
- nebula-resilience: Metrics collection
- nebula-resource: Pool maintenance
- nebula-resource: Pool shutdown
- nebula-memory: ArenaGuard implementation
- nebula-parameter: Display condition evaluation

### Refactoring (5 items)
- nebula-parameter: Display system rewrite
- nebula-parameter: Trait system update
- nebula-error: Error hierarchy splitting
- nebula-memory: Arena config mapping
- nebula-derive: Smart default validators

### Optimization (2 items)
- nebula-error: Cow<'static, str> for messages
- nebula-error: Retry trait extensibility

### Performance (1 item)
- nebula-error: Error creation benchmarks

### Fixes (5 items)
- nebula-memory: Cache policy type mismatches (3 locations)
- nebula-memory: LFU module migration
- nebula-validator: Optional with ?Sized types

### Tests (1 item)
- nebula-validator: Optional ?Sized test

### Dependencies (1 item)
- nebula-validator: LRU crate addition (4 locations)

---

## Priority Guidelines

### 🔴 HIGH Priority
- Critical functionality gaps affecting core features
- Blocking issues preventing proper system operation
- Security vulnerabilities
- Data loss risks

**Target Timeline:** Current sprint

### 🟡 MEDIUM Priority
- Feature enhancements that improve user experience
- Non-critical functionality gaps
- Significant performance improvements
- Important refactoring for maintainability

**Target Timeline:** Next 2-3 sprints

### 🟢 LOW Priority
- Nice-to-have optimizations
- Minor refactoring
- Edge case fixes
- Non-critical test coverage

**Target Timeline:** Future backlog

---

## Tracking Process

### Adding New TODOs
Use standardized format in code:
```rust
// TODO(category): Description
// TODO(feature): Add support for custom serializers
// TODO(refactor): Extract common logic into helper function
// TODO(optimization): Use SIMD for batch operations
// TODO(performance): Profile and optimize hot path
// TODO(fix): Handle edge case when input is empty
// TODO(test): Add integration test for retry behavior
// TODO(docs): Document error handling patterns
// TODO(security): Validate input against SQL injection
```

### Categories
- `feature` - New functionality
- `refactor` - Code restructuring
- `optimization` - Code optimization
- `performance` - Performance improvement
- `fix` - Bug fix
- `test` - Test addition/fix
- `docs` - Documentation
- `security` - Security improvement
- `debt` - Technical debt
- `dependency` - Dependency management

### Review Cycle
- **Weekly:** Review and triage new TODOs
- **Monthly:** Update priorities based on roadmap
- **Quarterly:** Archive completed items and generate metrics

---

## Metrics

### Current Status
- **Total TODOs:** 41
- **High Priority:** 4 categories (17 items)
- **Medium Priority:** 4 categories (15 items)
- **Low Priority:** 3 categories (9 items)

### By Crate
- `nebula-error`: 7 items
- `nebula-log`: 6 items
- `nebula-memory`: 10 items
- `nebula-parameter`: 5 items
- `nebula-resilience`: 3 items
- `nebula-resource`: 2 items
- `nebula-validator`: 6 items
- `nebula-value`: 1 item
- `nebula-derive`: 1 item

### Completion Target
- **Sprint 5:** Address all HIGH priority items
- **Sprint 6-7:** Complete MEDIUM priority items
- **Sprint 8+:** LOW priority items as capacity allows

---

## Related Documents
- [Rust Refactoring Guide](rust_refactor_prompt.md)
- [Architecture Overview](../README.md)
- [Contributing Guidelines](../CONTRIBUTING.md)
