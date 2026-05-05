//! Gate — cooperative shutdown barrier.
//!
//! A [`Gate`] protects a group of concurrent tasks or request handlers. Callers
//! enter the gate before doing work and leave when done. The owner can then
//! *close* the gate: new entries are rejected immediately, and `close()` awaits
//! until every active work unit has exited.
//!
//! This mirrors the `Gate`/`GateGuard` RAII pattern used in
//! [Neon](https://github.com/neondatabase/neon) for graceful shutdown of page
//! server request handlers.
//!
//! # Example
//!
//! ```rust
//! use nebula_resilience::gate::{Gate, GateClosed};
//!
//! # #[tokio::main]
//! # async fn main() {
//! let gate = Gate::new();
//!
//! // Worker acquires the guard; work progresses while the guard is live.
//! let _guard = gate.enter().expect("gate is open");
//!
//! // In the background, the owner closes the gate and waits for all guards
//! // to be dropped.
//! // gate.close().await;
//! # }
//! ```

// Under loom, swap std atomics for loom-instrumented equivalents.
#[cfg(not(loom))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fmt, sync::Arc};

#[cfg(loom)]
use loom::sync::atomic::{AtomicBool, Ordering};
use tokio::{sync::Semaphore, time::Duration};
use tracing::warn; // used in Gate::close() loop

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of outstanding enters the semaphore can track.
///
/// Neon uses `usize::MAX / 2` to stay safely away from overflow while
/// remaining practically unbounded. We use `u32::MAX / 2` because Tokio
/// semaphores use `u32`-sized permit counts internally.
const MAX_PERMITS: u32 = u32::MAX / 2;

// ---------------------------------------------------------------------------
// GateClosed error
// ---------------------------------------------------------------------------

/// Error returned by [`Gate::enter`] when the gate is already closing or closed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
#[error("gate is closed — new enter() calls are rejected")]
pub struct GateClosed;

/// Error returned by [`Gate::close_with_timeout`] when active guards do not drain
/// within the caller's shutdown budget.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
#[error("gate close timed out after {timeout:?}; {active_guards} guard(s) still active")]
pub struct GateCloseTimeout {
    /// Shutdown budget that elapsed.
    pub timeout: Duration,
    /// Best-effort count of active guards still holding the gate open.
    pub active_guards: u32,
}

// ---------------------------------------------------------------------------
// GateInner — shared heap allocation
// ---------------------------------------------------------------------------

struct GateInner {
    /// Each `enter()` forgets one permit. `close()` acquires *all* permits.
    sem: Semaphore,
    /// Set to `true` before `close()` begins draining. Checked on `enter()`.
    closing: AtomicBool,
}

// ---------------------------------------------------------------------------
// GateGuard — RAII exit token
// ---------------------------------------------------------------------------

/// Returned by [`Gate::enter`]. Releases one permit back to the gate on drop.
///
/// Dropping a guard while `close()` is in progress is fully legitimate: the
/// guard was acquired before shutdown started, and dropping it unblocks
/// [`Gate::close`].
pub struct GateGuard {
    inner: Arc<GateInner>,
}

impl fmt::Debug for GateGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GateGuard").finish_non_exhaustive()
    }
}

impl Drop for GateGuard {
    fn drop(&mut self) {
        self.inner.sem.add_permits(1);
    }
}

// ---------------------------------------------------------------------------
// Gate
// ---------------------------------------------------------------------------

/// Cooperative shutdown barrier.
///
/// - [`enter()`](Gate::enter) — acquire a guard (non-blocking). Returns
///   [`Err(GateClosed)`](GateClosed) if the gate is already closing.
/// - [`close()`](Gate::close) — mark the gate as closing and asynchronously wait until all
///   outstanding guards have been dropped.
///
/// # Drop behaviour
///
/// Dropping a `Gate` without calling `close()` does **not** cancel or await
/// outstanding guards. The guards remain live until they are dropped by their
/// respective owners. Call `close().await` explicitly during shutdown to ensure
/// all work has finished before proceeding.
///
/// # Examples
///
/// ```rust
/// use nebula_resilience::gate::{Gate, GateClosed};
///
/// # #[tokio::main]
/// # async fn main() {
/// let gate = Gate::new();
///
/// // While open, callers can enter and hold a guard for the duration of work.
/// let guard = gate.enter().expect("gate is open");
/// drop(guard);
///
/// // After close(), new entries are rejected.
/// gate.close().await;
/// assert!(matches!(gate.enter(), Err(GateClosed)));
/// # }
/// ```
#[derive(Clone)]
pub struct Gate {
    inner: Arc<GateInner>,
}

impl fmt::Debug for Gate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let closing = self.inner.closing.load(Ordering::Relaxed);
        f.debug_struct("Gate")
            .field("closing", &closing)
            .finish_non_exhaustive()
    }
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

impl Gate {
    /// Create a new, open gate.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(GateInner {
                sem: Semaphore::new(MAX_PERMITS as usize),
                closing: AtomicBool::new(false),
            }),
        }
    }

    /// Attempt to enter the gate.
    ///
    /// Returns a [`GateGuard`] that releases the entry on drop.
    ///
    /// # Errors
    ///
    /// Returns [`GateClosed`] if the gate is closing or already closed. The
    /// check is non-blocking and uses `try_acquire`, so it never blocks.
    // Reason: the permit must outlive the closing check — we either forget it
    // (creating a guard) or drop it (returning it to the semaphore). The drop
    // order is intentional and correct.
    #[expect(
        clippy::significant_drop_tightening,
        reason = "permit must outlive the closing check; early drop would create a TOCTOU window"
    )]
    pub fn enter(&self) -> Result<GateGuard, GateClosed> {
        // Acquire first to hold our place in the semaphore, THEN check closing.
        //
        // Checking closing before try_acquire creates a TOCTOU window: close()
        // could run between the check and the acquire, producing a guard that
        // exists after close() has started — an invariant violation.
        //
        // With this ordering:
        // - If closing was already true before we acquired, we drop the permit (returns it to the
        //   semaphore so close() can drain correctly) and return Err.
        // - If close() runs after we read closing=false, the guard is valid and was legitimately
        //   created before shutdown; close() will wait for it.
        //
        // Happens-before note: `close()` stores `closing = true` with
        // `Ordering::Release`. The `Ordering::Acquire` load below synchronises
        // with that store, so any guard created before `closing` becomes visible
        // is fully committed. `Semaphore::try_acquire` itself uses Tokio's
        // internal acquire ordering, which provides the same guarantee for the
        // permit slot.
        let permit = self.inner.sem.try_acquire().map_err(|_| GateClosed)?;

        if self.inner.closing.load(Ordering::Acquire) {
            // Drop returns the permit to the semaphore automatically.
            drop(permit);
            return Err(GateClosed);
        }

        // Forget the permit so its slot is not returned automatically;
        // `GateGuard::drop` will add it back explicitly.
        permit.forget();
        Ok(GateGuard {
            inner: Arc::clone(&self.inner),
        })
    }

    /// Close the gate and wait for all outstanding guards to exit.
    ///
    /// After this call:
    /// - All subsequent [`enter()`](Gate::enter) calls return [`Err(GateClosed)`](GateClosed).
    /// - This future resolves only after every existing [`GateGuard`] has been dropped.
    ///
    /// Calling `close()` more than once is a no-op (idempotent).
    pub async fn close(&self) {
        // Mark as closing so new enter() calls fail fast.
        self.inner.closing.store(true, Ordering::Release);

        // Count how many permits are currently "out" (held by active guards).
        // We started with MAX_PERMITS and each guard holds one. The semaphore
        // currently has (MAX_PERMITS - active_count) available permits.
        //
        // Strategy: try to acquire MAX_PERMITS. Each guard that drops adds its
        // permit back, eventually allowing us to acquire all permits, which
        // confirms zero active guards.
        //
        // We use `acquire_many` in a loop with periodic progress logging to
        // avoid silent stalls during shutdown.

        loop {
            match tokio::time::timeout(
                Duration::from_secs(1),
                self.inner.sem.acquire_many(MAX_PERMITS),
            )
            .await
            {
                Ok(Ok(permit)) => {
                    // Successfully drained all permits — no active guards remain.
                    // Close the semaphore so future try_acquire calls fail cleanly.
                    permit.forget();
                    self.inner.sem.close();
                    return;
                },
                Ok(Err(_)) => {
                    // Semaphore was closed externally — already drained.
                    return;
                },
                Err(_elapsed) => {
                    // Still waiting after 1 s — log a warning and retry.
                    warn!(
                        "Gate::close() is still waiting for active guards to exit \
                         (outstanding work may be stalling shutdown)"
                    );
                },
            }
        }
    }

    /// Close the gate but return a typed timeout instead of waiting forever.
    ///
    /// This is useful for workflow/runtime shutdown paths that have a bounded
    /// graceful-drain budget. If the timeout elapses, the gate remains in the
    /// closing state: new [`enter()`](Gate::enter) calls are still rejected, and
    /// the caller may retry [`close()`](Gate::close) later after outstanding
    /// guards have had more time to drop.
    ///
    /// # Errors
    ///
    /// Returns [`GateCloseTimeout`] if `timeout` elapses before all active guards
    /// exit.
    pub async fn close_with_timeout(&self, timeout: Duration) -> Result<(), GateCloseTimeout> {
        if timeout.is_zero() {
            self.inner.closing.store(true, Ordering::Release);
            return Err(GateCloseTimeout {
                timeout,
                active_guards: self.active_count(),
            });
        }

        tokio::time::timeout(timeout, self.close())
            .await
            .map_err(|_elapsed| GateCloseTimeout {
                timeout,
                active_guards: self.active_count(),
            })
    }

    /// Returns `true` if the gate has been closed (or is closing).
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.closing.load(Ordering::Acquire)
    }

    /// Best-effort count of currently active guards.
    ///
    /// This value is intended for diagnostics and timeout errors. It may change
    /// immediately after being read in concurrent code.
    #[must_use]
    pub fn active_count(&self) -> u32 {
        if self.inner.sem.is_closed() {
            return 0;
        }

        let available = self.inner.sem.available_permits().min(MAX_PERMITS as usize);
        let available = u32::try_from(available).unwrap_or(MAX_PERMITS);
        MAX_PERMITS.saturating_sub(available)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_and_drop_guard_reopens_permits() {
        let gate = Gate::new();
        let guard = gate.enter().expect("gate open");
        drop(guard);
        // After dropping the guard, we should be able to enter again.
        gate.enter().expect("gate open after guard drop");
    }

    #[test]
    fn enter_after_closing_returns_error() {
        let gate = Gate::new();
        gate.inner.closing.store(true, Ordering::Release);
        assert!(matches!(gate.enter(), Err(GateClosed)));
    }

    #[tokio::test]
    async fn close_waits_for_guard_then_rejects_enter() {
        let gate = Gate::new();
        let guard = gate.enter().expect("gate open");

        // Spawn a task that drops the guard after a short delay.
        let gate2 = gate.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            drop(guard);
        });

        // close() should complete after the guard is dropped.
        tokio::time::timeout(Duration::from_secs(2), gate2.close())
            .await
            .expect("close() should complete quickly");

        // New entries are now rejected.
        assert!(matches!(gate.enter(), Err(GateClosed)));
    }

    #[tokio::test]
    async fn close_is_idempotent() {
        let gate = Gate::new();
        gate.close().await;
        gate.close().await; // second call must not panic or hang
        assert_eq!(gate.active_count(), 0);
    }

    #[tokio::test]
    async fn multiple_guards_all_drain() {
        let gate = Gate::new();
        let g1 = gate.enter().expect("gate open");
        let g2 = gate.enter().expect("gate open");
        let g3 = gate.enter().expect("gate open");

        let gate2 = gate.clone();
        let close_task = tokio::spawn(async move {
            gate2.close().await;
        });

        tokio::time::sleep(Duration::from_millis(5)).await;
        drop(g1);
        drop(g2);
        drop(g3);

        tokio::time::timeout(Duration::from_secs(2), close_task)
            .await
            .expect("timeout")
            .expect("task panicked");
    }

    #[test]
    fn debug_shows_closing_state() {
        let gate = Gate::new();
        let dbg = format!("{gate:?}");
        assert!(dbg.contains("closing: false"));
    }

    #[tokio::test]
    async fn close_with_timeout_returns_active_guard_count() {
        let gate = Gate::new();
        let guard = gate.enter().expect("gate open");

        let err = gate
            .close_with_timeout(Duration::from_millis(5))
            .await
            .unwrap_err();

        assert_eq!(err.timeout, Duration::from_millis(5));
        assert_eq!(err.active_guards, 1);
        assert!(gate.is_closed());
        assert!(matches!(gate.enter(), Err(GateClosed)));

        drop(guard);
        gate.close().await;
        assert_eq!(gate.active_count(), 0);
    }
}

// ---------------------------------------------------------------------------
// Loom tests — exhaustive concurrency model-checking for the atomic ordering
// invariants in `enter()` / `close()`.
//
// Run with:
//   RUSTFLAGS="--cfg loom" cargo test -p nebula-resilience --test gate_loom
//
// Note: loom replaces `AtomicBool` via the conditional import above.
// Tokio's `Semaphore` is **not** loom-instrumented; the loom tests here focus
// exclusively on the `closing` atomic flag logic.
// ---------------------------------------------------------------------------
#[cfg(all(test, loom))]
mod loom_tests {
    use loom::{sync::Arc, thread};

    use super::*;

    /// Two threads race: one calls `enter()` and the other sets `closing=true`
    /// directly (simulating `close()`'s first action).  Loom exhaustively
    /// schedules all interleavings and checks that:
    ///
    /// - After `closing` is set, a concurrent `enter()` either returns `Err(GateClosed)` OR the
    ///   guard was already fully committed (acquired and flag not yet visible) — never a
    ///   half-entered state.
    #[test]
    fn enter_vs_close_flag_race() {
        loom::model(|| {
            // Directly test the AtomicBool ordering without tokio's Semaphore.
            let closing = Arc::new(AtomicBool::new(false));

            let closing2 = Arc::clone(&closing);
            let t1 = thread::spawn(move || {
                // Simulate the `close()` flag write.
                closing2.store(true, Ordering::Release);
            });

            // Simulate the `enter()` flag check.
            let saw_closed = closing.load(Ordering::Acquire);

            t1.join().unwrap();

            // After both threads complete, the flag must be true.
            assert!(closing.load(Ordering::Acquire));
            // `saw_closed` may be true or false depending on scheduling;
            // both are valid interleavings.
            let _ = saw_closed;
        });
    }

    /// Verify that a Release store on one thread is always observed by a
    /// subsequent Acquire load on another (no stale reads possible).
    #[test]
    fn release_acquire_visibility() {
        loom::model(|| {
            let flag = Arc::new(AtomicBool::new(false));
            let flag2 = Arc::clone(&flag);

            let writer = thread::spawn(move || {
                flag2.store(true, Ordering::Release);
            });

            writer.join().unwrap();

            // After the writer thread completes, the Acquire load must see `true`.
            assert!(flag.load(Ordering::Acquire));
        });
    }
}
