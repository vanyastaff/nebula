//! Periodic deletion of expired sessions, verification tokens and OAuth
//! states.
//!
//! Every read already refuses an expired row, and the session, verification
//! token and OAuth state repositories have each exposed `cleanup_expired`,
//! but nothing called it, so the tables kept every login, reset link and
//! abandoned OAuth flow for the life of the deployment.
//!
//! The cadence is the idempotency sweep interval: one operator knob for the
//! API's housekeeping, and `0` disables both.

use std::{sync::Arc, time::Duration};

use tokio_util::sync::CancellationToken;

use crate::domain::auth::backend::AuthBackend;

/// An app-owned sweep that deletes expired authentication rows on a fixed
/// cadence.
pub struct AuthExpirySweeper {
    backend: Arc<dyn AuthBackend>,
    interval: Duration,
}

impl AuthExpirySweeper {
    /// Build a sweeper. Returns `None` when `interval` is zero, the
    /// "disabled" convention the other API sweeps use.
    #[must_use]
    pub fn new(backend: Arc<dyn AuthBackend>, interval: Duration) -> Option<Self> {
        if interval.is_zero() {
            return None;
        }
        Some(Self { backend, interval })
    }

    /// Run until `shutdown` is cancelled. A failed sweep is logged and
    /// retried on the next tick.
    pub async fn run(self, shutdown: CancellationToken) {
        tracing::info!(
            interval_secs = self.interval.as_secs(),
            "auth expiry sweep started"
        );
        let mut ticker = tokio::time::interval(self.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first tick fires immediately; skip it so the sweep does not
        // compete with the boot path.
        ticker.tick().await;

        loop {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => {
                    tracing::info!("auth expiry sweep stopping");
                    return;
                }
                _ = ticker.tick() => {
                    match self.backend.sweep_expired().await {
                        Ok(0) => tracing::debug!("auth expiry sweep: nothing expired"),
                        Ok(deleted) => {
                            tracing::info!(deleted, "auth expiry sweep deleted expired rows");
                        },
                        Err(error) => {
                            tracing::error!(%error, "auth expiry sweep failed; retrying next tick");
                        },
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::auth::backend::InMemoryAuthBackend;

    #[test]
    fn zero_interval_produces_no_sweeper() {
        let backend: Arc<dyn AuthBackend> = Arc::new(InMemoryAuthBackend::new());
        assert!(AuthExpirySweeper::new(backend, Duration::ZERO).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn the_sweep_stops_on_shutdown() {
        let backend: Arc<dyn AuthBackend> = Arc::new(InMemoryAuthBackend::new());
        let sweeper = AuthExpirySweeper::new(backend, Duration::from_mins(1))
            .expect("a non-zero interval must produce a sweeper");
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(sweeper.run(shutdown.clone()));
        tokio::time::advance(Duration::from_mins(3)).await;
        shutdown.cancel();
        task.await.expect("the sweep ends cleanly");
    }
}
