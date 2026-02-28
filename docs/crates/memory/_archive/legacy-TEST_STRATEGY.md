# Test Strategy

## Test Pyramid

### Unit Tests

- **Location**: `src/*/mod.rs`, inline `#[cfg(test)]` modules
- **Scope**: Individual functions, error constructors, trait implementations
- **Examples**:
  - `error.rs`: Error message formatting, `is_retryable()` classification
  - `allocator/stats.rs`: Statistics calculation, overflow handling
  - `cache/policies/*.rs`: Eviction policy correctness

### Integration Tests

- **Location**: `tests/*.rs`
- **Scope**: Cross-module interactions, feature combinations
- **Examples**:
  - `safety_check.rs`: Allocator safety invariants
  - `sealed_traits_demo.rs`: Sealed trait pattern verification
  - Pool + arena composition scenarios

### Contract Tests

- **Scope**: Verify behavior contracts with dependent crates
- **Examples**:
  - `MemoryMonitor` correctly interprets `nebula-system::MemoryInfo`
  - `ComputeCache` API stability for `nebula-expression`
  - Error type compatibility across crate boundaries

### End-to-End Tests

- **Scope**: Full workflow simulation (in `nebula-engine` or integration suite)
- **Examples**:
  - Workflow execution with pooled action instances
  - Memory pressure handling during high-concurrency workloads

## Critical Invariants

### Allocator Invariants

1. **Allocation alignment**: Returned pointer aligned to `layout.align()`
2. **Size guarantee**: Allocated region has at least `layout.size()` bytes
3. **Double-free safety**: Deallocation of same pointer twice is UB (documented)
4. **Reset invalidation**: All pointers invalid after `Resettable::reset()`

### Pool Invariants

1. **Return-to-pool**: `PooledValue<T>` returns item on drop
2. **Capacity bound**: Pool never holds more than configured capacity
3. **Object isolation**: Returned objects do not alias

### Cache Invariants

1. **Eviction correctness**: LRU evicts least-recently-used entry
2. **TTL expiry**: Expired entries not returned on lookup
3. **Capacity enforcement**: Cache size never exceeds configured max

### Budget Invariants

1. **Limit enforcement**: `BudgetExceeded` returned when limit reached
2. **Atomic updates**: Concurrent reservations serialized correctly
3. **Release accuracy**: Released bytes accurately tracked

## Scenario Matrix

| Scenario | Test Coverage |
|----------|---------------|
| **Happy path** | Alloc -> use -> dealloc succeeds |
| **Retry path** | Pool exhausted -> wait/cleanup -> retry succeeds |
| **Cancellation path** | Allocation interrupted mid-operation |
| **Timeout path** | Async operations respect timeout bounds |
| **Upgrade/migration path** | API changes preserve behavior |

### Detailed Scenarios

- **High allocation churn**: Rapid alloc/dealloc cycles
- **Concurrent pool access**: Multiple threads acquire/release
- **Cache eviction storm**: All entries expire simultaneously
- **Pressure escalation**: Low -> Medium -> High -> Critical transitions
- **Feature combinations**: `stats` + `monitoring` + `async` together

## Tooling

### Property Testing

- **Framework**: `proptest`
- **Location**: `tests/` or inline
- **Coverage**:
  - Arbitrary layout sizes/alignments
  - Random pool acquire/release sequences
  - Cache key/value combinations

### Fuzzing

- **Framework**: `cargo-fuzz` (planned)
- **Targets**:
  - Allocator layout parsing
  - Cache key deserialization
  - Configuration parsing

### Benchmarks

- **Framework**: `criterion`
- **Location**: `benches/`
- **Suites**:
  - `allocator_benchmarks.rs`: Bump/pool/stack allocator throughput
  - `real_world_scenarios.rs`: Workflow-like allocation patterns

### CI Quality Gates

```yaml
# Must pass before merge
- cargo fmt --all -- --check
- cargo clippy --workspace -- -D warnings
- cargo check --workspace --all-targets
- cargo test --workspace
- cargo test -p nebula-memory --all-features
- cargo doc --no-deps --workspace
- cargo audit
```

## Exit Criteria

### Coverage Goals

- **Line coverage**: > 80% for `src/` modules
- **Branch coverage**: > 70% for conditional logic
- **Unsafe coverage**: 100% of unsafe blocks have corresponding tests

### Flaky Test Budget

- **Allowed flaky rate**: < 1% of test runs
- **Action on flakiness**: Quarantine and fix within 1 sprint
- **Concurrent test isolation**: Required for thread-safety tests

### Performance Regression Thresholds

| Metric | Threshold |
|--------|-----------|
| Bump allocator throughput | < 5% regression |
| Pool acquire latency p99 | < 10% regression |
| Cache lookup latency p99 | < 10% regression |

### Test Execution Requirements

- All tests pass on:
  - `x86_64-unknown-linux-gnu`
  - `x86_64-apple-darwin`
  - `x86_64-pc-windows-msvc`
- Multi-threaded tests: `#[tokio::test(flavor = "multi_thread")]`
- Time-sensitive tests: Use `tokio::time::pause()` for determinism
