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

use super::metrics::RefreshSchedulerMetrics;

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

    pub(crate) fn cover_refresh_horizon(
        mut self,
        required: Duration,
    ) -> Result<Self, CredentialRefreshSchedulerConfigError> {
        if required > self.horizon.get() {
            self.horizon = CredentialRefreshHorizon::new(required)
                .map_err(|_| CredentialRefreshSchedulerConfigError::PolicyHorizonTooLarge)?;
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
    /// A registered credential policy cannot be represented by the storage port.
    #[error("credential refresh policy horizon exceeds the supported bound")]
    PolicyHorizonTooLarge,
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
    ReconciliationRequired,
    RetryGateFinalization,
    ReauthDecisionFinalization,
    PostProviderPersistence,
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
        metrics: RefreshSchedulerMetrics,
    ) -> Result<Self, CredentialRefreshSchedulerConfigError> {
        let config = config.validate()?;
        let shutdown = CancellationToken::new();
        let task_shutdown = shutdown.clone();
        let task = tokio::spawn(async move {
            run_loop(schedule, executor, config, metrics, task_shutdown).await;
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
    metrics: RefreshSchedulerMetrics,
    shutdown: CancellationToken,
) {
    let mut after = None;
    run_tick(
        schedule.as_ref(),
        Arc::clone(&executor),
        config,
        &metrics,
        &shutdown,
        &mut after,
    )
    .await;
    let mut ticker = tokio::time::interval(config.cadence);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ticker.tick().await;
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = ticker.tick() => {
                run_tick(
                    schedule.as_ref(),
                    Arc::clone(&executor),
                    config,
                    &metrics,
                    &shutdown,
                    &mut after,
                ).await;
            }
        }
    }
}

async fn run_tick(
    schedule: &dyn CredentialRefreshSchedule,
    executor: Arc<dyn ScheduledRefreshExecutor>,
    config: CredentialRefreshSchedulerConfig,
    metrics: &RefreshSchedulerMetrics,
    shutdown: &CancellationToken,
    after: &mut Option<CredentialRefreshCursor>,
) {
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
                metrics.cycles_scan_failed.inc();
                tracing::warn!(%error, "credential due-refresh scan failed");
                return;
            },
        };
        if candidates.is_empty() {
            *after = None;
            metrics.cycles_completed.inc();
            return;
        }
        *after = candidates.last().map(DueCredentialRefresh::cursor);
        let page_full = candidates.len() == usize::from(config.page_size.get());
        if !execute_page(
            candidates,
            executor.clone(),
            config.concurrency,
            metrics,
            shutdown,
        )
        .await
        {
            return;
        }
        if !page_full {
            *after = None;
            metrics.cycles_completed.inc();
            return;
        }
    }
    metrics.cycles_page_bound.inc();
    tracing::warn!(
        max_pages = config.max_pages_per_tick.get(),
        "credential due-refresh scan reached its per-tick page bound"
    );
}

async fn execute_page(
    candidates: Vec<DueCredentialRefresh>,
    executor: Arc<dyn ScheduledRefreshExecutor>,
    concurrency: NonZeroU16,
    metrics: &RefreshSchedulerMetrics,
    shutdown: &CancellationToken,
) -> bool {
    let mut candidates = candidates.into_iter();
    loop {
        let mut tasks = JoinSet::new();
        for candidate in candidates.by_ref().take(usize::from(concurrency.get())) {
            let executor = Arc::clone(&executor);
            tasks.spawn(async move { executor.refresh_due(candidate).await });
        }
        if tasks.is_empty() {
            return true;
        }
        loop {
            tokio::select! {
                () = shutdown.cancelled() => {
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                    return false;
                },
                result = tasks.join_next() => match result {
                    Some(Ok(disposition)) => {
                        metric_for_disposition(metrics, disposition).inc();
                        tracing::debug!(?disposition, "credential scheduled refresh completed");
                    },
                    Some(Err(error)) => {
                        metrics.candidates_task_failed.inc();
                        if !error.is_cancelled() {
                            tracing::error!(%error, "credential scheduled refresh task failed");
                        }
                    },
                    None => break,
                },
            }
        }
    }
}

fn metric_for_disposition(
    metrics: &RefreshSchedulerMetrics,
    disposition: ScheduledRefreshDisposition,
) -> &nebula_metrics::Counter {
    match disposition {
        ScheduledRefreshDisposition::Refreshed => &metrics.candidates_refreshed,
        ScheduledRefreshDisposition::NoLongerDue => &metrics.candidates_no_longer_due,
        ScheduledRefreshDisposition::Unsupported => &metrics.candidates_unsupported,
        ScheduledRefreshDisposition::Deferred => &metrics.candidates_deferred,
        ScheduledRefreshDisposition::Blocked => &metrics.candidates_blocked,
        ScheduledRefreshDisposition::ReauthRequired => &metrics.candidates_reauth_required,
        ScheduledRefreshDisposition::TransientFailure => &metrics.candidates_transient_failure,
        ScheduledRefreshDisposition::OutcomeUnknown => &metrics.candidates_outcome_unknown,
        ScheduledRefreshDisposition::ReconciliationRequired => {
            &metrics.candidates_reconciliation_required
        },
        ScheduledRefreshDisposition::RetryGateFinalization => {
            &metrics.candidates_retry_gate_finalization
        },
        ScheduledRefreshDisposition::ReauthDecisionFinalization => {
            &metrics.candidates_reauth_decision_finalization
        },
        ScheduledRefreshDisposition::PostProviderPersistence => {
            &metrics.candidates_post_provider_persistence
        },
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::atomic::{AtomicUsize, Ordering},
    };

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

    #[derive(Debug)]
    struct FailingSchedule;

    #[async_trait]
    impl CredentialRefreshSchedule for FailingSchedule {
        async fn scan_due(
            &self,
            _after: Option<&CredentialRefreshCursor>,
            _horizon: CredentialRefreshHorizon,
            _limit: CredentialRefreshPageSize,
        ) -> Result<Vec<DueCredentialRefresh>, CredentialRefreshScheduleError> {
            Err(CredentialRefreshScheduleError::CorruptRecord)
        }
    }

    #[derive(Debug)]
    struct ScriptedExecutor(std::sync::Mutex<VecDeque<ScheduledRefreshDisposition>>);

    #[async_trait]
    impl ScheduledRefreshExecutor for ScriptedExecutor {
        async fn refresh_due(
            &self,
            _candidate: DueCredentialRefresh,
        ) -> ScheduledRefreshDisposition {
            self.0
                .lock()
                .expect("script mutex is not poisoned")
                .pop_front()
                .expect("one disposition exists per candidate")
        }
    }

    #[derive(Debug)]
    struct PanickingExecutor;

    #[async_trait]
    impl ScheduledRefreshExecutor for PanickingExecutor {
        async fn refresh_due(
            &self,
            _candidate: DueCredentialRefresh,
        ) -> ScheduledRefreshDisposition {
            panic!("scripted candidate task failure")
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
            expires_at,
        )
    }

    #[test]
    fn scheduler_horizon_covers_the_largest_registered_policy() {
        let config = CredentialRefreshSchedulerConfig::default()
            .cover_refresh_horizon(Duration::from_mins(12))
            .expect("fixture policy horizon is bounded");

        assert_eq!(config.horizon.get(), Duration::from_mins(12));
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
        let mut after = None;
        let metrics = RefreshSchedulerMetrics::for_tests().expect("test metrics are valid");

        run_tick(
            &schedule,
            executor.clone(),
            config,
            &metrics,
            &CancellationToken::new(),
            &mut after,
        )
        .await;

        assert_eq!(schedule.scans.load(Ordering::SeqCst), 1);
        assert_eq!(executor.0.load(Ordering::SeqCst), 2);
        assert!(after.is_some());
        assert_eq!(metrics.cycles_page_bound.get(), 1);
        assert_eq!(metrics.candidates_refreshed.get(), 2);
    }

    #[tokio::test]
    async fn consecutive_ticks_continue_after_the_page_bound() {
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
        let shutdown = CancellationToken::new();
        let mut after = None;
        let metrics = RefreshSchedulerMetrics::for_tests().expect("test metrics are valid");

        run_tick(
            &schedule,
            executor.clone(),
            config,
            &metrics,
            &shutdown,
            &mut after,
        )
        .await;
        run_tick(
            &schedule,
            executor.clone(),
            config,
            &metrics,
            &shutdown,
            &mut after,
        )
        .await;

        assert_eq!(executor.0.load(Ordering::SeqCst), 3);
        assert!(after.is_none());
        assert_eq!(metrics.cycles_page_bound.get(), 1);
        assert_eq!(metrics.cycles_completed.get(), 1);
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
        let mut after = None;
        let metrics = RefreshSchedulerMetrics::for_tests().expect("test metrics are valid");

        run_tick(
            &schedule,
            executor.clone(),
            CredentialRefreshSchedulerConfig::default(),
            &metrics,
            &shutdown,
            &mut after,
        )
        .await;

        assert_eq!(schedule.scans.load(Ordering::SeqCst), 0);
        assert_eq!(executor.0.load(Ordering::SeqCst), 0);
        assert_eq!(metrics.cycles_completed.get(), 0);
        assert_eq!(metrics.cycles_scan_failed.get(), 0);
        assert_eq!(metrics.cycles_page_bound.get(), 0);
    }

    #[tokio::test]
    async fn scan_failure_is_counted_once_without_candidate_work() {
        let metrics = RefreshSchedulerMetrics::for_tests().expect("test metrics are valid");
        let mut after = None;
        run_tick(
            &FailingSchedule,
            Arc::new(CountingExecutor::default()),
            CredentialRefreshSchedulerConfig::default(),
            &metrics,
            &CancellationToken::new(),
            &mut after,
        )
        .await;

        assert_eq!(metrics.cycles_scan_failed.get(), 1);
        assert_eq!(metrics.cycles_completed.get(), 0);
        assert_eq!(metrics.candidates_refreshed.get(), 0);
    }

    #[tokio::test]
    async fn every_candidate_disposition_has_one_prebound_counter() {
        let dispositions = VecDeque::from([
            ScheduledRefreshDisposition::Refreshed,
            ScheduledRefreshDisposition::NoLongerDue,
            ScheduledRefreshDisposition::Unsupported,
            ScheduledRefreshDisposition::Deferred,
            ScheduledRefreshDisposition::Blocked,
            ScheduledRefreshDisposition::ReauthRequired,
            ScheduledRefreshDisposition::TransientFailure,
            ScheduledRefreshDisposition::OutcomeUnknown,
            ScheduledRefreshDisposition::ReconciliationRequired,
            ScheduledRefreshDisposition::RetryGateFinalization,
            ScheduledRefreshDisposition::ReauthDecisionFinalization,
            ScheduledRefreshDisposition::PostProviderPersistence,
        ]);
        let candidates = (0..dispositions.len())
            .map(|offset| candidate(offset as i64))
            .collect();
        let metrics = RefreshSchedulerMetrics::for_tests().expect("test metrics are valid");

        assert!(
            execute_page(
                candidates,
                Arc::new(ScriptedExecutor(std::sync::Mutex::new(dispositions))),
                NonZeroU16::MIN,
                &metrics,
                &CancellationToken::new(),
            )
            .await
        );

        assert_eq!(metrics.candidates_refreshed.get(), 1);
        assert_eq!(metrics.candidates_no_longer_due.get(), 1);
        assert_eq!(metrics.candidates_unsupported.get(), 1);
        assert_eq!(metrics.candidates_deferred.get(), 1);
        assert_eq!(metrics.candidates_blocked.get(), 1);
        assert_eq!(metrics.candidates_reauth_required.get(), 1);
        assert_eq!(metrics.candidates_transient_failure.get(), 1);
        assert_eq!(metrics.candidates_outcome_unknown.get(), 1);
        assert_eq!(metrics.candidates_reconciliation_required.get(), 1);
        assert_eq!(metrics.candidates_retry_gate_finalization.get(), 1);
        assert_eq!(metrics.candidates_reauth_decision_finalization.get(), 1);
        assert_eq!(metrics.candidates_post_provider_persistence.get(), 1);
        assert_eq!(metrics.candidates_task_failed.get(), 0);
    }

    #[tokio::test]
    async fn failed_candidate_task_is_counted_without_a_disposition() {
        let metrics = RefreshSchedulerMetrics::for_tests().expect("test metrics are valid");

        assert!(
            execute_page(
                vec![candidate(0)],
                Arc::new(PanickingExecutor),
                NonZeroU16::MIN,
                &metrics,
                &CancellationToken::new(),
            )
            .await
        );

        assert_eq!(metrics.candidates_task_failed.get(), 1);
        assert_eq!(metrics.candidates_refreshed.get(), 0);
    }

    #[tokio::test]
    async fn cancellation_aborts_in_flight_refresh_work() {
        let executor = Arc::new(BlockingExecutor::default());
        let shutdown = CancellationToken::new();
        let task_shutdown = shutdown.clone();
        let task_executor = executor.clone();
        let metrics = RefreshSchedulerMetrics::for_tests().expect("test metrics are valid");
        let task_metrics = metrics.clone();
        let task = tokio::spawn(async move {
            execute_page(
                vec![candidate(0)],
                task_executor,
                NonZeroU16::MIN,
                &task_metrics,
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
        assert_eq!(metrics.candidates_refreshed.get(), 0);
        assert_eq!(metrics.candidates_task_failed.get(), 0);
    }
}
