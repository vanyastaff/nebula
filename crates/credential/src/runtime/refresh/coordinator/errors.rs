use crate::RefreshNotAppliedContext;

use nebula_storage_port::store::RefreshClaimError as RepoError;

// ──────────────────────────────────────────────────────────────────────────
// Errors surfaced from refresh_coalesced
// ──────────────────────────────────────────────────────────────────────────

/// Failures returned by [`super::RefreshCoordinator::refresh_coalesced`].
///
/// `CoalescedByOtherReplica` is **success** for the caller (state was
/// already fresh after another replica's refresh; just re-read state). All
/// other variants are real errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RefreshError {
    /// The caller's configured contention budget elapsed before an L2 claim
    /// could be acquired.
    ///
    /// This remains pre-provider and replay-safe. It is surfaced when the
    /// contender's claim keeps being heartbeat-extended while adaptive polls
    /// consume `refresh_timeout`.
    #[error("contention budget exhausted before claim acquisition")]
    ContentionExhausted,
    /// Another replica's refresh succeeded while we were waiting on L2;
    /// caller treats as success and re-reads state.
    #[error("refresh coalesced by another replica (success \u{2014} re-read state)")]
    CoalescedByOtherReplica,
    /// Storage repo error (e.g. DB connectivity loss).
    #[error("storage repo error: {0}")]
    Repo(#[from] RepoError),
    /// Background heartbeat ownership was lost before the provider boundary.
    ///
    /// The provider closure was never started. Once the sentinel transition
    /// confirms entry into the provider/persistence critical section,
    /// heartbeat loss can no longer cancel that section.
    #[error("L2 claim lost before provider dispatch \u{2014} refresh was not started")]
    ClaimLostBeforeProvider,
    /// The caller stopped waiting after the provider boundary, or the owned
    /// task terminated without returning an exact disposition.
    ///
    /// This is deliberately non-retryable at the resolver boundary. The owned
    /// task continues after an ordinary timeout. Panic/runtime cancellation
    /// retains the sentinel claim; once its lease expires, storage exposes it
    /// as durable fail-closed poison until explicit reconciliation.
    #[error("provider/persistence refresh outcome is pending or unknown; do not retry")]
    CriticalOutcomePending,
    /// Another in-process attempt reached an exact finalization failure that
    /// cannot safely be replayed.
    ///
    /// The winner retained the durable claim as poison. Unlike
    /// [`Self::CriticalOutcomePending`], the operation outcome is known; the
    /// command owner must surface its operation-specific reconciliation
    /// contract.
    #[error("a concurrent refresh operation requires reconciliation before retrying")]
    ReconciliationRequired,
    /// Another in-process attempt reached an exact, replay-safe outcome but
    /// did not advance authoritative state.
    ///
    /// No provider or persistence operation remains pending, but the
    /// coordinator deliberately does not turn every waiter into an immediate
    /// retry. The caller may retry later under its normal backoff and circuit
    /// policy.
    #[error("a concurrent refresh attempt completed without advancing credential state")]
    PriorAttemptNoProgress,
    /// A backend-authoritative retry gate forbids provider dispatch for the
    /// current credential epoch.
    ///
    /// The caller must re-read the typed gate evidence rather than treating
    /// this as successful coalescing or a generic retryable failure.
    #[error("credential refresh retry is suppressed by durable aggregate state: {0}")]
    RetrySuppressed(Box<RefreshNotAppliedContext>),
    /// The authoritative credential state could not be rechecked after
    /// contention, so provider dispatch was denied.
    ///
    /// This failure occurs before the provider boundary and is therefore safe
    /// for the command owner to retry after the state source recovers.
    #[error(transparent)]
    StateRecheck(#[from] RefreshRecheckError),
    /// A different operation, or an unclassified legacy incident, retains
    /// durable authority and cannot be resolved as a refresh.
    #[error("credential operation requires reconciliation before use")]
    OperationBlocked {
        /// Kind read from the retained durable claim.
        operation: nebula_storage_port::store::CredentialOperationKind,
    },
}

/// Closed failure taxonomy for the pre-provider state recheck.
///
/// A recheck is mandatory after L1/L2 contention because the refresh closure
/// captures state loaded before waiting. Neither storage failure nor corrupt
/// state may be flattened into `true`: doing so would authorize provider
/// egress with a stale rotating grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RefreshRecheckError {
    /// The authoritative state source could not be read.
    #[error("credential state recheck is unavailable")]
    Unavailable,
    /// The current persisted state could not be validated for the operation.
    #[error("credential state recheck found invalid state")]
    InvalidState,
}

/// Authoritative result of rechecking a refresh contender's captured epoch.
#[derive(Debug)]
#[non_exhaustive]
pub enum RefreshRecheck {
    /// The same credential epoch still needs provider/local refresh work.
    Needed,
    /// Authoritative state advanced or no longer needs this operation.
    Satisfied,
    /// A durable retry gate forbids dispatch for the current epoch.
    Suppressed(Box<RefreshNotAppliedContext>),
}

/// Exact disposition of an owned provider/persistence refresh section.
///
/// The coordinator needs this distinction to finalize the durable L2 claim
/// safely and tell L1 waiters what the completion proves. A durable state
/// advance may release immediately and permits a later refresh epoch. An exact
/// replay-safe outcome without a state advance also releases, but waiters do
/// not automatically retry it. An exact finalization failure after either a
/// provider or local refresh, or an unknown provider/commit outcome, stops
/// heartbeats but deliberately leaves the sentinel claim in place. Once its
/// lease expires, storage keeps it as durable fail-closed poison so refresh
/// work cannot replay before explicit reconciliation.
#[derive(Debug)]
#[must_use = "the refresh disposition controls whether the durable claim may be released"]
#[non_exhaustive]
pub enum RefreshDisposition<T> {
    /// The operation durably advanced the authoritative state consulted by
    /// `needs_refresh_after_backoff`.
    ///
    /// A waiter still observing work after this completion is handling a later
    /// logical epoch and may enter a new winner election. Callers must use this
    /// variant only after an acknowledged state transition (including a
    /// durable `reauth_required` transition).
    StateAdvanced(T),
    /// The operation reached an exact, replay-safe outcome without advancing
    /// authoritative state.
    ///
    /// L2 is released, but L1 waiters receive
    /// [`RefreshError::PriorAttemptNoProgress`] instead of immediately
    /// replaying the operation as a herd.
    NoStateChange(T),
    /// Refresh work completed, but its new state was definitely not persisted.
    ///
    /// The enclosed error is exact, yet another replica must not immediately
    /// repeat the refresh against stale authoritative state. Like an unknown
    /// acknowledgement, this retains the sentinel claim. Expiry converts it
    /// into durable poison, which only an owner-qualified reconciliation
    /// command may clear.
    RetryUnsafe(T),
    /// Provider dispatch or persistence commit completed without an exact
    /// acknowledgement.
    ///
    /// The enclosed value is still returned to the waiting caller (normally a
    /// typed `OutcomeUnknown` error), while the claim remains retained as
    /// durable poison after expiry.
    OutcomeUnknown(T),
}

impl<T> RefreshDisposition<T> {
    /// Construct a disposition backed by an acknowledged authoritative state
    /// transition.
    pub fn state_advanced(value: T) -> Self {
        Self::StateAdvanced(value)
    }

    /// Construct an exact, replay-safe disposition that did not change
    /// authoritative state.
    pub fn no_state_change(value: T) -> Self {
        Self::NoStateChange(value)
    }

    /// Construct a definite finalization failure that is unsafe to replay.
    pub fn retry_unsafe(value: T) -> Self {
        Self::RetryUnsafe(value)
    }

    /// Construct an unknown provider-or-commit disposition.
    pub fn outcome_unknown(value: T) -> Self {
        Self::OutcomeUnknown(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ClaimFinalization {
    Release,
    RetainAsPoison,
}
