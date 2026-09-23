//! Bounded process-local driver for durable due-refresh discovery.
//!
//! The schedule is deliberately at-least-once. Every candidate is rechecked by
//! the credential service and enters provider egress only through the existing
//! cross-replica refresh claim.

use std::{fmt, num::NonZeroU16, sync::Arc, time::Duration};

use async_trait::async_trait;
use nebula_storage_port::{
    CredentialRefreshCursor, CredentialRefreshHorizon, CredentialRefreshPageSize,
    CredentialRefreshSchedule, DueCredentialRefresh,
};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

/// Validated scheduling policy for one runtime replica.
#[derive(Debug, Clone, Copy)]
pub struct CredentialRefreshSchedulerConfig {
    /// Delay between complete due scans.
    pub cadence: Duration,
    /// Backend-clock look-ahead used to select expiring credentials.
    pub horizon: CredentialRefreshHorizon,
    /// Maximum rows fetched in one storage call.
    pub page_size: CredentialRefreshPageSize,
    /// Maximum pages consumed during one tick.
    pub max_pages_per_tick: NonZeroU16,
    /// Maximum concurrent service refresh calls in this replica.
    pub concurrency: NonZeroU16,
}

impl Default for CredentialRefreshSchedulerConfig {
    fn default() -> Self {
        const DEFAULT_MAX_PAGES_PER_TICK: NonZeroU16 =
            NonZeroU16::new(10).expect("default refresh page count is non-zero");
        const DEFAULT_CONCURRENCY: NonZeroU16 =
            NonZeroU16::new(8).expect("default refresh concurrency is non-zero");

        Self {
            cadence: Duration::from_secs(30),
            horizon: CredentialRefreshHorizon::default(),
            page_size: CredentialRefreshPageSize::default(),
            max_pages_per_tick: DEFAULT_MAX_PAGES_PER_TICK,
            concurrency: DEFAULT_CONCURRENCY,
        }
    }
}

impl CredentialRefreshSchedulerConfig {
    /// Validate cadence constraints before spawning background work.
    pub fn validate(self) -> Result<Self, CredentialRefreshSchedulerConfigError> {
        if self.cadence.is_zero() {
            return Err(CredentialRefreshSchedulerConfigError::ZeroCadence);
        }
        Ok(self)
    }
}

/// Invalid due-refresh scheduler configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialRefreshSchedulerConfigError {
    /// A zero cadence would create a busy loop.
    #[error("credential refresh scheduler cadence must be non-zero")]
    ZeroCadence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScheduledRefreshDisposition {
    Refreshed,
    NoLongerDue,
    Unsupported,
    Deferred,
    Blocked,
    ReauthRequired,
    TransientFailure,
    OutcomeUnknown,
}

#[async_trait]
pub(crate) trait ScheduledRefreshExecutor: Send + Sync + 'static {
    async fn refresh_due(&self, candidate: DueCredentialRefresh) -> ScheduledRefreshDisposition;
}

pub(crate) struct CredentialRefreshSchedulerTask {
    shutdown: CancellationToken,
    task: Option<JoinHandle<()>>,
}

impl fmt::Debug for CredentialRefreshSchedulerTask {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialRefreshSchedulerTask")
            .field(
                "is_finished",
                &self.task.as_ref().is_none_or(JoinHandle::is_finished),
            )
            .finish()
    }
}

impl CredentialRefreshSchedulerTask {
    pub(crate) fn spawn(
        schedule: Arc<dyn CredentialRefreshSchedule>,
        executor: Arc<dyn ScheduledRefreshExecutor>,
        config: CredentialRefreshSchedulerConfig,
    ) -> Result<Self, CredentialRefreshSchedulerConfigError> {
        let config = config.validate()?;
        let shutdown = CancellationToken::new();
        let task_shutdown = shutdown.clone();
        let task = tokio::spawn(async move {
            run_loop(schedule, executor, config, task_shutdown).await;
        });
        Ok(Self {
            shutdown,
            task: Some(task),
        })
    }

    pub(crate) async fn shutdown(&mut self) {
        self.shutdown.cancel();
        if let Some(mut task) = self.task.take()
            && let Err(error) = (&mut task).await
            && !error.is_cancelled()
        {
            tracing::error!(%error, "credential refresh scheduler failed during shutdown");
        }
    }
}

impl Drop for CredentialRefreshSchedulerTask {
    fn drop(&mut self) {
        self.shutdown.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn run_loop(
    schedule: Arc<dyn CredentialRefreshSchedule>,
    executor: Arc<dyn ScheduledRefreshExecutor>,
    config: CredentialRefreshSchedulerConfig,
    shutdown: CancellationToken,
) {
    run_tick(schedule.as_ref(), Arc::clone(&executor), config, &shutdown).await;
    let mut ticker = tokio::time::interval(config.cadence);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ticker.tick().await;
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = ticker.tick() => {
                run_tick(schedule.as_ref(), Arc::clone(&executor), config, &shutdown).await;
            }
        }
    }
}

async fn run_tick(
    schedule: &dyn CredentialRefreshSchedule,
    executor: Arc<dyn ScheduledRefreshExecutor>,
    config: CredentialRefreshSchedulerConfig,
    shutdown: &CancellationToken,
) {
    let mut after: Option<CredentialRefreshCursor> = None;
    for _ in 0..config.max_pages_per_tick.get() {
        if shutdown.is_cancelled() {
            return;
        }
        let candidates = match schedule
            .scan_due(after.as_ref(), config.horizon, config.page_size)
            .await
        {
            Ok(candidates) => candidates,
            Err(error) => {
                tracing::warn!(%error, "credential due-refresh scan failed");
                return;
            },
        };
        if candidates.is_empty() {
            return;
        }
        after = candidates.last().map(DueCredentialRefresh::cursor);
        let page_full = candidates.len() == usize::from(config.page_size.get());
        execute_page(candidates, executor.clone(), config.concurrency, shutdown).await;
        if !page_full {
            return;
        }
    }
    tracing::warn!(
        max_pages = config.max_pages_per_tick.get(),
        "credential due-refresh scan reached its per-tick page bound"
    );
}

async fn execute_page(
    candidates: Vec<DueCredentialRefresh>,
    executor: Arc<dyn ScheduledRefreshExecutor>,
    concurrency: NonZeroU16,
    shutdown: &CancellationToken,
) {
    let mut candidates = candidates.into_iter();
    loop {
        let mut tasks = JoinSet::new();
        for candidate in candidates.by_ref().take(usize::from(concurrency.get())) {
            let executor = Arc::clone(&executor);
            tasks.spawn(async move { executor.refresh_due(candidate).await });
        }
        if tasks.is_empty() {
            return;
        }
        loop {
            tokio::select! {
                () = shutdown.cancelled() => {
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                    return;
                },
                result = tasks.join_next() => match result {
                    Some(Ok(disposition)) => {
                        tracing::debug!(?disposition, "credential scheduled refresh completed");
                    },
                    Some(Err(error)) if !error.is_cancelled() => {
                        tracing::error!(%error, "credential scheduled refresh task failed");
                    },
                    Some(Err(_)) => {},
                    None => break,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::{DateTime, Utc};
    use nebula_storage_port::{
        CredentialId, CredentialOwner, CredentialRefreshScheduleError, CredentialSelector,
    };

    use super::*;

    #[derive(Debug)]
    struct FixedSchedule {
        candidates: Vec<DueCredentialRefresh>,
        scans: AtomicUsize,
    }

    #[async_trait]
    impl CredentialRefreshSchedule for FixedSchedule {
        async fn scan_due(
            &self,
            after: Option<&CredentialRefreshCursor>,
            _horizon: CredentialRefreshHorizon,
            limit: CredentialRefreshPageSize,
        ) -> Result<Vec<DueCredentialRefresh>, CredentialRefreshScheduleError> {
            self.scans.fetch_add(1, Ordering::SeqCst);
            let start = after
                .and_then(|cursor| {
                    self.candidates
                        .iter()
                        .position(|candidate| candidate.cursor() == *cursor)
                })
                .map_or(0, |index| index + 1);
            Ok(self
                .candidates
                .iter()
                .skip(start)
                .take(usize::from(limit.get()))
                .cloned()
                .collect())
        }
    }

    #[derive(Debug, Default)]
    struct CountingExecutor(AtomicUsize);

    #[async_trait]
    impl ScheduledRefreshExecutor for CountingExecutor {
        async fn refresh_due(
            &self,
            _candidate: DueCredentialRefresh,
        ) -> ScheduledRefreshDisposition {
            self.0.fetch_add(1, Ordering::SeqCst);
            ScheduledRefreshDisposition::Refreshed
        }
    }

    #[derive(Debug, Default)]
    struct BlockingExecutor(AtomicUsize);

    #[async_trait]
    impl ScheduledRefreshExecutor for BlockingExecutor {
        async fn refresh_due(
            &self,
            _candidate: DueCredentialRefresh,
        ) -> ScheduledRefreshDisposition {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::future::pending().await
        }
    }

    fn candidate(offset_seconds: i64) -> DueCredentialRefresh {
        let expires_at = DateTime::<Utc>::from_timestamp(1_800_000_000 + offset_seconds, 0)
            .expect("fixture timestamp is valid");
        DueCredentialRefresh::new(
            CredentialSelector::new(
                CredentialOwner::from_canonical("scheduler-test-owner"),
                CredentialId::new(),
            ),
            "oauth2".to_owned(),
            expires_at,
        )
    }

    #[tokio::test]
    async fn tick_stops_at_page_bound_without_losing_page_work() {
        let schedule = FixedSchedule {
            candidates: vec![candidate(0), candidate(1), candidate(2)],
            scans: AtomicUsize::new(0),
        };
        let executor = Arc::new(CountingExecutor::default());
        let config = CredentialRefreshSchedulerConfig {
            page_size: CredentialRefreshPageSize::new(2).expect("fixture page size is valid"),
            max_pages_per_tick: NonZeroU16::MIN,
            ..CredentialRefreshSchedulerConfig::default()
        };

        run_tick(
            &schedule,
            executor.clone(),
            config,
            &CancellationToken::new(),
        )
        .await;

        assert_eq!(schedule.scans.load(Ordering::SeqCst), 1);
        assert_eq!(executor.0.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn cancelled_tick_never_scans_or_dispatches() {
        let schedule = FixedSchedule {
            candidates: vec![candidate(0)],
            scans: AtomicUsize::new(0),
        };
        let executor = Arc::new(CountingExecutor::default());
        let shutdown = CancellationToken::new();
        shutdown.cancel();

        run_tick(
            &schedule,
            executor.clone(),
            CredentialRefreshSchedulerConfig::default(),
            &shutdown,
        )
        .await;

        assert_eq!(schedule.scans.load(Ordering::SeqCst), 0);
        assert_eq!(executor.0.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cancellation_aborts_in_flight_refresh_work() {
        let executor = Arc::new(BlockingExecutor::default());
        let shutdown = CancellationToken::new();
        let task_shutdown = shutdown.clone();
        let task_executor = executor.clone();
        let task = tokio::spawn(async move {
            execute_page(
                vec![candidate(0)],
                task_executor,
                NonZeroU16::MIN,
                &task_shutdown,
            )
            .await;
        });
        while executor.0.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("scheduler cancellation must be bounded")
            .expect("scheduler task must join cleanly");
    }
}
