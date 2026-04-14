# nebula-resilience ASM-Guided Optimizations

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Optimize hot-path assembly in nebula-resilience based on cargo-asm audit findings (F1-F10).

**Architecture:** Six targeted changes to circuit breaker, retry, and rate limiter hot paths. Power-of-two ring buffer eliminates div+bounds checks; chunked sum enables SIMD auto-vectorization; AtomicU32 removes lock for read-only state query; powi replaces powf in backoff.

**Tech Stack:** Rust 1.93, parking_lot, tokio, fastrand, criterion benchmarks

---

## Task 1: Power-of-Two Ring Buffer (F3+F4)

Eliminates `div` instruction (~35 cycles) and bounds-check panics in `OutcomeWindow::record`.

**Files:**
- Modify: `crates/resilience/src/circuit_breaker.rs:196-263` (OutcomeWindow)
- Test: existing tests in same file + `benches/sliding_window_cb.rs`

**Step 1: Modify `OutcomeWindow` to use power-of-two capacity with bitmask**

Replace the struct and all methods:

```rust
#[doc(hidden)]
#[derive(Debug)]
pub struct OutcomeWindow {
    /// 1 = failure, 0 = success -- one byte per slot, contiguous for SIMD.
    failure_ring: Box<[u8]>,
    /// 1 = slow call, 0 = normal -- one byte per slot, contiguous for SIMD.
    slow_ring: Box<[u8]>,
    /// Bitmask for wrapping: always `capacity - 1` where capacity is a power of two.
    mask: usize,
    head: usize,
    len: usize,
}

impl OutcomeWindow {
    #[must_use]
    pub fn new(requested: usize) -> Self {
        let cap = requested.next_power_of_two().max(1);
        Self {
            failure_ring: vec![0u8; cap].into_boxed_slice(),
            slow_ring: vec![0u8; cap].into_boxed_slice(),
            mask: cap - 1,
            head: 0,
            len: 0,
        }
    }

    pub fn record(&mut self, is_failure: bool, is_slow: bool) {
        let cap = self.mask + 1;
        // SAFETY invariant: head is always < cap because mask = cap-1 and cap is power-of-two.
        // Bounds checks are redundant but kept for safe Rust; LLVM eliminates them
        // because (head & mask) < cap is provable.
        self.failure_ring[self.head] = u8::from(is_failure);
        self.slow_ring[self.head] = u8::from(is_slow);
        self.head = (self.head + 1) & self.mask;
        if self.len < cap {
            self.len += 1;
        }
    }

    // Reason: usize to u32 cast is safe for practical window sizes (< 2^32).
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub const fn total(&self) -> u32 {
        self.len as u32
    }

    /// Returns the requested (logical) capacity, not the internal power-of-two capacity.
    /// Used only by tests/benchmarks that need to know the usable window size.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.mask + 1
    }

    #[must_use]
    pub fn failure_count(&self) -> u32 {
        byte_sum(self.active_slice(&self.failure_ring))
    }

    #[must_use]
    pub fn slow_count(&self) -> u32 {
        byte_sum(self.active_slice(&self.slow_ring))
    }

    fn active_slice<'a>(&self, ring: &'a [u8]) -> &'a [u8] {
        let cap = self.mask + 1;
        if self.len < cap {
            &ring[..self.len]
        } else {
            ring
        }
    }

    fn reset(&mut self) {
        self.head = 0;
        self.len = 0;
        self.failure_ring.fill(0);
        self.slow_ring.fill(0);
    }
}
```

**Step 2: Add `byte_sum` helper that LLVM can auto-vectorize**

Add this free function above `OutcomeWindow` impl:

```rust
/// Sum a slice of 0/1 bytes into a u32.
///
/// Chunked iteration helps LLVM auto-vectorize via `psadbw`/`vpsadbw`.
/// Each chunk accumulates in u8 (max 255 iterations before overflow),
/// then widens to u32. For slices <= 255 this is a single pass.
#[inline]
fn byte_sum(slice: &[u8]) -> u32 {
    slice
        .chunks(255)
        .map(|chunk| {
            chunk.iter().copied().sum::<u8>() as u32
        })
        .sum()
}
```

**Step 3: Run tests and benchmarks**

```bash
rtk cargo nextest run -p nebula-resilience
rtk cargo bench --no-run -p nebula-resilience --bench sliding_window_cb
```

Expected: all tests pass, benchmarks compile. `failure_count`/`slow_count` should show improvement at window sizes >= 32.

**Step 4: Verify ASM improvement**

```bash
cargo asm -p nebula-resilience --lib "nebula_resilience::circuit_breaker::OutcomeWindow::failure_count"
```

Expected: should see `psadbw` or `vpsadbw` SIMD instructions instead of scalar byte-at-a-time loop. The `div` instruction should be gone from `record`, replaced by `and`.

---

## Task 2: AtomicU32 for Circuit State Read (F10)

Removes mutex acquire/release (~40 cycles) from read-only `circuit_state()` query.

**Files:**
- Modify: `crates/resilience/src/circuit_breaker.rs` (struct fields, `circuit_state`, state transitions)

**Step 1: Add an atomic state mirror alongside the Mutex**

Add a field to `CircuitBreaker`:

```rust
use std::sync::atomic::{AtomicU32, Ordering};

pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    /// Atomic mirror of the current state for lock-free observability reads.
    /// Kept in sync by all state transitions inside the mutex.
    atomic_state: AtomicU32,
    state: Mutex<InnerState>,
    clock: Arc<dyn Clock>,
    sink: Arc<dyn MetricsSink>,
    on_state_change: Option<StateChangeCallback>,
}
```

Add constants for state encoding:

```rust
const STATE_CLOSED: u32 = 0;
const STATE_OPEN: u32 = 1;
const STATE_HALF_OPEN: u32 = 2;
```

**Step 2: Initialize atomic_state in constructors**

In `new()`:
```rust
atomic_state: AtomicU32::new(STATE_CLOSED),
```

**Step 3: Update `circuit_state()` to use atomic read**

```rust
pub fn circuit_state(&self) -> CircuitState {
    match self.atomic_state.load(Ordering::Relaxed) {
        STATE_OPEN => CircuitState::Open,
        STATE_HALF_OPEN => CircuitState::HalfOpen,
        _ => CircuitState::Closed,
    }
}
```

**Step 4: Update all state transitions to write atomic_state**

Every place inside the mutex that changes `inner.state` must also store to `atomic_state`. These are:

- `trip_open()`: add `self.atomic_state.store(STATE_OPEN, Ordering::Relaxed);`
- `reset_counters()`: needs `&self` or pass atomic ref -- simplest: make it a method on `CircuitBreaker` instead of associated fn, or update atomic after calling it.
- `try_acquire()` (Open->HalfOpen transition): add `self.atomic_state.store(STATE_HALF_OPEN, Ordering::Relaxed);`
- `force_open()`: add store after setting `inner.state`
- `force_close()`: add store after `reset_counters`

Since `reset_counters` is currently `fn reset_counters(inner: &mut InnerState)` (no `&self`), the pattern is: update atomic AFTER calling reset_counters, at each call site. There are 2 call sites: `force_close` and `close_from_half_open`.

For `close_from_half_open`: change from associated fn to method on `&self`:
```rust
fn close_from_half_open(&self, inner: &mut InnerState) -> (CircuitState, CircuitState) {
    let prev = to_circuit_state(inner.state);
    Self::reset_counters(inner);
    self.atomic_state.store(STATE_CLOSED, Ordering::Relaxed);
    (prev, CircuitState::Closed)
}
```

**Step 5: Run tests**

```bash
rtk cargo nextest run -p nebula-resilience
```

**Step 6: Verify ASM**

```bash
cargo asm -p nebula-resilience --lib "nebula_resilience::circuit_breaker::CircuitBreaker::circuit_state"
```

Expected: no `lock cmpxchg`, just a `mov eax, [rcx + offset]` (plain load) + match on value.

---

## Task 3: Exponential Backoff -- Already Uses `powi` (F5 -- No Change Needed)

**Files:** `crates/resilience/src/retry.rs:103`

The source already uses `multiplier.powi(attempt as i32)`. The ASM showed `call pow` because `as_millis()` returns `u128` which gets converted to `f64` via `__floattidf`. The `powi` itself is fine.

The `call pow` in ASM is actually `__floattidf` (u128->f64 conversion) not `pow`. **No action needed.**

---

## Task 4: Simplify `apply_jitter` NaN Guard (F6)

Minor cleanup -- the existing code is correct but verbose.

**Files:**
- Modify: `crates/resilience/src/retry.rs:446-468`

**Step 1: Simplify the guard**

Current code has a complex bit-level NaN check that LLVM generates 22 instructions for. Simplify:

```rust
fn apply_jitter(delay: Duration, jitter: &JitterConfig, attempt: u32) -> Duration {
    match jitter {
        JitterConfig::None => delay,
        JitterConfig::Full { factor, seed } => {
            let factor = *factor;
            if !factor.is_finite() || factor <= 0.0 {
                return delay;
            }

            let base = delay.as_secs_f64();
            let clamped_factor = factor.min(1.0);
            let rand_val = seed.map_or_else(fastrand::f64, |s| {
                fastrand::Rng::with_seed(s.wrapping_add(u64::from(attempt))).f64()
            });
            let total = clamped_factor.mul_add(base * rand_val, base);
            if !total.is_finite() || total < 0.0 {
                return delay;
            }
            Duration::from_secs_f64(total.min(Duration::MAX.as_secs_f64()))
        }
    }
}
```

Changes:
1. Dereference `factor` once (`let factor = *factor`)
2. Use `mul_add` for fused multiply-add (1 instruction vs 2)
3. Use `< 0.0` instead of `is_sign_negative()` (avoids separate sign-bit check)

**Step 2: Run tests**

```bash
rtk cargo nextest run -p nebula-resilience -- apply_jitter
```

---

## Task 5: Update Doc Comment on OutcomeWindow (Accuracy)

**Files:**
- Modify: `crates/resilience/src/circuit_breaker.rs:187-194`

Update the doc comment to reflect reality:

```rust
/// Fixed-size ring buffer of call outcomes for rate-based circuit breaking.
///
/// Stores failure and slow-call flags in separate byte arrays. The
/// `byte_sum` helper uses chunked iteration that LLVM auto-vectorizes
/// with `psadbw`/`vpsadbw` SIMD instructions at window sizes >= 32.
///
/// Capacity is rounded up to the next power of two so that the ring
/// pointer wraps via bitmask (`& mask`) instead of integer division.
///
/// Made `pub` so it can be benchmarked directly from `benches/sliding_window_cb.rs`.
```

---

## Task 6: Update `.claude/crates/resilience.md` Context

**Files:**
- Modify: `.claude/crates/resilience.md`

Update invariants section to reflect:
- OutcomeWindow uses power-of-two capacity with bitmask wrapping
- `byte_sum` chunked helper enables SIMD vectorization
- `circuit_state()` uses `AtomicU32` lock-free read
- `apply_jitter` uses `mul_add` for FMA

---

## Verification

After all tasks:

```bash
rtk cargo fmt
rtk cargo clippy -p nebula-resilience -- -D warnings
rtk cargo nextest run -p nebula-resilience
rtk cargo test --doc -p nebula-resilience
rtk cargo bench --no-run -p nebula-resilience
```

All must pass green.
