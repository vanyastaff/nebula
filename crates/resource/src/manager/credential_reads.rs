//! Join-next coalescing of credential availability reads.
//!
//! A strict manager reads a bound credential's availability before admitting
//! each new unit of work (Design CONTRACT: every new credentialed unit reads
//! availability first). Many acquires of the same credential arrive at once,
//! so reads are coalesced per **credential lane** — one lane per
//! `(credential id, owner scope, contract key)` — with the **join-next** rule
//! (review ADM-D8):
//!
//! - A caller may only take the result of a read *issued at or after it
//!   arrived*. Joining a read already in flight when it arrived is forbidden:
//!   that read may have started before a block committed, and the caller
//!   would admit on a stale answer. There is no freshness window.
//! - On arrival a caller needs read number `issued + 1`. If no read is in
//!   flight it issues that read itself (it becomes the lane's **leader**);
//!   otherwise it waits for the read in flight to finish and then either
//!   takes the next read or issues it. At most one read per lane is in flight
//!   (depth 1), so a burst of `n` callers during one read costs two reads.
//! - A leader that is dropped (its acquire was cancelled) or times out
//!   publishes nothing; a waiter wakes and re-elects itself.
//!
//! No task is spawned per read: the leader's own future performs it, so
//! dropping an acquire drops its read. Lanes exist only while a caller uses
//! them.
//!
//! The lane map's mutex is taken before a lane's state mutex, and neither is
//! held across an await.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use nebula_credential::{
    CredentialAvailability, CredentialAvailabilityObservation, CredentialAvailabilityObserver,
    CredentialId, CredentialKey, CredentialObserveError, TenantScope,
};
use nebula_metrics::{
    Counter, Histogram, LabelSet, MetricsRegistry, MetricsResult,
    naming::{
        NEBULA_RESOURCE_CREDENTIAL_ADMISSION_DENIED_TOTAL,
        NEBULA_RESOURCE_CREDENTIAL_ADMISSION_JOINED_TOTAL,
        NEBULA_RESOURCE_CREDENTIAL_ADMISSION_READ_DURATION_SECONDS,
        NEBULA_RESOURCE_CREDENTIAL_ADMISSION_READS_TOTAL, credential_admission_denied_reason,
        credential_admission_read_outcome,
    },
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::error::CredentialUnavailableReason;

/// Upper bound of one credential availability read. A read that does not
/// answer in time refuses the unit as
/// [`CheckUnavailable`](CredentialUnavailableReason::CheckUnavailable).
pub(crate) const CREDENTIAL_READ_TIMEOUT: Duration = Duration::from_secs(2);

/// Why a read produced no observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadFailure {
    /// The observer answered with an error.
    Observe(CredentialObserveError),
    /// No read issued after the caller arrived answered before its deadline.
    TimedOut,
    /// The manager is shutting down.
    Cancelled,
}

impl From<CredentialObserveError> for ReadFailure {
    fn from(error: CredentialObserveError) -> Self {
        match error {
            CredentialObserveError::Cancelled => Self::Cancelled,
            error => Self::Observe(error),
        }
    }
}

/// One slot check's answer.
pub(crate) type ReadResult = Result<CredentialAvailabilityObservation, ReadFailure>;

/// The answer an observer call produced, as published to a lane.
pub(crate) type Published = Result<CredentialAvailabilityObservation, CredentialObserveError>;

/// Per-lane state. `issued` counts reads started on the lane, `latest` is the
/// last read that answered, tagged with its number.
#[derive(Debug, Default)]
struct LaneState {
    issued: u64,
    in_flight: bool,
    latest: Option<(u64, Published)>,
    /// Callers inside [`CredentialReads::read_after_arrival`] on this lane;
    /// changed only under the lane map's mutex.
    users: usize,
}

#[derive(Debug)]
struct Lane {
    scope: TenantScope,
    key: CredentialKey,
    state: Mutex<LaneState>,
    /// Woken when a read on the lane ends (answered or abandoned).
    ended: Notify,
}

impl Lane {
    fn state(&self) -> MutexGuard<'_, LaneState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn matches(&self, scope: &TenantScope, key: &CredentialKey) -> bool {
        &self.scope == scope && &self.key == key
    }
}

/// Registry-bound series of the strict read.
#[derive(Debug, Clone)]
pub(crate) struct CredentialAdmissionMetrics {
    available: Counter,
    refresh_in_flight: Counter,
    blocked: Counter,
    absent: Counter,
    unavailable: Counter,
    timed_out: Counter,
    cancelled: Counter,
    joined: Counter,
    read_seconds: Histogram,
    denied: [Counter; 6],
}

impl CredentialAdmissionMetrics {
    pub(crate) fn new(registry: &MetricsRegistry) -> MetricsResult<Self> {
        let read = |outcome: &str| {
            registry.counter_labeled(
                NEBULA_RESOURCE_CREDENTIAL_ADMISSION_READS_TOTAL,
                &registry.interner().single("outcome", outcome),
            )
        };
        let denied = |reason: CredentialUnavailableReason| {
            registry.counter_labeled(
                NEBULA_RESOURCE_CREDENTIAL_ADMISSION_DENIED_TOTAL,
                &registry
                    .interner()
                    .single("reason", denied_reason_label(reason)),
            )
        };
        Ok(Self {
            available: read(credential_admission_read_outcome::AVAILABLE)?,
            refresh_in_flight: read(credential_admission_read_outcome::REFRESH_IN_FLIGHT)?,
            blocked: read(credential_admission_read_outcome::BLOCKED)?,
            absent: read(credential_admission_read_outcome::ABSENT)?,
            unavailable: read(credential_admission_read_outcome::UNAVAILABLE)?,
            timed_out: read(credential_admission_read_outcome::TIMED_OUT)?,
            cancelled: read(credential_admission_read_outcome::CANCELLED)?,
            joined: registry.counter(NEBULA_RESOURCE_CREDENTIAL_ADMISSION_JOINED_TOTAL)?,
            read_seconds: registry.histogram_with_buckets_labeled(
                NEBULA_RESOURCE_CREDENTIAL_ADMISSION_READ_DURATION_SECONDS,
                &LabelSet::empty(),
                vec![
                    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0,
                ],
            )?,
            denied: [
                denied(CredentialUnavailableReason::ReauthRequired)?,
                denied(CredentialUnavailableReason::OperationBlocked)?,
                denied(CredentialUnavailableReason::RefreshInFlight)?,
                denied(CredentialUnavailableReason::Rebinding)?,
                denied(CredentialUnavailableReason::CheckUnavailable)?,
                denied(CredentialUnavailableReason::Absent)?,
            ],
        })
    }

    fn record_read(&self, outcome: Option<&Published>, elapsed: Duration) {
        self.read_seconds.observe(elapsed.as_secs_f64());
        let counter = match outcome {
            None => &self.timed_out,
            Some(Ok(observation)) => match observation.availability() {
                CredentialAvailability::Available => &self.available,
                CredentialAvailability::RefreshInFlight => &self.refresh_in_flight,
                _ => &self.blocked,
            },
            Some(Err(
                CredentialObserveError::Absent | CredentialObserveError::WrongCredentialKey,
            )) => &self.absent,
            // Shutdown ended the read: not a store outage.
            Some(Err(CredentialObserveError::Cancelled)) => &self.cancelled,
            Some(Err(_)) => &self.unavailable,
        };
        counter.inc();
    }

    /// Counts one refused unit of work.
    pub(crate) fn record_denied(&self, reason: CredentialUnavailableReason) {
        let index = match reason {
            CredentialUnavailableReason::ReauthRequired => 0,
            CredentialUnavailableReason::OperationBlocked => 1,
            CredentialUnavailableReason::RefreshInFlight => 2,
            CredentialUnavailableReason::Rebinding => 3,
            CredentialUnavailableReason::CheckUnavailable => 4,
            CredentialUnavailableReason::Absent => 5,
        };
        self.denied[index].inc();
    }

    #[cfg(test)]
    pub(crate) fn reads(&self) -> [u64; 7] {
        [
            self.available.get(),
            self.refresh_in_flight.get(),
            self.blocked.get(),
            self.absent.get(),
            self.unavailable.get(),
            self.timed_out.get(),
            self.cancelled.get(),
        ]
    }

    #[cfg(test)]
    pub(crate) fn joined(&self) -> u64 {
        self.joined.get()
    }

    #[cfg(test)]
    pub(crate) fn denied(&self, reason: CredentialUnavailableReason) -> u64 {
        let reasons = [
            CredentialUnavailableReason::ReauthRequired,
            CredentialUnavailableReason::OperationBlocked,
            CredentialUnavailableReason::RefreshInFlight,
            CredentialUnavailableReason::Rebinding,
            CredentialUnavailableReason::CheckUnavailable,
            CredentialUnavailableReason::Absent,
        ];
        reasons
            .iter()
            .position(|candidate| *candidate == reason)
            .map_or(0, |index| self.denied[index].get())
    }
}

/// The stable `reason` label of a refusal.
fn denied_reason_label(reason: CredentialUnavailableReason) -> &'static str {
    match reason {
        CredentialUnavailableReason::ReauthRequired => {
            credential_admission_denied_reason::REAUTH_REQUIRED
        },
        CredentialUnavailableReason::OperationBlocked => {
            credential_admission_denied_reason::OPERATION_BLOCKED
        },
        CredentialUnavailableReason::RefreshInFlight => {
            credential_admission_denied_reason::REFRESH_IN_FLIGHT
        },
        CredentialUnavailableReason::Rebinding => credential_admission_denied_reason::REBINDING,
        CredentialUnavailableReason::CheckUnavailable => {
            credential_admission_denied_reason::CHECK_UNAVAILABLE
        },
        CredentialUnavailableReason::Absent => credential_admission_denied_reason::ABSENT,
    }
}

/// The manager's coalescer of credential availability reads; see the module
/// docs.
pub(crate) struct CredentialReads {
    observer: Arc<dyn CredentialAvailabilityObserver>,
    lanes: Mutex<HashMap<CredentialId, Vec<Arc<Lane>>>>,
    /// The manager's cancellation: shutdown ends every read.
    cancel: CancellationToken,
    metrics: Option<CredentialAdmissionMetrics>,
}

impl std::fmt::Debug for CredentialReads {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialReads")
            .field("lanes", &self.lanes().len())
            .finish_non_exhaustive()
    }
}

/// What a caller does after looking at its lane.
enum Step {
    Answered(Published),
    Lead(u64),
    Wait,
}

impl CredentialReads {
    pub(crate) fn new(
        observer: Arc<dyn CredentialAvailabilityObserver>,
        cancel: CancellationToken,
        metrics: Option<CredentialAdmissionMetrics>,
    ) -> Self {
        Self {
            observer,
            lanes: Mutex::new(HashMap::new()),
            cancel,
            metrics,
        }
    }

    pub(crate) fn metrics(&self) -> Option<&CredentialAdmissionMetrics> {
        self.metrics.as_ref()
    }

    fn lanes(&self) -> MutexGuard<'_, HashMap<CredentialId, Vec<Arc<Lane>>>> {
        self.lanes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads the availability of `credential_id` for `scope` under contract
    /// `key`, answering only with a read issued at or after this call began,
    /// before `deadline`. See the module docs.
    ///
    /// Cancel safe: dropping the future drops the read it leads, and a
    /// waiter re-elects itself.
    pub(crate) async fn read_after_arrival(
        &self,
        scope: &TenantScope,
        credential_id: CredentialId,
        key: &CredentialKey,
        deadline: tokio::time::Instant,
    ) -> ReadResult {
        let user = LaneUser::enter(self, scope, credential_id, key);
        let lane = &user.lane;
        let need = lane.state().issued + 1;
        loop {
            let ended = lane.ended.notified();
            tokio::pin!(ended);
            let step = {
                let mut state = lane.state();
                match &state.latest {
                    Some((seq, published)) if *seq >= need => Step::Answered(*published),
                    _ if !state.in_flight => {
                        state.in_flight = true;
                        state.issued += 1;
                        Step::Lead(state.issued)
                    },
                    _ => {
                        // Registered while the state is locked: the leader
                        // publishes under the same lock, so its wake-up
                        // cannot be missed.
                        ended.as_mut().enable();
                        Step::Wait
                    },
                }
            };
            match step {
                Step::Answered(published) => {
                    // Only a caller that did not lead is ever answered here.
                    if let Some(metrics) = &self.metrics {
                        metrics.joined.inc();
                    }
                    return published.map_err(ReadFailure::from);
                },
                Step::Lead(seq) => {
                    match self.lead(lane, seq, credential_id, deadline).await {
                        Some(published) => return published.map_err(ReadFailure::from),
                        // Timed out: nothing was published.
                        None => return Err(ReadFailure::TimedOut),
                    }
                },
                Step::Wait => {
                    tokio::select! {
                        biased;
                        () = self.cancel.cancelled() => return Err(ReadFailure::Cancelled),
                        () = &mut ended => {},
                        () = tokio::time::sleep_until(deadline) => return Err(ReadFailure::TimedOut),
                    }
                },
            }
        }
    }

    /// Performs read `seq` of `lane` and publishes its answer; `None` when it
    /// timed out. A cancelled manager publishes `Cancelled` so every waiter
    /// ends too.
    async fn lead(
        &self,
        lane: &Arc<Lane>,
        seq: u64,
        credential_id: CredentialId,
        deadline: tokio::time::Instant,
    ) -> Option<Published> {
        let lead = Leadership {
            lane,
            published: false,
        };
        let started = tokio::time::Instant::now();
        let read = self.observer.observe_availability(
            &lane.scope,
            credential_id,
            lane.key.clone(),
            // The observer only listens; a clone avoids registering a child
            // token on the manager's token for every read.
            self.cancel.clone(),
        );
        let answered = tokio::select! {
            biased;
            () = self.cancel.cancelled() => Some(Err(CredentialObserveError::Cancelled)),
            answered = tokio::time::timeout_at(deadline, read) => answered.ok(),
        };
        if let Some(metrics) = &self.metrics {
            metrics.record_read(answered.as_ref(), started.elapsed());
        }
        if let Some(published) = answered {
            lead.publish(seq, published);
        }
        answered
    }

    /// Number of live lanes (tests).
    #[cfg(test)]
    pub(crate) fn lane_count(&self) -> usize {
        self.lanes().values().map(Vec::len).sum()
    }

    /// Callers currently on the lanes of `credential_id` (tests).
    #[cfg(test)]
    pub(crate) fn users(&self, credential_id: CredentialId) -> usize {
        self.lanes()
            .get(&credential_id)
            .map_or(0, |lanes| lanes.iter().map(|lane| lane.state().users).sum())
    }
}

/// A caller's presence on a lane; the last one out removes the lane.
struct LaneUser<'a> {
    reads: &'a CredentialReads,
    credential_id: CredentialId,
    lane: Arc<Lane>,
}

impl<'a> LaneUser<'a> {
    fn enter(
        reads: &'a CredentialReads,
        scope: &TenantScope,
        credential_id: CredentialId,
        key: &CredentialKey,
    ) -> Self {
        let mut lanes = reads.lanes();
        let entries = lanes.entry(credential_id).or_default();
        let lane = if let Some(lane) = entries.iter().find(|lane| lane.matches(scope, key)) {
            Arc::clone(lane)
        } else {
            let lane = Arc::new(Lane {
                scope: scope.clone(),
                key: key.clone(),
                state: Mutex::new(LaneState::default()),
                ended: Notify::new(),
            });
            entries.push(Arc::clone(&lane));
            lane
        };
        lane.state().users += 1;
        Self {
            reads,
            credential_id,
            lane,
        }
    }
}

impl Drop for LaneUser<'_> {
    fn drop(&mut self) {
        let mut lanes = self.reads.lanes();
        let mut state = self.lane.state();
        state.users = state.users.saturating_sub(1);
        if state.users != 0 {
            return;
        }
        drop(state);
        if let Some(entries) = lanes.get_mut(&self.credential_id) {
            entries.retain(|lane| !Arc::ptr_eq(lane, &self.lane));
            if entries.is_empty() {
                lanes.remove(&self.credential_id);
            }
        }
    }
}

/// The leader's hold on its lane's single in-flight read: released on
/// publish, and on drop when the read was abandoned (cancelled acquire or
/// timeout), so a waiter can issue the next read.
struct Leadership<'a> {
    lane: &'a Arc<Lane>,
    published: bool,
}

impl Leadership<'_> {
    fn publish(mut self, seq: u64, published: Published) {
        let mut state = self.lane.state();
        state.latest = Some((seq, published));
        state.in_flight = false;
        drop(state);
        self.published = true;
        self.lane.ended.notify_waiters();
    }
}

impl Drop for Leadership<'_> {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        self.lane.state().in_flight = false;
        self.lane.ended.notify_waiters();
    }
}

#[cfg(test)]
#[path = "credential_reads_tests.rs"]
pub(crate) mod tests;
