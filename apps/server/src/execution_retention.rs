//! Opt-in deletion of finished executions past a configured age.
//!
//! Without it the deployment keeps every execution it ever ran, with its
//! journal, node results and queued commands. `API_EXECUTION_RETENTION` sets
//! how long a terminal execution stays after it finished; unset or `0`
//! keeps executions forever, so no user data is deleted without an
//! operator's decision. Live executions are never deleted.

use std::sync::Arc;
use std::time::Duration;

use nebula_storage_port::store::ExecutionRetention;
use tokio_util::sync::CancellationToken;

/// The environment variable that turns retention on.
pub(crate) const RETENTION_VAR: &str = "API_EXECUTION_RETENTION";

/// The shortest accepted retention. A finished execution is what a client
/// reads its result from and what a replayed start key answers with, so
/// deleting it within minutes of finishing would break both.
const MIN_RETENTION: Duration = Duration::from_hours(1);

/// How often the sweep runs.
const SWEEP_INTERVAL: Duration = Duration::from_hours(1);

/// Executions deleted per statement. Each batch is its own short write, so a
/// large backlog never holds the database for one long transaction.
const BATCH_SIZE: u32 = 500;

/// An invalid `API_EXECUTION_RETENTION`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RetentionConfigError {
    /// The value is not `0` or a whole number followed by `s`, `m`, `h` or `d`.
    #[error("{RETENTION_VAR} must be 0 or a whole number followed by s, m, h or d (e.g. 30d)")]
    Malformed,
    /// The value is shorter than one hour.
    #[error("{RETENTION_VAR} must be at least 1h")]
    TooShort,
}

/// Read the retention from the environment: `None` keeps executions forever.
///
/// # Errors
/// Returns [`RetentionConfigError`] for a malformed or too-short value.
pub(crate) fn retention_from_env() -> Result<Option<Duration>, RetentionConfigError> {
    match std::env::var(RETENTION_VAR) {
        Ok(raw) => parse_retention(&raw),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(RetentionConfigError::Malformed),
    }
}

/// Parse `0`, empty, or `<n><unit>` with unit `s`, `m`, `h` or `d`.
fn parse_retention(raw: &str) -> Result<Option<Duration>, RetentionConfigError> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "0" {
        return Ok(None);
    }
    let split = raw.len() - 1;
    let (amount, unit) = (raw.get(..split), raw.get(split..));
    let unit_secs: u64 = match unit {
        Some("s") => 1,
        Some("m") => 60,
        Some("h") => 3_600,
        Some("d") => 86_400,
        _ => return Err(RetentionConfigError::Malformed),
    };
    let amount: u64 = amount
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|digits| digits.parse().ok())
        .ok_or(RetentionConfigError::Malformed)?;
    let retention = amount
        .checked_mul(unit_secs)
        .map(Duration::from_secs)
        .ok_or(RetentionConfigError::Malformed)?;
    if retention < MIN_RETENTION {
        return Err(RetentionConfigError::TooShort);
    }
    Ok(Some(retention))
}

/// An app-owned sweep that deletes terminal executions older than the
/// retention, in bounded batches, once at startup and then hourly.
pub(crate) struct ExecutionRetentionSweeper {
    store: Arc<dyn ExecutionRetention>,
    retention: Duration,
    interval: Duration,
    batch_size: u32,
}

impl ExecutionRetentionSweeper {
    /// Build a sweeper for a configured retention.
    pub(crate) fn new(store: Arc<dyn ExecutionRetention>, retention: Duration) -> Self {
        Self {
            store,
            retention,
            interval: SWEEP_INTERVAL,
            batch_size: BATCH_SIZE,
        }
    }

    /// Run until `shutdown` is cancelled. A failed batch is logged and the
    /// sweep resumes on the next tick.
    pub(crate) async fn run(self, shutdown: CancellationToken) {
        tracing::info!(
            retention_secs = self.retention.as_secs(),
            interval_secs = self.interval.as_secs(),
            "execution retention sweep started"
        );
        let mut ticker = tokio::time::interval(self.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => {
                    tracing::info!("execution retention sweep stopping");
                    return;
                }
                _ = ticker.tick() => self.sweep(&shutdown).await,
            }
        }
    }

    /// Delete batches until one comes back short, an error occurs, or
    /// shutdown is requested.
    async fn sweep(&self, shutdown: &CancellationToken) {
        let mut deleted_total: u64 = 0;
        loop {
            match self
                .store
                .delete_finished_executions(self.retention, self.batch_size)
                .await
            {
                Ok(deleted) => {
                    deleted_total = deleted_total.saturating_add(deleted);
                    if deleted < u64::from(self.batch_size) || shutdown.is_cancelled() {
                        break;
                    }
                    tokio::task::yield_now().await;
                },
                Err(error) => {
                    tracing::error!(%error, "execution retention sweep failed; retrying next tick");
                    break;
                },
            }
        }
        if deleted_total == 0 {
            tracing::debug!("execution retention sweep: nothing past retention");
        } else {
            tracing::info!(
                deleted = deleted_total,
                retention_secs = self.retention.as_secs(),
                "execution retention sweep deleted finished executions"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use nebula_storage_port::StorageError;

    use super::*;

    #[test]
    fn unset_empty_and_zero_keep_executions_forever() {
        assert!(matches!(parse_retention(""), Ok(None)));
        assert!(matches!(parse_retention("0"), Ok(None)));
        assert!(matches!(parse_retention("  0 "), Ok(None)));
    }

    #[test]
    fn units_scale_to_seconds() {
        assert_eq!(
            parse_retention("30d").ok().flatten(),
            Some(Duration::from_hours(720))
        );
        assert_eq!(
            parse_retention("12h").ok().flatten(),
            Some(Duration::from_hours(12))
        );
        assert_eq!(
            parse_retention("90m").ok().flatten(),
            Some(Duration::from_mins(90))
        );
        assert_eq!(
            parse_retention("7200s").ok().flatten(),
            Some(Duration::from_hours(2))
        );
    }

    #[test]
    fn malformed_and_short_values_are_rejected() {
        for raw in [
            "30",
            "d",
            "-1d",
            "1.5d",
            "30 d",
            "30w",
            "99999999999999999999d",
        ] {
            assert!(
                matches!(parse_retention(raw), Err(RetentionConfigError::Malformed)),
                "{raw:?} must be rejected as malformed"
            );
        }
        assert!(matches!(
            parse_retention("59m"),
            Err(RetentionConfigError::TooShort)
        ));
    }

    /// Hands out a scripted sequence of batch results and records each call.
    #[derive(Debug)]
    struct ScriptedStore {
        results: Mutex<Vec<Result<u64, StorageError>>>,
        calls: Mutex<Vec<(Duration, u32)>>,
    }

    #[async_trait::async_trait]
    impl ExecutionRetention for ScriptedStore {
        async fn delete_finished_executions(
            &self,
            retention: Duration,
            limit: u32,
        ) -> Result<u64, StorageError> {
            self.calls.lock().unwrap().push((retention, limit));
            let mut results = self.results.lock().unwrap();
            if results.is_empty() {
                Ok(0)
            } else {
                results.remove(0)
            }
        }
    }

    fn sweeper(store: Arc<ScriptedStore>) -> ExecutionRetentionSweeper {
        let mut sweeper = ExecutionRetentionSweeper::new(store, Duration::from_hours(24));
        sweeper.batch_size = 2;
        sweeper
    }

    /// Full batches repeat until a short one; the configured retention and
    /// batch size reach the store unchanged.
    #[tokio::test]
    async fn a_sweep_repeats_full_batches_until_a_short_one() {
        let store = Arc::new(ScriptedStore {
            results: Mutex::new(vec![Ok(2), Ok(2), Ok(1), Ok(2)]),
            calls: Mutex::new(Vec::new()),
        });
        sweeper(Arc::clone(&store))
            .sweep(&CancellationToken::new())
            .await;
        let calls = store.calls.lock().unwrap().clone();
        assert_eq!(calls, vec![(Duration::from_hours(24), 2); 3]);
    }

    #[tokio::test]
    async fn a_failed_batch_ends_the_sweep() {
        let store = Arc::new(ScriptedStore {
            results: Mutex::new(vec![Ok(2), Err(StorageError::Connection("down".into()))]),
            calls: Mutex::new(Vec::new()),
        });
        sweeper(Arc::clone(&store))
            .sweep(&CancellationToken::new())
            .await;
        assert_eq!(store.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn the_sweep_runs_at_startup_and_stops_on_shutdown() {
        let store = Arc::new(ScriptedStore {
            results: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
        });
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(sweeper(Arc::clone(&store)).run(shutdown.clone()));
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            store.calls.lock().unwrap().len(),
            1,
            "the first sweep runs at startup"
        );
        shutdown.cancel();
        task.await.expect("the sweep ends cleanly");
    }
}
