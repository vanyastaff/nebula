---
title: "[MEDIUM] Improve test coverage across all crates"
labels: testing, medium-priority, quality
assignees:
milestone: Sprint 7
---

## Problem

Test coverage analysis reveals gaps in the testing strategy. While some crates have excellent coverage, others lack comprehensive tests, particularly for error paths and edge cases.

## Current Test Distribution

### Test Files by Type

**Unit Tests:** ~120 test files
**Integration Tests:** ~30 test files
**Examples:** ~45 example files
**Benchmarks:** ~15 benchmark files

### Coverage Gaps Identified

#### 1. Excessive `unwrap()` in Tests
- **1681 occurrences** of `.unwrap()` or `.expect()` across 207 files
- Most in tests (acceptable) but some in production code
- Risk: Panics instead of proper error handling in edge cases

#### 2. Dead Code in Test Infrastructure
- Multiple `#[allow(dead_code)]` in test utils
- TestInstance with `todo!()` implementations (Issue #7)
- Incomplete test fixtures

#### 3. Missing Integration Tests
- Limited cross-crate integration testing
- Few end-to-end workflow tests
- Minimal async/concurrent tests

#### 4. Insufficient Error Path Testing
- Focus on happy paths
- Error conditions often untested
- Edge cases not covered

## Impact

🟡 **MEDIUM Priority** - Quality and reliability concern

**Consequences:**
- Bugs not caught until production
- Regressions not detected
- Low confidence in refactoring
- Difficult to maintain
- Poor user experience with errors

## Action Items

### Phase 1: Coverage Baseline (Sprint 7)
- [ ] Set up code coverage tooling
  - [ ] Install `cargo-tarpaulin` or `cargo-llvm-cov`
  - [ ] Generate baseline coverage report
  - [ ] Set up coverage CI integration
- [ ] Analyze coverage per crate
  - [ ] Identify crates below 70% coverage
  - [ ] Find critical paths with no coverage
  - [ ] Document coverage gaps

### Phase 2: Critical Path Testing
- [ ] **nebula-memory** (allocators)
  - [ ] Test all error conditions (OOM, invalid free, etc.)
  - [ ] Add stress tests for concurrent access
  - [ ] Expand Miri coverage (see Issue #8)
  - [ ] Add leak detection tests
- [ ] **nebula-resource** (lifecycle)
  - [ ] Fix TestInstance (see Issue #7)
  - [ ] Test all lifecycle transitions
  - [ ] Test pool exhaustion scenarios
  - [ ] Test shutdown and cleanup
- [ ] **nebula-resilience** (patterns)
  - [ ] Test circuit breaker state transitions
  - [ ] Test timeout edge cases
  - [ ] Test bulkhead saturation
  - [ ] Test rate limiter accuracy
- [ ] **nebula-validator** (validation)
  - [ ] Test all validator combinators
  - [ ] Test error message generation
  - [ ] Test recursive validation
  - [ ] Test performance limits

### Phase 3: Error Path Testing
- [ ] Add negative tests for all public APIs
  - [ ] Invalid input tests
  - [ ] Boundary condition tests
  - [ ] Concurrent access tests
  - [ ] Resource exhaustion tests
- [ ] Test error propagation
  - [ ] Error conversion tests
  - [ ] Error context preservation
  - [ ] Error recovery tests
- [ ] Test panic safety
  - [ ] Drop during panic tests
  - [ ] Resource cleanup tests
  - [ ] Poisoned lock handling

### Phase 4: Integration Testing
- [ ] Add cross-crate integration tests
  - [ ] nebula-resource + nebula-credential
  - [ ] nebula-config + nebula-validator
  - [ ] nebula-expression + nebula-value
  - [ ] nebula-resilience + nebula-resource
- [ ] Add end-to-end workflow tests
  - [ ] Full parameter validation workflows
  - [ ] Complete resource lifecycle
  - [ ] Multi-level caching scenarios
  - [ ] Resilience pattern composition
- [ ] Add async/concurrent tests
  - [ ] Tokio integration tests
  - [ ] Concurrent pool access
  - [ ] Async validator chains
  - [ ] Parallel expression evaluation

### Phase 5: Test Infrastructure
- [ ] Improve test utilities
  - [ ] Create test fixture builders
  - [ ] Add assertion helpers
  - [ ] Provide mock implementations
  - [ ] Add property-based testing (proptest)
- [ ] Standardize test organization
  - [ ] Consistent test module naming
  - [ ] Separate unit/integration tests
  - [ ] Group related tests
- [ ] Add test documentation
  - [ ] Document testing patterns
  - [ ] Provide test templates
  - [ ] Explain coverage goals

### Phase 6: CI Enforcement
- [ ] Add coverage gates
  - [ ] Minimum 70% coverage per crate
  - [ ] Minimum 80% for critical crates
  - [ ] Fail CI if coverage drops
- [ ] Add test quality checks
  - [ ] Flag excessive `unwrap()` in production code
  - [ ] Require tests for new features
  - [ ] Enforce test naming conventions
- [ ] Performance regression tests
  - [ ] Benchmark critical paths
  - [ ] Track performance over time
  - [ ] Alert on regressions

## Coverage Tools

### cargo-tarpaulin
```bash
# Install
cargo install cargo-tarpaulin

# Generate coverage
cargo tarpaulin --workspace --out Html --output-dir coverage/

# CI integration
cargo tarpaulin --workspace --out Xml --coveralls $COVERALLS_TOKEN
```

### cargo-llvm-cov
```bash
# Install
cargo install cargo-llvm-cov

# Generate coverage
cargo llvm-cov --html --workspace

# Show summary
cargo llvm-cov --summary-only
```

### CI Configuration
```yaml
# .github/workflows/coverage.yml
name: Coverage
on: [push, pull_request]

jobs:
  coverage:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo install cargo-llvm-cov
      - run: cargo llvm-cov --workspace --lcov --output-path lcov.info
      - uses: codecov/codecov-action@v3
        with:
          files: lcov.info
          fail_ci_if_error: true
```

## Test Organization

### Current Structure
```
crates/
  nebula-memory/
    tests/           # Integration tests
    src/
      lib.rs         # Unit tests inline
```

### Proposed Structure
```
crates/
  nebula-memory/
    tests/
      integration/   # Cross-module tests
      unit/          # Extracted unit tests
      fixtures/      # Test data and helpers
    src/
      lib.rs         # Minimal inline tests
```

## Priority Targets

### HIGH Priority (Sprint 7)
| Crate | Current Coverage | Target | Focus Areas |
|-------|-----------------|---------|-------------|
| nebula-memory | ~60% | 80% | Allocators, error paths |
| nebula-resource | ~55% | 75% | Lifecycle, pools |
| nebula-validator | ~70% | 85% | Edge cases, combinators |
| nebula-resilience | ~50% | 75% | Pattern state machines |

### MEDIUM Priority (Sprint 8)
| Crate | Current Coverage | Target | Focus Areas |
|-------|-----------------|---------|-------------|
| nebula-expression | ~65% | 75% | Parser, evaluator |
| nebula-config | ~60% | 75% | Loaders, validation |
| nebula-parameter | ~55% | 70% | Types, validation |
| nebula-credential | ~70% | 80% | Flows, security |

### LOW Priority (Sprint 9)
| Crate | Current Coverage | Target | Focus Areas |
|-------|-----------------|---------|-------------|
| nebula-error | ~75% | 85% | Context, retry |
| nebula-value | ~75% | 85% | Conversions, ops |
| nebula-log | ~65% | 75% | Formatting, layers |
| nebula-system | ~60% | 70% | Platform-specific |

## Property-Based Testing

### Add proptest for Critical Paths
```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_allocator_any_size(size in 1usize..1024) {
        let allocator = BumpAllocator::new(4096);
        let result = allocator.alloc_bytes(size);
        prop_assert!(result.is_ok());
    }

    #[test]
    fn test_validator_any_string(s in ".*") {
        let validator = StringValidator::new();
        // Should not panic
        let _ = validator.validate(&s);
    }
}
```

## Metrics

### Current State (Estimated)
- **Overall coverage:** ~65%
- **Critical crate coverage:** ~60%
- **Error path coverage:** ~30%
- **Integration test count:** ~30

### Target State (After Sprint 8)
- **Overall coverage:** >75%
- **Critical crate coverage:** >80%
- **Error path coverage:** >60%
- **Integration test count:** >100

## Examples to Add

### Error Path Testing
```rust
#[test]
fn test_pool_exhaustion() {
    let pool = Pool::new(PoolConfig::default().with_max_size(2));

    // Allocate maximum
    let obj1 = pool.get().unwrap();
    let obj2 = pool.get().unwrap();

    // Should fail gracefully
    let result = pool.try_get(Duration::from_millis(10));
    assert!(matches!(result, Err(PoolError::Timeout)));
}
```

### Concurrent Testing
```rust
#[tokio::test]
async fn test_concurrent_pool_access() {
    let pool = Arc::new(ThreadSafePool::new(Config::default()));

    let handles: Vec<_> = (0..100)
        .map(|_| {
            let p = pool.clone();
            tokio::spawn(async move {
                let obj = p.get().await.unwrap();
                // Use object
                drop(obj);
            })
        })
        .collect();

    for handle in handles {
        handle.await.unwrap();
    }

    // All objects should be returned
    assert_eq!(pool.available(), pool.capacity());
}
```

### Property-Based Testing
```rust
proptest! {
    #[test]
    fn test_arena_reset_idempotent(
        alloc_sizes in prop::collection::vec(1usize..1024, 0..100)
    ) {
        let arena = Arena::new(8192);

        // Allocate various sizes
        for size in &alloc_sizes {
            let _ = arena.alloc_bytes(*size);
        }

        // Reset multiple times - should be idempotent
        arena.reset();
        let pos1 = arena.used();
        arena.reset();
        let pos2 = arena.used();

        prop_assert_eq!(pos1, pos2);
        prop_assert_eq!(pos1, 0);
    }
}
```

## References

- [Rust Book: Testing](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [proptest: Property-based testing](https://github.com/proptest-rs/proptest)
- [cargo-tarpaulin](https://github.com/xd009642/tarpaulin)
- [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov)
- Technical Debt Tracker: [docs/TECHNICAL_DEBT.md](../TECHNICAL_DEBT.md)
- Related: Issue #7 (TestInstance), Issue #8 (Unsafe/Miri)

## Acceptance Criteria

- [ ] Coverage tooling set up and integrated in CI
- [ ] All critical crates achieve >80% coverage
- [ ] All error paths have dedicated tests
- [ ] 100+ integration tests added
- [ ] Property-based tests for key algorithms
- [ ] Test infrastructure documentation complete
- [ ] CI fails on coverage drops
- [ ] Benchmark suite for performance regression

## Timeline

- **Sprint 7**: Coverage baseline + critical path testing
- **Sprint 8**: Error paths + integration tests
- **Sprint 9**: Test infrastructure + CI enforcement
