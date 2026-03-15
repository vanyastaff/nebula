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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::time::Duration;
use tracing::warn;

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
/// Logs a `WARN`-level message if the guard is dropped while the gate is
/// already closing (indicating that the caller entered the gate during or
/// after shutdown initiated — a sign of a race in the caller).
pub struct GateGuard {
    inner: Arc<GateInner>,
}

impl Drop for GateGuard {
    fn drop(&mut self) {
        if self.inner.closing.load(Ordering::Acquire) {
            warn!("GateGuard dropped while gate is closing — possible shutdown race");
        }
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
/// - [`close()`](Gate::close) — mark the gate as closing and asynchronously
///   wait until all outstanding guards have been dropped.
#[derive(Clone)]
pub struct Gate {
    inner: Arc<GateInner>,
}

impl std::fmt::Debug for Gate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let closing = self.inner.closing.load(Ordering::Relaxed);
        f.debug_struct("Gate")
            .field("closing", &closing)
            .finish_non_exhaustive()
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
    pub fn enter(&self) -> Result<GateGuard, GateClosed> {
        if self.inner.closing.load(Ordering::Acquire) {
            return Err(GateClosed);
        }
        // try_acquire fails only when the semaphore is closed or has no permits.
        // Since we start with MAX_PERMITS and only close in `Gate::close`,
        // a failure here means the gate is effectively closed.
        self.inner.sem.try_acquire().map_or_else(
            |_| Err(GateClosed),
            |permit| {
                // Forget the permit so its slot is not returned automatically;
                // `GateGuard::drop` will add it back explicitly.
                permit.forget();
                Ok(GateGuard {
                    inner: Arc::clone(&self.inner),
                })
            },
        )
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
                }
                Ok(Err(_)) => {
                    // Semaphore was closed externally — already drained.
                    return;
                }
                Err(_elapsed) => {
                    // Still waiting after 1 s — log a warning and retry.
                    warn!(
                        "Gate::close() is still waiting for active guards to exit \
                         (outstanding work may be stalling shutdown)"
                    );
                }
            }
        }
    }

    /// Returns `true` if the gate has been closed (or is closing).
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.closing.load(Ordering::Acquire)
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
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            drop(guard);
        });

        // close() should complete after the guard is dropped.
        tokio::time::timeout(std::time::Duration::from_secs(2), gate2.close())
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

        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        drop(g1);
        drop(g2);
        drop(g3);

        tokio::time::timeout(std::time::Duration::from_secs(2), close_task)
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
}
