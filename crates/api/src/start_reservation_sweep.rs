//! Periodic eviction of expired keyed-start reservations.
//!
//! [`StartAcceptanceStore::accept_keyed_start`] records a reservation per
//! `(scope, start_key)` so a replayed start returns the original receipt
//! instead of launching a second execution. Nothing ever removed those rows:
//! the port has exposed
//! [`evict_reservations_older_than`](StartAcceptanceStore::evict_reservations_older_than)
//! and all three adapters implement it, but no caller existed, so the table
//! grew for the life of the deployment. A cleanup contract with no caller is
//! not a retention policy — it only looks like one.
//!
//! ## Retention
//!
//! A start key answers exactly the question `Idempotency-Key` answers
//! everywhere else in this API: *for how long may a client repeat this request
//! and get the first result back?* Two different answers to that question would
//! be a bug an operator could not see, so the retention here is the configured
//! idempotency TTL and the cadence is the configured idempotency sweep
//! interval — one knob, one meaning.
//!
//! Evicting a reservation does not delete the execution it points at. It only
//! stops that start key from replaying: a client repeating the key past the
//! retention starts a new execution, which is the documented meaning of the
//! window elapsing.

use std::{sync::Arc, time::Duration};

use nebula_storage_port::store::StartAcceptanceStore;
use tokio_util::sync::CancellationToken;

/// An app-owned sweep that expires keyed-start reservations on a fixed cadence.
pub struct StartReservationSweeper {
    store: Arc<dyn StartAcceptanceStore>,
    retention: Duration,
    interval: Duration,
}

impl StartReservationSweeper {
    /// Build a sweeper.
    ///
    /// Returns `None` when `interval` is zero, the same "disabled" convention
    /// the idempotency sweep uses for single-process and development runs.
    /// Returning `None` rather than a sweeper that never fires keeps the
    /// disabled case visible at the composition root instead of hiding it
    /// inside a loop that quietly does nothing.
    #[must_use]
    pub fn new(
        store: Arc<dyn StartAcceptanceStore>,
        retention: Duration,
        interval: Duration,
    ) -> Option<Self> {
        if interval.is_zero() {
            return None;
        }
        Some(Self {
            store,
            retention,
            interval,
        })
    }

    /// Run until `shutdown` is cancelled.
    ///
    /// A failed sweep is logged and retried on the next tick; it must not end
    /// the task, because the alternative to one missed sweep is no sweeps at
    /// all for the rest of the process's life.
    pub async fn run(self, shutdown: CancellationToken) {
        tracing::info!(
            retention_secs = self.retention.as_secs(),
            interval_secs = self.interval.as_secs(),
            "start-key reservation sweep started"
        );
        let mut ticker = tokio::time::interval(self.interval);
        // The first tick fires immediately; skip it. At startup the newest
        // reservations are the ones most likely to be replayed, and a sweep
        // competing with the boot path buys nothing.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;

        loop {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => {
                    tracing::info!("start-key reservation sweep stopping");
                    return;
                }
                _ = ticker.tick() => {
                    match self.store.evict_reservations_older_than(self.retention).await {
                        Ok(0) => {
                            tracing::debug!("start-key reservation sweep: nothing expired");
                        },
                        Ok(evicted) => {
                            tracing::info!(
                                evicted,
                                retention_secs = self.retention.as_secs(),
                                "start-key reservation sweep evicted expired reservations"
                            );
                        },
                        Err(error) => {
                            tracing::error!(
                                %error,
                                "start-key reservation sweep failed; retrying next tick"
                            );
                        },
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    };

    use nebula_storage_port::{
        StorageError,
        store::{KeyedStart, StartAcceptance},
    };

    use super::*;

    /// Records what the sweep asked for, so the test can assert the retention
    /// actually reaches the store rather than that a loop merely ran.
    #[derive(Debug)]
    struct RecordingStore {
        sweeps: AtomicUsize,
        last_retention_secs: AtomicU64,
    }

    #[async_trait::async_trait]
    impl StartAcceptanceStore for RecordingStore {
        async fn accept_keyed_start(
            &self,
            _start: &KeyedStart<'_>,
        ) -> Result<StartAcceptance, StorageError> {
            unreachable!("the sweep never accepts starts")
        }

        async fn evict_reservations_older_than(
            &self,
            retention: Duration,
        ) -> Result<u64, StorageError> {
            self.last_retention_secs
                .store(retention.as_secs(), Ordering::SeqCst);
            self.sweeps.fetch_add(1, Ordering::SeqCst);
            Ok(0)
        }
    }

    /// A zero interval disables the sweep, and says so by not producing one —
    /// so the composition root can report the deployment is accumulating
    /// reservations instead of silently spawning a task that never fires.
    #[test]
    fn zero_interval_produces_no_sweeper() {
        let store = Arc::new(RecordingStore {
            sweeps: AtomicUsize::new(0),
            last_retention_secs: AtomicU64::new(0),
        });
        assert!(
            StartReservationSweeper::new(store, Duration::from_hours(1), Duration::ZERO).is_none()
        );
    }

    /// The sweep evicts on its cadence and passes the configured retention
    /// through unchanged.
    ///
    /// Falsifiability: drop the `evict_reservations_older_than` call from the
    /// tick arm and `sweeps` stays 0; pass a different duration and the
    /// retention assertion fails.
    #[tokio::test(start_paused = true)]
    async fn sweeps_on_cadence_with_the_configured_retention() {
        let store = Arc::new(RecordingStore {
            sweeps: AtomicUsize::new(0),
            last_retention_secs: AtomicU64::new(0),
        });
        let sweeper = StartReservationSweeper::new(
            Arc::clone(&store) as Arc<dyn StartAcceptanceStore>,
            Duration::from_hours(24),
            Duration::from_mins(1),
        )
        .expect("a non-zero interval must produce a sweeper");

        let shutdown = CancellationToken::new();
        let task = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { sweeper.run(shutdown).await }
        });

        // Let the task reach its first `tick().await` before moving time.
        tokio::task::yield_now().await;

        // The immediate first tick is deliberately skipped, so nothing has run
        // yet — a sweep here would mean the loop competes with the boot path.
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            store.sweeps.load(Ordering::SeqCst),
            0,
            "the startup tick must be skipped"
        );

        tokio::time::advance(Duration::from_mins(1)).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(
            store.sweeps.load(Ordering::SeqCst) >= 1,
            "one interval must produce a sweep"
        );
        assert_eq!(
            store.last_retention_secs.load(Ordering::SeqCst),
            86_400,
            "the configured retention must reach the store unchanged"
        );

        shutdown.cancel();
        task.await.expect("the sweep must stop on cancellation");
    }
}
