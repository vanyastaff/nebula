//! The lease-owning facade ([`Managed`]), its units ([`Unit`]) and the
//! per-unit runtime: permit, spawn, deadline, attempt admission and the
//! settled outcome.

use std::{
    any::type_name,
    fmt,
    future::Future,
    num::NonZeroU32,
    panic::AssertUnwindSafe,
    pin::Pin,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering},
    },
    task::{Context, Poll},
    time::Instant,
};

use futures::FutureExt as _;
use nebula_core::ResourceKey;
use nebula_eventbus::EventBus;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::{
    Operation,
    cost::{Cost, SentState},
    error::OpError,
    pin::PinSlots,
    strict::{UnitPin, capture_pin},
};
use crate::{
    error::{Error, ErrorKind},
    events::ResourceEvent,
    guard::{LeaseClosing, ResourceGuard},
    metrics::ResourceOpsMetrics,
    rate_limit::{DEFAULT_MAX_PENALTY, Verdict},
    resource::Provider,
    runtime::{
        admission::{AdmissionGeneration, CloseCause},
        managed::ManagedResource,
    },
    topology_tag::TopologyTag,
};

/// Host cap on a unit's deadline, and the deadline a unit gets unless
/// [`Unit::with_deadline`] shortens it. Interim: equal to
/// [`DEFAULT_MAX_PENALTY`] until the package fixes a host budget.
pub const UNIT_DEADLINE_CAP: std::time::Duration = DEFAULT_MAX_PENALTY;

/// Units that may run at once on one lease whose topology checks out an
/// instance exclusively ([`Pooled`](crate::Pooled), [`Bounded`](crate::Bounded)).
/// Interim: per-unit checkout replaces the lease-wide cap later.
const EXCLUSIVE_UNIT_CAP: usize = 1;

/// Units that may run at once on one lease of a shared instance
/// ([`Resident`](crate::Resident), custom topologies). Interim.
const SHARED_UNIT_CAP: usize = 64;

const PENDING: u8 = 0;
const GRANTED: u8 = 1;
const CANCELLED: u8 = 2;

/// The lease a [`Managed`] facade owns, shared by every unit it started.
///
/// Dropping the last reference drops the guard, which releases the lease;
/// an attempt that [`taint`](Attempt::taint)ed it is re-applied first, so
/// the release bypasses recycling.
struct ManagedLease<R: Provider> {
    guard: ResourceGuard<R>,
    managed: Arc<ManagedResource<R>>,
    generation: Arc<AdmissionGeneration>,
    key: ResourceKey,
    metrics: Option<ResourceOpsMetrics>,
    events: Option<Arc<EventBus<ResourceEvent>>>,
    units: Arc<Semaphore>,
    tainted: AtomicBool,
}

impl<R: Provider> ManagedLease<R> {
    /// Refuses a new attempt once the lease's admission generation closed:
    /// `Revoked` for a tainted row, `CredentialUnavailable` when a credential
    /// suspension closed it, `Cancelled` otherwise (removal, shutdown).
    ///
    /// The same mapping as the acquire path's hand-out refusal, applied to
    /// the lease's own generation rather than the row's current one.
    fn admission_refusal(&self) -> Result<(), OpError> {
        if self.managed.is_tainted() {
            return Err(OpError::new(
                ErrorKind::Revoked,
                "resource tainted by a credential revoke; new attempts refused",
            ));
        }
        if !self.generation.is_closed() {
            return Ok(());
        }
        Err(match self.generation.close_cause() {
            Some(CloseCause::Credential(reason)) => OpError::new(
                ErrorKind::CredentialUnavailable { reason },
                "bound credential unavailable; new attempts refused",
            ),
            None => OpError::new(ErrorKind::Cancelled, "lease closing; new attempts refused"),
        })
    }

    fn record_attempt(&self, granted: bool) {
        if let Some(metrics) = &self.metrics {
            metrics.record_call_attempt(granted);
        }
    }

    /// Records a settled unit: span fields, counters, and the
    /// outcome-unknown event.
    fn record_settled<T>(
        &self,
        span: &tracing::Span,
        result: &Result<T, OpError>,
        sent: SentState,
        attempts: u32,
    ) {
        let outcome = match result {
            Ok(_) => "ok",
            Err(error) if error.is_outcome_unknown() => "outcome_unknown",
            Err(_) => "error",
        };
        span.record("attempts", attempts);
        span.record("sent", sent.as_str());
        span.record("outcome", outcome);
        if let Some(metrics) = &self.metrics {
            metrics.record_call_unit(sent);
        }
        if let Err(error) = result {
            if error.is_outcome_unknown() {
                tracing::warn!(
                    parent: span,
                    resource.key = %self.key,
                    kind = %error.kind(),
                    sent = sent.as_str(),
                    "managed unit failed with an unknown outcome; reconcile before retrying"
                );
                if let Some(events) = &self.events {
                    let _ = events.emit(ResourceEvent::UnitOutcomeUnknown {
                        key: self.key.clone(),
                    });
                }
            } else {
                tracing::debug!(
                    parent: span,
                    kind = %error.kind(),
                    sent = sent.as_str(),
                    "managed unit failed"
                );
            }
        }
    }
}

impl<R: Provider> Drop for ManagedLease<R> {
    fn drop(&mut self) {
        if *self.tainted.get_mut() {
            self.guard.taint();
        }
    }
}

/// A lease turned into a managed call facade.
///
/// Built by [`ResourceGuard::into_managed`]. Provider calls go through
/// [`submit`](Self::submit), one [`Operation`] per [`Unit`]; each attempt of
/// a unit is admitted against the lease, booked on the row's rate limit and
/// settled. There is deliberately no `Deref` to the instance: a call cannot
/// skip admission and the limit by accident, and the instance is reached
/// only inside a granted [`Attempt`].
///
/// ```compile_fail
/// use nebula_resource::{PinSlots, Provider, call::Managed};
///
/// fn skip_the_facade<R: Provider + PinSlots>(managed: &Managed<R>) -> &R::Instance {
///     &**managed
/// }
/// ```
///
/// Cloning shares the lease: the lease is released when the last clone and
/// the last unit any clone started are gone, so a revoke or shutdown drain
/// waits for running units.
pub struct Managed<R: Provider> {
    lease: Arc<ManagedLease<R>>,
}

impl<R: Provider> Clone for Managed<R> {
    fn clone(&self) -> Self {
        Self {
            lease: Arc::clone(&self.lease),
        }
    }
}

impl<R: Provider> fmt::Debug for Managed<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Managed")
            .field("resource_key", &self.lease.key)
            .field("generation", &self.lease.guard.generation())
            .finish_non_exhaustive()
    }
}

impl<R: Provider + PinSlots> From<ResourceGuard<R>> for Managed<R> {
    fn from(guard: ResourceGuard<R>) -> Self {
        guard.into_managed()
    }
}

impl<R: Provider> Managed<R> {
    /// The row's key.
    #[must_use]
    pub fn resource_key(&self) -> &ResourceKey {
        &self.lease.key
    }

    /// The closing notice of the lease's admission generation. Once it
    /// fires, new attempts are refused; attempts already granted run on.
    #[must_use]
    pub fn closing(&self) -> LeaseClosing {
        self.lease.guard.closing()
    }

    /// Whether the lease's admission generation closed.
    #[must_use]
    pub fn is_closing(&self) -> bool {
        self.lease.guard.is_closing()
    }
}

impl<R: Provider + PinSlots> Managed<R> {
    pub(crate) fn from_guard(guard: ResourceGuard<R>) -> Self {
        let managed = Arc::clone(guard.managed());
        managed.rate_limiter.latch_per_attempt();
        let cap = match guard.topology_tag() {
            TopologyTag::Pool | TopologyTag::Bounded => EXCLUSIVE_UNIT_CAP,
            TopologyTag::Resident | TopologyTag::Custom => SHARED_UNIT_CAP,
        };
        let lease = ManagedLease {
            generation: Arc::clone(guard.admission()),
            key: guard.resource_key().clone(),
            metrics: guard.metrics().cloned(),
            events: guard.event_bus().cloned(),
            units: Arc::new(Semaphore::new(cap)),
            tainted: AtomicBool::new(false),
            managed,
            guard,
        };
        Self {
            lease: Arc::new(lease),
        }
    }

    /// Submits `operation` as one unit of work.
    ///
    /// The unit is lazy: nothing happens until it is first polled. Its first
    /// poll waits for one of the lease's unit slots, then hands the
    /// operation to the runtime, which runs it under the unit's deadline and
    /// settles it. Dropping the [`Unit`] before its first poll means the
    /// operation never ran; dropping it later only stops waiting — the
    /// runtime still settles the unit and the lease stays held until it
    /// ends.
    pub fn submit<O: Operation<R>>(&self, operation: O) -> Unit<O::Output> {
        let shared = Arc::new(UnitShared::new());
        let span = tracing::info_span!(
            "nebula.resource.unit",
            key = %self.lease.key,
            operation = type_name::<O>(),
            attempts = tracing::field::Empty,
            sent = tracing::field::Empty,
            outcome = tracing::field::Empty,
        );
        let run = start_unit::<R, O>(
            Arc::clone(&self.lease),
            Arc::clone(&shared),
            operation,
            span.clone(),
        )
        .instrument(span);
        Unit {
            shared,
            key: self.lease.key.clone(),
            run: Box::pin(run),
        }
    }
}

/// State one unit shares between its handle, its runtime task and its
/// attempts.
struct UnitShared {
    /// `PENDING` until the first grant or a cancel, whichever comes first.
    state: AtomicU8,
    /// Fired by [`Unit::cancel`] while no attempt was granted.
    cancel: CancellationToken,
    deadline: Mutex<tokio::time::Instant>,
    /// Attempts granted so far.
    granted: AtomicU32,
    /// Worst settled [`SentState`] rank across granted attempts.
    worst: AtomicU8,
}

impl UnitShared {
    fn new() -> Self {
        let now = tokio::time::Instant::now().into_std();
        let deadline = crate::deadline::deadline_after(
            now,
            UNIT_DEADLINE_CAP,
            crate::deadline::UNBOUNDED_HORIZON,
        );
        Self {
            state: AtomicU8::new(PENDING),
            cancel: CancellationToken::new(),
            deadline: Mutex::new(tokio::time::Instant::from_std(deadline)),
            granted: AtomicU32::new(0),
            worst: AtomicU8::new(SentState::NotSent.rank()),
        }
    }

    fn deadline(&self) -> tokio::time::Instant {
        *self.deadline.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn is_granted(&self) -> bool {
        self.state.load(Ordering::Acquire) == GRANTED
    }

    /// Grants one attempt, unless the unit was cancelled before its first.
    fn grant(&self) -> Result<(), OpError> {
        match self
            .state
            .compare_exchange(PENDING, GRANTED, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) | Err(GRANTED) => {
                self.granted.fetch_add(1, Ordering::AcqRel);
                Ok(())
            },
            Err(_) => Err(cancelled_before_grant()),
        }
    }

    /// Cancels the unit if no attempt was granted yet; otherwise nothing.
    fn cancel(&self) {
        if self
            .state
            .compare_exchange(PENDING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.cancel.cancel();
        }
    }

    fn record(&self, sent: SentState) {
        self.worst.fetch_max(sent.rank(), Ordering::AcqRel);
    }

    fn attempts(&self) -> u32 {
        self.granted.load(Ordering::Acquire)
    }

    /// The unit's sent state: `NotSent` when no attempt was granted, however
    /// the author settled; otherwise the worst settled attempt, raised to
    /// `MaybeSent` when the unit ended abnormally (deadline, panic).
    fn fold(&self, abnormal: bool) -> SentState {
        if self.attempts() == 0 {
            return SentState::NotSent;
        }
        if abnormal {
            return SentState::MaybeSent;
        }
        SentState::from_rank(self.worst.load(Ordering::Acquire))
    }

    fn state_name(&self) -> &'static str {
        match self.state.load(Ordering::Acquire) {
            PENDING => "pending",
            GRANTED => "granted",
            _ => "cancelled",
        }
    }
}

fn cancelled_before_grant() -> OpError {
    OpError::new(
        ErrorKind::Cancelled,
        "unit cancelled before its first attempt",
    )
}

/// One submitted [`Operation`]: a future of its settled outcome.
///
/// Lazy until first polled (see [`Managed::submit`]). Once started the
/// runtime owns the unit: dropping this handle stops waiting but never
/// aborts the operation — the runtime settles it, and the lease is released
/// only after it ends.
#[must_use = "a unit does nothing until awaited; dropped before its first poll it never runs"]
pub struct Unit<T> {
    shared: Arc<UnitShared>,
    key: ResourceKey,
    run: Pin<Box<dyn Future<Output = Result<T, OpError>> + Send>>,
}

impl<T> Unit<T> {
    /// Shortens the unit's deadline to `deadline` (it can never exceed
    /// [`UNIT_DEADLINE_CAP`] from submission). Call it before the first
    /// poll: the runtime reads the deadline when the unit starts.
    pub fn with_deadline(self, deadline: Instant) -> Self {
        {
            let requested = tokio::time::Instant::from_std(deadline);
            let mut current = self
                .shared
                .deadline
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *current = (*current).min(requested);
        }
        self
    }

    /// Cancels the unit if none of its attempts was granted yet: a wait for
    /// a unit slot or for the first attempt's quota ends, and the unit
    /// settles `Cancelled` and `NotSent`. Once an attempt was granted the
    /// cancel is ignored and the runtime settles the unit as it ends.
    /// Idempotent.
    pub fn cancel(&self) {
        self.shared.cancel();
    }
}

impl<T> Future for Unit<T> {
    type Output = Result<T, OpError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().run.as_mut().poll(cx)
    }
}

impl<T> fmt::Debug for Unit<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Unit")
            .field("resource_key", &self.key)
            .field("state", &self.shared.state_name())
            .field("attempts", &self.shared.attempts())
            .finish_non_exhaustive()
    }
}

/// The caller's side of a unit: waits for a unit slot on the lease, then
/// spawns the runtime task and waits for its outcome.
async fn start_unit<R, O>(
    lease: Arc<ManagedLease<R>>,
    shared: Arc<UnitShared>,
    operation: O,
    span: tracing::Span,
) -> Result<O::Output, OpError>
where
    R: Provider + PinSlots,
    O: Operation<R>,
{
    let deadline = shared.deadline();
    let permit = tokio::select! {
        biased;
        () = shared.cancel.cancelled() => Err(cancelled_before_grant()),
        () = lease.generation.token().cancelled() => {
            Err(lease.admission_refusal().err().unwrap_or_else(|| {
                OpError::new(ErrorKind::Cancelled, "lease closing; new attempts refused")
            }))
        },
        permit = Arc::clone(&lease.units).acquire_owned() => permit.map_err(|_closed| {
            OpError::new(ErrorKind::Cancelled, "lease closing; new attempts refused")
        }),
        () = tokio::time::sleep_until(deadline) => Err(OpError::new(
            ErrorKind::Backpressure,
            "the lease's unit slots stayed full until the unit deadline",
        )),
    };
    let permit = match permit {
        Ok(permit) => permit,
        Err(refusal) => {
            let result = Err(refusal.settled(SentState::NotSent, O::EFFECT, &lease.key));
            lease.record_settled(&span, &result, SentState::NotSent, 0);
            return result;
        },
    };
    let runtime = tokio::spawn(
        run_unit::<R, O>(
            Arc::clone(&lease),
            Arc::clone(&shared),
            operation,
            permit,
            deadline,
            span.clone(),
        )
        .instrument(span),
    );
    match runtime.await {
        Ok(outcome) => outcome,
        // Only a runtime shutdown aborts the task; the unit never settled.
        Err(_aborted) => Err(OpError::new(
            ErrorKind::Cancelled,
            "the runtime stopped before the unit settled",
        )
        .settled(shared.fold(true), O::EFFECT, &lease.key)),
    }
}

/// The runtime's side of a unit: runs the operation under the deadline and
/// settles the outcome whether or not anyone waits. The slots are pinned at
/// the unit's first grant, not here.
async fn run_unit<R, O>(
    lease: Arc<ManagedLease<R>>,
    shared: Arc<UnitShared>,
    operation: O,
    _permit: OwnedSemaphorePermit,
    deadline: tokio::time::Instant,
    span: tracing::Span,
) -> Result<O::Output, OpError>
where
    R: Provider + PinSlots,
    O: Operation<R>,
{
    let max_attempts = operation.max_attempts();
    let outcome = {
        let mut cx = OpCx {
            lease: &lease,
            pin: None,
            shared: &shared,
            deadline,
            max_attempts,
        };
        let run = tokio::time::timeout_at(deadline, operation.run(&mut cx));
        AssertUnwindSafe(run).catch_unwind().await
    };
    let (result, abnormal) = match outcome {
        Ok(Ok(result)) => (result, false),
        Ok(Err(_elapsed)) => (
            Err(OpError::new(ErrorKind::Transient, "unit deadline elapsed")),
            true,
        ),
        Err(_panic) => {
            let kind = if shared.attempts() == 0 {
                ErrorKind::Permanent
            } else {
                ErrorKind::Transient
            };
            (Err(OpError::new(kind, "operation panicked")), true)
        },
    };
    let sent = shared.fold(abnormal);
    let result = result.map_err(|error| error.settled(sent, O::EFFECT, &lease.key));
    lease.record_settled(&span, &result, sent, shared.attempts());
    result
}

/// What an [`Operation`] runs against: the unit's deadline, its attempt
/// budget and the lease's closing notice, and the only way to reach the
/// provider — [`attempt`](Self::attempt).
pub struct OpCx<'u, R: Provider + PinSlots> {
    lease: &'u ManagedLease<R>,
    /// The unit's slots, pinned at its first grant.
    pin: Option<UnitPin<R::Pinned>>,
    shared: &'u UnitShared,
    deadline: tokio::time::Instant,
    max_attempts: NonZeroU32,
}

impl<R: Provider + PinSlots> fmt::Debug for OpCx<'_, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpCx")
            .field("resource_key", &self.lease.key)
            .field("attempts", &self.shared.attempts())
            .field("max_attempts", &self.max_attempts)
            .finish_non_exhaustive()
    }
}

impl<R: Provider + PinSlots> OpCx<'_, R> {
    /// Asks for one provider attempt costing `cost`.
    ///
    /// The single linearization point of a unit. In order:
    ///
    /// 1. **Budget** — past [`Operation::max_attempts`] granted attempts the
    ///    attempt is refused permanently.
    /// 2. **Admission** — the lease's admission generation must be open:
    ///    `Revoked` for a tainted row, `CredentialUnavailable` when a
    ///    credential suspension closed it, `Cancelled` on removal or
    ///    shutdown.
    /// 3. **Quota** — unless the cost is [`Cost::FREE`], the cost is booked
    ///    on the row's limit, waiting for its slot no later than the unit's
    ///    deadline. The wait ends early when the lease closes or the unit is
    ///    cancelled before its first grant; a slot booked meanwhile is
    ///    forfeited. A cost above the burst fails permanently, a slot past
    ///    the deadline is `Exhausted` with a `retry_after`, an unreachable
    ///    limit store is `Backpressure`.
    /// 4. **Final admission** — the lease is checked again, after the wait.
    ///    Until the strict per-acquire credential read lands, this re-checks
    ///    local admission only, not a fresh credential read.
    /// 5. **Grant** — the attempt is granted unless [`Unit::cancel`] won the
    ///    race for the unit's first grant.
    ///
    /// Every refusal means nothing reached the provider.
    ///
    /// # Errors
    ///
    /// The refusal of whichever step refused, as above.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it resolves forfeits any booked slot and
    /// grants nothing.
    pub async fn attempt(&mut self, cost: Cost) -> Result<Attempt<'_, R>, OpError> {
        let (lease, shared) = (self.lease, self.shared);
        let admitted = self.admit(&cost).await;
        lease.record_attempt(admitted.is_ok());
        let pin = admitted?;
        Ok(Attempt {
            lease,
            pinned: pin.pinned(),
            shared,
            cost,
            settled: false,
        })
    }

    /// Steps 1–5 of [`attempt`](Self::attempt); on a grant, the unit's pin
    /// (captured now for its first grant).
    async fn admit(&mut self, cost: &Cost) -> Result<&UnitPin<R::Pinned>, OpError> {
        self.admit_local(cost).await?;
        // The first grant pins the slots; a refused first attempt keeps no
        // pin, so the next attempt pins afresh.
        let (pin, fresh) = match self.pin.take() {
            Some(pin) => (pin, false),
            None => (capture_pin(&self.lease.managed), true),
        };
        // The seam for the strict per-attempt credential read: today the
        // lease's local admission only.
        let granted = self
            .lease
            .admission_refusal()
            .and_then(|()| self.shared.grant());
        match granted {
            Ok(()) => Ok(&*self.pin.insert(pin)),
            Err(refusal) => {
                if !fresh {
                    self.pin = Some(pin);
                }
                Err(refusal)
            },
        }
    }

    /// Budget, lease admission and the quota booking: every step before the
    /// unit's slots are pinned.
    async fn admit_local(&self, cost: &Cost) -> Result<(), OpError> {
        if self.shared.attempts() >= self.max_attempts.get() {
            return Err(OpError::new(
                ErrorKind::Permanent,
                "attempt budget exhausted",
            ));
        }
        self.lease.admission_refusal()?;
        if !cost.is_free() {
            let limiter = &self.lease.managed.rate_limiter;
            let deadline = Some(self.deadline.into_std());
            let cancel = (!self.shared.is_granted()).then_some(&self.shared.cancel);
            let booking = async {
                match cost.key() {
                    Some((dimension, value)) => {
                        limiter
                            .ready_for_weighted(dimension, value, cost.permits(), deadline)
                            .await
                    },
                    None => limiter.ready_weighted(cost.permits(), deadline).await,
                }
            };
            if let Err(error) = limiter
                .wait_under(&self.lease.generation, cancel, booking)
                .await
            {
                self.lease.admission_refusal()?;
                return Err(quota_refusal(&error));
            }
        }
        Ok(())
    }

    /// The unit's deadline: the operation is stopped at it, and no quota wait
    /// runs past it.
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.deadline.into_std()
    }

    /// The closing notice of the lease. A granted attempt is not aborted
    /// when it fires; an operation that wants to stop early selects on
    /// [`LeaseClosing::closed`].
    #[must_use]
    pub fn closing(&self) -> LeaseClosing {
        self.lease.guard.closing()
    }

    /// Attempts granted to this unit so far.
    #[must_use]
    pub fn attempts(&self) -> u32 {
        self.shared.attempts()
    }
}

/// The refusal of an attempt whose quota booking failed.
fn quota_refusal(error: &Error) -> OpError {
    let detail = match error.kind() {
        ErrorKind::Cancelled => "unit cancelled before its first attempt",
        ErrorKind::Exhausted { .. } => "rate limit slot lands past the unit deadline",
        ErrorKind::Backpressure => "rate limit store unavailable; attempt refused",
        ErrorKind::Permanent => "rate limit can never grant the attempt's cost",
        _ => "rate limit refused the attempt",
    };
    OpError::new(error.kind().clone(), detail)
}

/// One granted provider attempt.
///
/// The only way to reach the instance and the unit's pinned credential
/// slots. Settle it with what happened to the request; an attempt dropped
/// unsettled counts as [`SentState::MaybeSent`].
pub struct Attempt<'a, R: Provider + PinSlots> {
    lease: &'a ManagedLease<R>,
    pinned: &'a R::Pinned,
    shared: &'a UnitShared,
    cost: Cost,
    settled: bool,
}

impl<R: Provider + PinSlots> fmt::Debug for Attempt<'_, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Attempt")
            .field("resource_key", &self.lease.key)
            .field("cost", &self.cost)
            .field("settled", &self.settled)
            .finish_non_exhaustive()
    }
}

impl<R: Provider + PinSlots> Attempt<'_, R> {
    /// The instance the lease holds.
    #[must_use]
    pub fn instance(&self) -> &R::Instance {
        &self.lease.guard
    }

    /// The credential slots pinned at the unit's first grant; the same for
    /// every attempt of the unit.
    #[must_use]
    pub fn slots(&self) -> &R::Pinned {
        self.pinned
    }

    /// Reports what the provider said about its limit on this attempt: a
    /// [`Verdict::Throttled`] pauses every caller of the quota (a keyed
    /// cost's [`Verdict::KeyThrottled`] pauses only its key), a
    /// [`Verdict::Pass`] resets the backoff. Bounded; never fails the unit.
    pub async fn report(&self, verdict: Verdict) {
        self.lease
            .managed
            .rate_limiter
            .report(verdict, self.cost.key())
            .await;
    }

    /// Marks the lease tainted: when it is released it is destroyed, not
    /// recycled. Use it when the instance is left in an unknown state.
    pub fn taint(&self) {
        self.lease.tainted.store(true, Ordering::Release);
    }

    /// Settles the attempt with what happened to the request.
    pub fn settle(self, sent: SentState) {
        let mut attempt = self;
        attempt.shared.record(sent);
        attempt.settled = true;
    }
}

impl<R: Provider + PinSlots> Drop for Attempt<'_, R> {
    fn drop(&mut self) {
        if !self.settled {
            self.shared.record(SentState::MaybeSent);
        }
    }
}
