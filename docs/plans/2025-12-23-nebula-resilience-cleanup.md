# nebula-resilience Clean Code Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Refactor nebula-resilience crate to clean, idiomatic Rust 1.92+ code with comprehensive test coverage using TDD, clear documentation, and optimized performance.

**Architecture:** The crate provides resilience patterns (circuit breaker, retry, bulkhead, rate limiter, timeout, fallback, hedge) with advanced Rust type system features including const generics, phantom types, GATs, and sealed traits. We'll clean up code duplication, improve documentation clarity, add missing tests, and optimize hot paths.

**Tech Stack:** Rust 2024 Edition (MSRV 1.90+), Tokio async runtime, DashMap for concurrent state, Serde for serialization, tracing for observability.

---

## Summary of Identified Issues

### High Priority
1. **Duplicate state definitions** - `circuit_states` module duplicates `TypestateCircuitState` in traits.rs
2. **Missing error type with thiserror** - `ResilienceError` manually implements Error instead of using thiserror per CLAUDE.md
3. **Incomplete test coverage** - Rate limiter adaptive algorithm lacks integration tests
4. **Dead code warnings** - Several `#[allow(dead_code)]` markers that should be cleaned up

### Medium Priority
5. **Documentation inconsistency** - Some advanced type features lack usage examples
6. **`BulkheadPermit` active_operations tracking** - Field is never incremented/decremented
7. **Retry condition uses format!("{:?}", error)** - Should use pattern matching or trait bounds
8. **Missing const assertions** - Some const generic validations happen only at runtime

### Low Priority
9. **Unused imports and markers** - Variance markers defined but not used
10. **Inconsistent builder patterns** - Some use `#[must_use]`, others don't consistently

---

## Task 1: Fix ResilienceError to Use thiserror

**Files:**
- Modify: `crates/nebula-resilience/src/core/error.rs`
- Modify: `crates/nebula-resilience/Cargo.toml`

### Step 1: Write the failing test

Add test to verify thiserror integration works correctly.

```rust
// In crates/nebula-resilience/src/core/error.rs, add to tests module:
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let error = ResilienceError::timeout(Duration::from_secs(5));
        let display = error.to_string();
        assert!(display.contains("5s"), "Expected duration in display: {}", display);
    }

    #[test]
    fn test_error_source_chain() {
        let inner = ResilienceError::timeout(Duration::from_millis(100));
        let outer = ResilienceError::RetryLimitExceeded {
            attempts: 3,
            last_error: Some(Box::new(inner)),
        };
        
        // Verify error chain works
        assert!(outer.to_string().contains("3 attempts"));
    }

    #[test]
    fn test_error_classification() {
        assert_eq!(ResilienceError::timeout(Duration::from_secs(1)).classify(), ErrorClass::Transient);
        assert_eq!(ResilienceError::circuit_breaker_open("open").classify(), ErrorClass::ResourceExhaustion);
        assert_eq!(ResilienceError::InvalidConfig { message: "bad".into() }.classify(), ErrorClass::Configuration);
    }
}
```

### Step 2: Run test to verify it fails

Run: `cargo test -p nebula-resilience error::tests --no-fail-fast`
Expected: Tests should pass (existing implementation), but we'll refactor to thiserror

### Step 3: Add thiserror dependency and refactor error.rs

```toml
# In Cargo.toml, add:
thiserror = { workspace = true }
```

```rust
// Replace error.rs with thiserror-based implementation:
//! Error types for resilience operations

use std::time::Duration;
use thiserror::Error;

/// Core resilience errors
#[derive(Debug, Error, Clone)]
#[must_use = "ResilienceError should be returned or handled"]
pub enum ResilienceError {
    /// Operation timed out
    #[error("Operation timed out after {duration:?}{}", context.as_ref().map(|c| format!(" - {}", c)).unwrap_or_default())]
    Timeout {
        duration: Duration,
        context: Option<String>,
    },

    /// Circuit breaker is open
    #[error("Circuit breaker is {state}{}", retry_after.map(|d| format!(" (retry after {:?})", d)).unwrap_or_default())]
    CircuitBreakerOpen {
        state: String,
        retry_after: Option<Duration>,
    },

    /// Bulkhead is full
    #[error("Bulkhead full: max={max_concurrency}, queued={queued}")]
    BulkheadFull {
        max_concurrency: usize,
        queued: usize,
    },

    /// Rate limit exceeded
    #[error("Rate limit exceeded: limit={limit}/s, current={current}/s{}", retry_after.map(|d| format!(" (retry after {:?})", d)).unwrap_or_default())]
    RateLimitExceeded {
        retry_after: Option<Duration>,
        limit: f64,
        current: f64,
    },

    /// Retry limit exceeded
    #[error("Retry limit exceeded after {attempts} attempts{}", last_error.as_ref().map(|e| format!(" - last error: {}", e)).unwrap_or_default())]
    RetryLimitExceeded {
        attempts: usize,
        last_error: Option<Box<ResilienceError>>,
    },

    /// Fallback operation failed
    #[error("Fallback failed: {reason}")]
    FallbackFailed {
        reason: String,
        original_error: Option<Box<ResilienceError>>,
    },

    /// Operation was cancelled
    #[error("Operation cancelled{}", reason.as_ref().map(|r| format!(": {}", r)).unwrap_or_default())]
    Cancelled {
        reason: Option<String>,
    },

    /// Invalid configuration
    #[error("Invalid configuration: {message}")]
    InvalidConfig {
        message: String,
    },

    /// Custom error for extensions
    #[error("{message}")]
    Custom {
        message: String,
        retryable: bool,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}
```

### Step 4: Run tests to verify they pass

Run: `cargo test -p nebula-resilience`
Expected: PASS - all existing tests should pass

### Step 5: Commit

```bash
git add crates/nebula-resilience/Cargo.toml crates/nebula-resilience/src/core/error.rs
git commit -m "refactor(nebula-resilience): use thiserror for error handling"
```

---

## Task 2: Remove Duplicate Circuit State Definitions

**Files:**
- Modify: `crates/nebula-resilience/src/patterns/circuit_breaker.rs`
- Modify: `crates/nebula-resilience/src/core/traits.rs`

### Step 1: Write the failing test

```rust
// Add to circuit_breaker.rs tests:
#[test]
fn test_state_types_are_consistent() {
    use crate::core::traits::circuit_states::{Closed, Open, HalfOpen};
    
    // Verify the phantom type states exist and work
    let _closed: std::marker::PhantomData<Closed> = std::marker::PhantomData;
    let _open: std::marker::PhantomData<Open> = std::marker::PhantomData;
    let _half_open: std::marker::PhantomData<HalfOpen> = std::marker::PhantomData;
}
```

### Step 2: Run test to verify baseline

Run: `cargo test -p nebula-resilience test_state_types_are_consistent`
Expected: PASS

### Step 3: Remove duplicate states module from circuit_breaker.rs

Remove the `pub mod states` block from `circuit_breaker.rs` and import from `core::traits::circuit_states` instead. Update all references.

```rust
// In circuit_breaker.rs, replace:
// pub mod states { ... }

// With:
pub use crate::core::traits::circuit_states::{
    Closed, HalfOpen, Open, StateMetadata, StateTransition,
    TypestateCircuitState as CircuitState,
};
```

### Step 4: Run tests to verify refactoring works

Run: `cargo test -p nebula-resilience`
Expected: PASS

### Step 5: Commit

```bash
git add crates/nebula-resilience/src/patterns/circuit_breaker.rs crates/nebula-resilience/src/core/traits.rs
git commit -m "refactor(nebula-resilience): consolidate circuit state types"
```

---

## Task 3: Fix BulkheadPermit Active Operations Tracking

**Files:**
- Modify: `crates/nebula-resilience/src/patterns/bulkhead.rs`

### Step 1: Write the failing test

```rust
#[tokio::test]
async fn test_bulkhead_active_operations_tracking() {
    let bulkhead = Bulkhead::new(3);
    
    assert_eq!(bulkhead.active_operations().await, 0);
    
    let permit1 = bulkhead.acquire().await.unwrap();
    assert_eq!(bulkhead.active_operations().await, 1);
    
    let permit2 = bulkhead.acquire().await.unwrap();
    assert_eq!(bulkhead.active_operations().await, 2);
    
    drop(permit1);
    // Give time for async drop
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    assert_eq!(bulkhead.active_operations().await, 1);
    
    drop(permit2);
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    assert_eq!(bulkhead.active_operations().await, 0);
}
```

### Step 2: Run test to verify it fails

Run: `cargo test -p nebula-resilience test_bulkhead_active_operations_tracking`
Expected: FAIL - active_operations is never incremented

### Step 3: Implement proper tracking

```rust
// In bulkhead.rs, update acquire method:
pub async fn acquire(&self) -> Result<BulkheadPermit, ResilienceError> {
    let permit = Arc::clone(&self.semaphore)
        .acquire_owned()
        .await
        .map_err(|_| ResilienceError::bulkhead_full(self.config.max_concurrency))?;

    // Increment active operations counter
    {
        let mut active = self.active_operations.write().await;
        *active += 1;
    }

    Ok(BulkheadPermit {
        permit,
        active_operations: Arc::clone(&self.active_operations),
    })
}

// Update BulkheadPermit to decrement on drop:
impl Drop for BulkheadPermit {
    fn drop(&mut self) {
        // Use tokio::spawn to handle async decrement
        let active_ops = Arc::clone(&self.active_operations);
        tokio::spawn(async move {
            let mut active = active_ops.write().await;
            *active = active.saturating_sub(1);
        });
    }
}
```

### Step 4: Run test to verify it passes

Run: `cargo test -p nebula-resilience test_bulkhead_active_operations_tracking`
Expected: PASS

### Step 5: Commit

```bash
git add crates/nebula-resilience/src/patterns/bulkhead.rs
git commit -m "fix(nebula-resilience): implement bulkhead active operations tracking"
```

---

## Task 4: Improve Retry Condition Error Matching

**Files:**
- Modify: `crates/nebula-resilience/src/patterns/retry.rs`

### Step 1: Write the failing test

```rust
#[test]
fn test_retry_condition_with_typed_errors() {
    let condition = ConservativeCondition::<3>::new();
    
    // Should retry on Timeout
    let timeout_err = ResilienceError::timeout(Duration::from_secs(1));
    assert!(condition.should_retry(&timeout_err, 0, Duration::ZERO));
    
    // Should NOT retry on InvalidConfig
    let config_err = ResilienceError::InvalidConfig { message: "bad".into() };
    assert!(!condition.should_retry(&config_err, 0, Duration::ZERO));
    
    // Terminal errors should return true for is_terminal
    assert!(condition.is_terminal(&config_err));
}
```

### Step 2: Run test to verify current behavior

Run: `cargo test -p nebula-resilience test_retry_condition_with_typed_errors`
Expected: May pass or fail depending on current string matching

### Step 3: Implement type-safe error matching for ResilienceError

```rust
// Add specialized implementation for ResilienceError:
impl<const MAX_ATTEMPTS: usize> RetryCondition<ResilienceError> for ConservativeCondition<MAX_ATTEMPTS> {
    fn should_retry(&self, error: &ResilienceError, attempt: usize, _elapsed: Duration) -> bool {
        if attempt >= MAX_ATTEMPTS {
            return false;
        }

        matches!(error,
            ResilienceError::Timeout { .. } |
            ResilienceError::RateLimitExceeded { .. } |
            ResilienceError::Custom { retryable: true, .. }
        )
    }

    fn is_terminal(&self, error: &ResilienceError) -> bool {
        matches!(error,
            ResilienceError::InvalidConfig { .. } |
            ResilienceError::Cancelled { .. } |
            ResilienceError::Custom { retryable: false, .. }
        )
    }

    fn custom_delay(&self, error: &ResilienceError, _attempt: usize) -> Option<Duration> {
        error.retry_after()
    }

    fn condition_name(&self) -> &'static str {
        "Conservative"
    }
}
```

### Step 4: Run tests to verify improvements

Run: `cargo test -p nebula-resilience retry`
Expected: PASS

### Step 5: Commit

```bash
git add crates/nebula-resilience/src/patterns/retry.rs
git commit -m "refactor(nebula-resilience): use pattern matching for retry conditions"
```

---

## Task 5: Add Comprehensive Rate Limiter Tests

**Files:**
- Create: `crates/nebula-resilience/tests/integration_rate_limiter.rs`

### Step 1: Write the failing test

```rust
//! Integration tests for rate limiter patterns

use nebula_resilience::patterns::rate_limiter::*;
use std::time::Duration;
use tokio::time::Instant;

#[tokio::test]
async fn test_token_bucket_rate_limiting() {
    let limiter = TokenBucket::new(10.0, 10); // 10 req/s, burst of 10
    
    // Should allow burst
    for _ in 0..10 {
        assert!(limiter.try_acquire().await.is_ok());
    }
    
    // Should be rate limited now
    assert!(limiter.try_acquire().await.is_err());
    
    // Wait for refill
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(limiter.try_acquire().await.is_ok());
}

#[tokio::test]
async fn test_sliding_window_accuracy() {
    let limiter = SlidingWindow::new(5, Duration::from_millis(100));
    
    // Fill the window
    for _ in 0..5 {
        assert!(limiter.try_acquire().await.is_ok());
    }
    
    // Should be limited
    assert!(limiter.try_acquire().await.is_err());
    
    // Wait for window to slide
    tokio::time::sleep(Duration::from_millis(110)).await;
    
    // Should allow more requests
    assert!(limiter.try_acquire().await.is_ok());
}

#[tokio::test]
async fn test_adaptive_rate_limiter_adjusts() {
    let limiter = AdaptiveRateLimiter::new(100.0, 10.0, 1000.0);
    
    // Record successes - rate should increase
    for _ in 0..20 {
        limiter.record_success().await;
    }
    
    let rate_after_success = limiter.current_rate().await;
    
    // Record failures - rate should decrease
    for _ in 0..10 {
        limiter.record_failure().await;
    }
    
    let rate_after_failure = limiter.current_rate().await;
    
    assert!(rate_after_failure < rate_after_success, 
        "Rate should decrease after failures: {} vs {}", 
        rate_after_failure, rate_after_success);
}

#[tokio::test]
async fn test_any_rate_limiter_enum() {
    let token_bucket = AnyRateLimiter::TokenBucket(TokenBucket::new(100.0, 10));
    let leaky = AnyRateLimiter::LeakyBucket(LeakyBucket::new(100.0, 10));
    
    // Both should work through the enum
    assert!(token_bucket.try_acquire().await.is_ok());
    assert!(leaky.try_acquire().await.is_ok());
}

#[tokio::test]
async fn test_concurrent_rate_limiting() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    
    let limiter = Arc::new(TokenBucket::new(50.0, 10)); // 50 req/s
    let success_count = Arc::new(AtomicUsize::new(0));
    let reject_count = Arc::new(AtomicUsize::new(0));
    
    let mut handles = vec![];
    
    // Spawn 100 concurrent requests
    for _ in 0..100 {
        let limiter = Arc::clone(&limiter);
        let success = Arc::clone(&success_count);
        let reject = Arc::clone(&reject_count);
        
        handles.push(tokio::spawn(async move {
            if limiter.try_acquire().await.is_ok() {
                success.fetch_add(1, Ordering::Relaxed);
            } else {
                reject.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    
    futures::future::join_all(handles).await;
    
    let successes = success_count.load(Ordering::Relaxed);
    let rejects = reject_count.load(Ordering::Relaxed);
    
    // Should have limited some requests
    assert!(successes <= 15, "Expected burst limit, got {} successes", successes);
    assert!(rejects >= 85, "Expected rejections, got {} rejects", rejects);
    assert_eq!(successes + rejects, 100);
}
```

### Step 2: Run test to verify it fails

Run: `cargo test -p nebula-resilience --test integration_rate_limiter`
Expected: FAIL - file doesn't exist yet

### Step 3: Create the test file

Create `crates/nebula-resilience/tests/integration_rate_limiter.rs` with the content above.

### Step 4: Run tests

Run: `cargo test -p nebula-resilience --test integration_rate_limiter`
Expected: PASS (or identify rate limiter bugs to fix)

### Step 5: Commit

```bash
git add crates/nebula-resilience/tests/integration_rate_limiter.rs
git commit -m "test(nebula-resilience): add comprehensive rate limiter integration tests"
```

---

## Task 6: Clean Up Dead Code and Warnings

**Files:**
- Modify: `crates/nebula-resilience/src/patterns/bulkhead.rs`
- Modify: `crates/nebula-resilience/src/patterns/circuit_breaker.rs`
- Modify: `crates/nebula-resilience/src/core/advanced.rs`

### Step 1: Run clippy to identify issues

Run: `cargo clippy -p nebula-resilience -- -D warnings 2>&1 | head -100`
Expected: List of warnings to fix

### Step 2: Fix identified issues

Remove or use dead code, fix clippy warnings. Common issues:
- Remove unused `half_open_operations` field or use it
- Remove unused variance markers or document them
- Fix any missing documentation

### Step 3: Run clippy again

Run: `cargo clippy -p nebula-resilience -- -D warnings`
Expected: No warnings

### Step 4: Run full test suite

Run: `cargo test -p nebula-resilience`
Expected: PASS

### Step 5: Commit

```bash
git add crates/nebula-resilience/
git commit -m "chore(nebula-resilience): clean up dead code and clippy warnings"
```

---

## Task 7: Add Documentation Examples for Advanced Features

**Files:**
- Modify: `crates/nebula-resilience/src/lib.rs`
- Modify: `crates/nebula-resilience/src/core/advanced.rs`

### Step 1: Write doc tests

```rust
// In lib.rs, add comprehensive examples:

/// # Typestate Pattern Example
/// 
/// ```rust
/// use nebula_resilience::builder::*;
/// use nebula_resilience::prelude::*;
/// 
/// // Type-safe builder that tracks configuration state
/// let resilience = ResilienceBuilder::new()
///     .with_circuit_breaker::<5, 30_000>(|config| {
///         config.with_half_open_limit(3)
///     });
/// ```
/// 
/// # Const Generic Validation
/// 
/// ```rust
/// use nebula_resilience::prelude::*;
/// 
/// // Compile-time validated circuit breaker
/// // FAILURE_THRESHOLD=5, RESET_TIMEOUT_MS=30000
/// let breaker = CircuitBreaker::<5, 30_000>::default();
/// ```
```

### Step 2: Run doc tests

Run: `cargo test -p nebula-resilience --doc`
Expected: PASS

### Step 3: Add more examples for advanced.rs

Document variance markers, GADT patterns, and typestate pattern with runnable examples.

### Step 4: Verify docs build

Run: `cargo doc -p nebula-resilience --no-deps`
Expected: Docs build without warnings

### Step 5: Commit

```bash
git add crates/nebula-resilience/src/lib.rs crates/nebula-resilience/src/core/advanced.rs
git commit -m "docs(nebula-resilience): add examples for advanced type features"
```

---

## Task 8: Add Const Assertion Validation

**Files:**
- Modify: `crates/nebula-resilience/src/patterns/circuit_breaker.rs`
- Modify: `crates/nebula-resilience/src/patterns/retry.rs`

### Step 1: Write compile-fail test concept

```rust
// Add const assertions in circuit_breaker.rs:
impl<const FAILURE_THRESHOLD: usize, const RESET_TIMEOUT_MS: u64>
    CircuitBreakerConfig<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>
{
    /// Compile-time validation
    const VALID: () = {
        assert!(FAILURE_THRESHOLD > 0, "FAILURE_THRESHOLD must be positive");
        assert!(RESET_TIMEOUT_MS > 0, "RESET_TIMEOUT_MS must be positive");
        assert!(RESET_TIMEOUT_MS <= 300_000, "RESET_TIMEOUT_MS must be <= 5 minutes");
    };

    pub fn new() -> Self {
        // Force const evaluation
        let _ = Self::VALID;
        Self::default()
    }
}
```

### Step 2: Verify compile-time validation works

Try to create invalid config:
```rust
// This should fail to compile:
// let config = CircuitBreakerConfig::<0, 30_000>::new();
```

### Step 3: Add to retry.rs

Similar const assertions for retry configuration.

### Step 4: Run tests

Run: `cargo test -p nebula-resilience`
Expected: PASS

### Step 5: Commit

```bash
git add crates/nebula-resilience/src/patterns/circuit_breaker.rs crates/nebula-resilience/src/patterns/retry.rs
git commit -m "feat(nebula-resilience): add compile-time const validation"
```

---

## Task 9: Optimize Circuit Breaker Hot Path

**Files:**
- Modify: `crates/nebula-resilience/src/patterns/circuit_breaker.rs`

### Step 1: Add benchmark

```rust
// In benches/circuit_breaker.rs, add:
fn bench_circuit_breaker_closed_fast_path(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let breaker = CircuitBreaker::<5, 30_000>::default();
    
    c.bench_function("circuit_breaker_closed_no_lock", |b| {
        b.to_async(&rt).iter(|| async {
            breaker.execute(|| async { Ok::<_, ResilienceError>(42) }).await
        })
    });
}
```

### Step 2: Run benchmark baseline

Run: `cargo bench -p nebula-resilience -- circuit_breaker`
Expected: Get baseline performance

### Step 3: Optimize with atomic state check

```rust
// Add atomic state for fast-path check in closed state:
struct CircuitBreakerInner<...> {
    // Add atomic for fast closed-state check
    is_definitely_closed: AtomicBool,
    // ... rest of fields
}

// In execute:
pub async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T> {
    // Fast path: if definitely closed, skip lock acquisition
    if self.inner.is_definitely_closed.load(Ordering::Relaxed) {
        // Execute directly, update state atomically on failure
    }
    // Slow path: acquire lock for state transitions
}
```

### Step 4: Run benchmark again

Run: `cargo bench -p nebula-resilience -- circuit_breaker`
Expected: Improvement in closed-state performance

### Step 5: Commit

```bash
git add crates/nebula-resilience/src/patterns/circuit_breaker.rs crates/nebula-resilience/benches/circuit_breaker.rs
git commit -m "perf(nebula-resilience): optimize circuit breaker closed-state fast path"
```

---

## Task 10: Final Cleanup and Verification

**Files:**
- All modified files

### Step 1: Run full test suite

Run: `cargo test --workspace`
Expected: PASS

### Step 2: Run clippy

Run: `cargo clippy --workspace -- -D warnings`
Expected: No warnings

### Step 3: Run fmt check

Run: `cargo fmt --all -- --check`
Expected: No formatting issues

### Step 4: Build docs

Run: `cargo doc --no-deps --workspace`
Expected: Docs build successfully

### Step 5: Run benchmarks

Run: `cargo bench -p nebula-resilience`
Expected: Benchmarks complete successfully

### Step 6: Final commit

```bash
git add .
git commit -m "chore(nebula-resilience): complete clean code refactoring"
```

---

## Post-Implementation Checklist

- [ ] All tests pass (`cargo test --workspace`)
- [ ] No clippy warnings (`cargo clippy --workspace -- -D warnings`)
- [ ] Code formatted (`cargo fmt --all`)
- [ ] Documentation builds (`cargo doc --no-deps --workspace`)
- [ ] Benchmarks run (`cargo bench -p nebula-resilience`)
- [ ] No dead code warnings
- [ ] thiserror used for error types
- [ ] Const assertions validate at compile time
- [ ] Rate limiter has integration tests
- [ ] Circuit breaker hot path optimized
- [ ] Documentation includes advanced feature examples
