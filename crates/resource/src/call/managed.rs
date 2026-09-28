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
    cost::{Cost, Effect, SentState},
    error::OpError,
    pin::PinSlots,
    row::{CheckoutFit, RowShared},
    session::{SessionBinding, SessionProvider},
    strict::{UnitPin, capture_pin},
};
use crate::{
    error::{Error, ErrorKind},
    events::ResourceEvent,
    guard::{LeaseClosing, ResourceGuard},
    metrics::{ResourceOpsMetrics, SessionOutcome},
    rate_limit::{DEFAULT_MAX_PENALTY, Verdict},
    resource::Provider,
    runtime::{
        admission::{AdmissionGeneration, CloseCause},
        managed::ManagedResource,
    },
    topology::{PoolProvider, Pooled},
    topology_tag::TopologyTag,
};

/// Host cap on a unit's deadline, and the deadline a unit gets unless
/// [`Unit::with_deadline`] shortens it. Interim: equal to
/// [`DEFAULT_MAX_PENALTY`] until the package fixes a host budget.
pub const UNIT_DEADLINE_CAP: std::time::Duration = DEFAULT_MAX_PENALTY;

/// Units that may run at once on one lease whose topology checks out an
/// instance exclusively ([`Pooled`], [`Bounded`](crate::Bounded)). A
/// [`ManagedRow`](super::ManagedRow) has no lease-wide cap: it checks out
/// per attempt.
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
pub(super) struct ManagedLease<R: Provider> {
    guard: ResourceGuard<R>,
    pub(super) managed: Arc<ManagedResource<R>>,
    pub(super) generation: Arc<AdmissionGeneration>,
    pub(super) key: ResourceKey,
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
    pub(super) fn admission_refusal(&self) -> Result<(), OpError> {
        generation_refusal(&self.managed, &self.generation)
    }
}

/// The refusal of new work under `generation` on `managed`: `Revoked` for a
/// tainted row, then, once the generation closed, `CredentialUnavailable`
/// when a credential suspension closed it and `Cancelled` otherwise.
pub(super) fn generation_refusal<R: Provider>(
    managed: &ManagedResource<R>,
    generation: &AdmissionGeneration,
) -> Result<(), OpError> {
    if managed.is_tainted() {
        return Err(OpError::new(
            ErrorKind::Revoked,
            "resource tainted by a credential revoke; new attempts refused",
        ));
    }
    if !generation.is_closed() {
        return Ok(());
    }
    Err(match generation.close_cause() {
        Some(CloseCause::Credential(reason)) => OpError::new(
            ErrorKind::CredentialUnavailable { reason },
            "bound credential unavailable; new attempts refused",
        ),
        None => OpError::new(ErrorKind::Cancelled, "lease closing; new attempts refused"),
    })
}

/// What a unit runs against: the lease a [`Managed`] facade owns, or the
/// row a [`ManagedRow`](super::ManagedRow) checks out from per attempt.
pub(super) enum UnitHost<R: Provider> {
    /// A lease-wide facade: every attempt runs on the lease's instance.
    Lease(Arc<ManagedLease<R>>),
    /// A row facade: every attempt checks out an instance of its own.
    Row(Arc<RowShared<R>>),
}

impl<R: Provider> UnitHost<R> {
    pub(super) fn key(&self) -> &ResourceKey {
        match self {
            Self::Lease(lease) => &lease.key,
            Self::Row(row) => &row.key,
        }
    }

    pub(super) fn managed(&self) -> &Arc<ManagedResource<R>> {
        match self {
            Self::Lease(lease) => &lease.managed,
            Self::Row(row) => &row.managed,
        }
    }

    fn metrics(&self) -> Option<&ResourceOpsMetrics> {
        match self {
            Self::Lease(lease) => lease.metrics.as_ref(),
            Self::Row(row) => row.link.metrics(),
        }
    }

    fn events(&self) -> Option<&Arc<EventBus<ResourceEvent>>> {
        match self {
            Self::Lease(lease) => lease.events.as_ref(),
            Self::Row(row) => Some(row.link.admission().events()),
        }
    }

    /// The admission generation a unit of this host is refused under: the
    /// lease's own, or the row's current one when the unit starts.
    fn unit_generation(&self) -> Result<Arc<AdmissionGeneration>, OpError> {
        match self {
            Self::Lease(lease) => Ok(Arc::clone(&lease.generation)),
            Self::Row(row) => row.unit_generation(),
        }
    }

    fn record_attempt(&self, granted: bool) {
        if let Some(metrics) = self.metrics() {
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
        if let Some(metrics) = self.metrics() {
            metrics.record_call_unit(sent);
        }
        if let Err(error) = result {
            if error.is_outcome_unknown() {
                tracing::warn!(
                    parent: span,
                    resource.key = %self.key(),
                    kind = %error.kind(),
                    sent = sent.as_str(),
                    "managed unit failed with an unknown outcome; reconcile before retrying"
                );
                if let Some(events) = self.events() {
                    let _ = events.emit(ResourceEvent::UnitOutcomeUnknown {
                        key: self.key().clone(),
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
        submit_unit(
            UnitHost::Lease(Arc::clone(&self.lease)),
            &UnitScope::default(),
            operation,
            O::EFFECT,
            type_name::<O>(),
        )
    }
}

/// What every unit of a row facade inherits from the caller that built the
/// facade: its cancellation and its deadline.
///
/// A parent cancellation that fires before a unit's first grant cancels the
/// unit as [`Unit::cancel`] does (`Cancelled`, `NotSent`); after the first
/// grant it is ignored and the unit runs to its own deadline (Design
/// DX-API.md:114). The parent deadline bounds every unit's deadline, below
/// [`UNIT_DEADLINE_CAP`]; [`Unit::with_deadline`] can only shorten it
/// further. A lease facade's units inherit nothing.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum UnitEffectPolicy {
    /// Library callers may submit every declared operation effect.
    #[default]
    Any,
    /// Action execution without effect-owner authority may perform reads only.
    ReadOnly,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct UnitScope {
    /// The parent cancellation: each unit's own cancel is a child of it.
    pub(crate) cancel: Option<CancellationToken>,
    /// The parent deadline.
    pub(crate) deadline: Option<Instant>,
    /// Effects this caller is authorized to submit.
    pub(crate) effect_policy: UnitEffectPolicy,
}

impl UnitScope {
    /// The scope of a caller's `ctx` (its cancellation token) and `options`
    /// (its deadline).
    pub(crate) fn from_parts(
        ctx: &crate::context::ResourceContext,
        options: &crate::options::AcquireOptions,
    ) -> Self {
        Self {
            cancel: Some(ctx.cancel_token().clone()),
            deadline: options.deadline,
            effect_policy: UnitEffectPolicy::Any,
        }
    }

    /// Restricts the facade to operations that declare [`Effect::Read`].
    pub(crate) fn read_only(mut self) -> Self {
        self.effect_policy = UnitEffectPolicy::ReadOnly;
        self
    }

    /// Whether the caller's authority admits `effect`.
    fn admits(&self, effect: Effect) -> bool {
        matches!(self.effect_policy, UnitEffectPolicy::Any) || effect == Effect::Read
    }
}

/// Builds the lazy [`Unit`] of `operation` on `host`, in a
/// `nebula.resource.unit` span naming it `operation_name`, under `scope`.
pub(super) fn submit_unit<R, O>(
    host: UnitHost<R>,
    scope: &UnitScope,
    operation: O,
    effect: Effect,
    operation_name: &'static str,
) -> Unit<O::Output>
where
    R: Provider + PinSlots,
    O: Operation<R>,
{
    let shared = Arc::new(UnitShared::new(scope));
    let key = host.key().clone();
    let span = tracing::info_span!(
        "nebula.resource.unit",
        key = %key,
        operation = operation_name,
        attempts = tracing::field::Empty,
        sent = tracing::field::Empty,
        outcome = tracing::field::Empty,
    );
    let run: Pin<Box<dyn Future<Output = Result<O::Output, OpError>> + Send>> = if scope
        .admits(effect)
    {
        Box::pin(
            start_unit::<R, O>(host, Arc::clone(&shared), operation, effect, span.clone())
                .instrument(span),
        )
    } else {
        let denied_shared = Arc::clone(&shared);
        let denied_span = span.clone();
        Box::pin(
                async move {
                    // Keep the operation lazy and let cancellation win while
                    // the unit is still pending, exactly as it does before a
                    // normal unit's first grant.
                    let _operation = operation;
                    let result = match denied_shared.refuse_if_cancelled() {
                        Ok(()) => {
                            tracing::warn!(
                                target: "nebula.resource",
                                parent: &denied_span,
                                effect = effect.as_str(),
                                "managed row refused an external effect without execution-owner authority"
                            );
                            Err(OpError::new(
                                ErrorKind::Permanent,
                                "managed row effect requires execution-owner authority",
                            )
                            .settled(SentState::NotSent, effect, host.key()))
                        },
                        Err(cancelled) => {
                            Err(cancelled.settled(SentState::NotSent, effect, host.key()))
                        },
                    };
                    host.record_settled(&denied_span, &result, SentState::NotSent, 0);
                    result
                }
                .instrument(span),
            )
    };
    Unit { shared, key, run }
}

/// State one unit shares between its handle, its runtime task and its
/// attempts.
pub(super) struct UnitShared {
    /// `PENDING` until the first grant or a cancel, whichever comes first.
    state: AtomicU8,
    /// Fired by [`Unit::cancel`] while no attempt was granted, or by the
    /// parent cancellation of the unit's [`UnitScope`] (a child token): the
    /// latter is honoured only until the first grant.
    cancel: CancellationToken,
    deadline: Mutex<tokio::time::Instant>,
    /// Attempts granted so far.
    granted: AtomicU32,
    /// Worst settled [`SentState`] rank across granted attempts.
    worst: AtomicU8,
}

impl UnitShared {
    /// A pending unit under `scope`: its cancel a child of the parent's, its
    /// deadline the cap from now or the parent's, whichever is earlier.
    pub(super) fn new(scope: &UnitScope) -> Self {
        let now = tokio::time::Instant::now().into_std();
        let capped = crate::deadline::deadline_after(
            now,
            UNIT_DEADLINE_CAP,
            crate::deadline::UNBOUNDED_HORIZON,
        );
        let deadline = scope.deadline.map_or(capped, |parent| capped.min(parent));
        Self {
            state: AtomicU8::new(PENDING),
            cancel: scope
                .cancel
                .as_ref()
                .map_or_else(CancellationToken::new, CancellationToken::child_token),
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

    /// The cancel a wait of the unit races: [`Unit::cancel`] until the
    /// first grant, nothing after it.
    pub(super) fn cancel_before_grant(&self) -> Option<&CancellationToken> {
        (!self.is_granted()).then_some(&self.cancel)
    }

    /// Refuses a unit whose cancel fired while no attempt was granted —
    /// [`Unit::cancel`] or the parent cancellation — and latches it
    /// cancelled. A fired cancel is ignored once an attempt was granted.
    fn refuse_if_cancelled(&self) -> Result<(), OpError> {
        if !self.cancel.is_cancelled() {
            return Ok(());
        }
        match self
            .state
            .compare_exchange(PENDING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
        {
            Err(GRANTED) => Ok(()),
            _ => Err(cancelled_before_grant()),
        }
    }

    /// Grants one attempt, unless the unit was cancelled before its first:
    /// by [`Unit::cancel`], or by the parent cancellation having fired by
    /// now. The grant and a parent cancel race here, never after: a parent
    /// cancel that fires once the first attempt was granted is ignored.
    pub(super) fn grant(&self) -> Result<(), OpError> {
        self.refuse_if_cancelled()?;
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

    pub(super) fn attempts(&self) -> u32 {
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

pub(super) fn cancelled_before_grant() -> OpError {
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

/// Waits for one of `lease`'s unit slots, until the unit's deadline.
async fn lease_unit_slot<R: Provider>(
    lease: &ManagedLease<R>,
    shared: &UnitShared,
    deadline: tokio::time::Instant,
) -> Result<OwnedSemaphorePermit, OpError> {
    tokio::select! {
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
    }
}

/// The caller's side of a unit: waits for a unit slot on a lease host, then
/// spawns the runtime task and waits for its outcome. `effect` is what the
/// unit's settled error reports.
async fn start_unit<R, O>(
    host: UnitHost<R>,
    shared: Arc<UnitShared>,
    operation: O,
    effect: Effect,
    span: tracing::Span,
) -> Result<O::Output, OpError>
where
    R: Provider + PinSlots,
    O: Operation<R>,
{
    let deadline = shared.deadline();
    // A unit cancelled before its first poll — its parent cancellation
    // already fired — settles here: nothing is spawned or checked out.
    // A row facade has no unit slots: each attempt queues on the row gate
    // with nothing checked out instead.
    let permit = match &host {
        _ if shared.cancel.is_cancelled() => shared
            .refuse_if_cancelled()
            .and(Err(cancelled_before_grant())),
        UnitHost::Lease(lease) => lease_unit_slot(lease, &shared, deadline).await.map(Some),
        // Awaited inside a session of the same row, the unit would wait for
        // the row gate while that session holds a checkout.
        UnitHost::Row(row) if super::session::in_session_of(row.marker()) => Err(OpError::new(
            ErrorKind::Permanent,
            "a unit of a row awaited inside a session of the same row; refused",
        )),
        UnitHost::Row(_) => Ok(None),
    };
    let permit = match permit {
        Ok(permit) => permit,
        Err(refusal) => {
            let result = Err(refusal.settled(SentState::NotSent, effect, host.key()));
            host.record_settled(&span, &result, SentState::NotSent, 0);
            return result;
        },
    };
    let key = host.key().clone();
    let runtime = tokio::spawn(
        run_unit::<R, O>(
            host,
            Arc::clone(&shared),
            operation,
            effect,
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
        .settled(shared.fold(true), effect, &key)),
    }
}

/// The runtime's side of a unit: runs the operation under the deadline and
/// settles the outcome whether or not anyone waits. The slots are pinned at
/// the unit's first grant, not here.
async fn run_unit<R, O>(
    host: UnitHost<R>,
    shared: Arc<UnitShared>,
    operation: O,
    effect: Effect,
    _permit: Option<OwnedSemaphorePermit>,
    deadline: tokio::time::Instant,
    span: tracing::Span,
) -> Result<O::Output, OpError>
where
    R: Provider + PinSlots,
    O: Operation<R>,
{
    let generation = match host.unit_generation() {
        Ok(generation) => generation,
        Err(refusal) => {
            let result = Err(refusal.settled(SentState::NotSent, effect, host.key()));
            host.record_settled(&span, &result, SentState::NotSent, 0);
            return result;
        },
    };
    let max_attempts = operation.max_attempts();
    let outcome = {
        let mut cx = OpCx {
            host: &host,
            generation: &generation,
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
    let result = result.map_err(|error| error.settled(sent, effect, host.key()));
    host.record_settled(&span, &result, sent, shared.attempts());
    result
}

/// What an [`Operation`] runs against: the unit's deadline, its attempt
/// budget and the lease's closing notice, and the only way to reach the
/// provider — [`attempt`](Self::attempt).
pub struct OpCx<'u, R: Provider + PinSlots> {
    pub(super) host: &'u UnitHost<R>,
    /// The admission generation the unit is refused under: the lease's, or
    /// the row's current one when the unit started.
    pub(super) generation: &'u Arc<AdmissionGeneration>,
    /// The unit's slots, pinned at its first grant.
    pub(super) pin: Option<UnitPin<R::Pinned>>,
    pub(super) shared: &'u UnitShared,
    pub(super) deadline: tokio::time::Instant,
    max_attempts: NonZeroU32,
}

impl<R: Provider + PinSlots> fmt::Debug for OpCx<'_, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpCx")
            .field("resource_key", self.host.key())
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
    /// 4. **Credential read** — on a strict manager (one with a credential
    ///    observer) and a row with bound slots, every bound credential's
    ///    availability is read after every wait of the attempt, outside every
    ///    lock, bounded by the unit's deadline and the read timeout, and
    ///    raced against the lease's generation and [`Unit::cancel`]. Every
    ///    attempt reads, [`Cost::FREE`] included; concurrent attempts of a
    ///    credential share reads join-next (only a read issued after the
    ///    attempt arrived answers it). An interim manager or a slot-less row
    ///    reads nothing.
    /// 5. **Pin** — the unit's first grant pins its credential slots
    ///    ([`Attempt::slots`]).
    /// 6. **Registration and grant** — on a strict read, under the manager's
    ///    admission lock: a revoke taint is `Revoked`, shutdown `Cancelled`;
    ///    the read is applied to the row (a blocked credential suspends it
    ///    and closes its leases, a usable one reopens or readmits it) and a
    ///    denying credential refuses `CredentialUnavailable` with the read's
    ///    reason; a suspended row or a closed lease refuses; a pin a rotation
    ///    superseded since the unit pinned it refuses `Rebinding`. Without a
    ///    strict read the lease's admission is re-checked, lock-free. The
    ///    attempt is then granted unless [`Unit::cancel`] won the race for
    ///    the unit's first grant.
    ///
    /// Every refusal means nothing reached the provider; a quota slot booked
    /// for a refused attempt is forfeited.
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
        let (host, shared) = (self.host, self.shared);
        let admitted = self.admit(&cost).await;
        host.record_attempt(admitted.is_ok());
        let (target, pin) = admitted?;
        Ok(Attempt {
            target,
            managed: host.managed(),
            pinned: pin.pinned(),
            shared,
            cost,
            settled: false,
        })
    }

    /// A session's single attempt on a row host: admitted like a row
    /// attempt, on a checkout that fits the provider's
    /// [`SessionBinding`], and destroyed on release unless the session
    /// closed cleanly ([`Attempt::end_session`]).
    pub(super) async fn attempt_session(&mut self, cost: Cost) -> Result<Attempt<'_, R>, OpError>
    where
        R: SessionProvider + PoolProvider + Provider<Topology = Pooled<R>> + Clone,
    {
        let UnitHost::Row(row) = self.host else {
            return Err(OpError::new(
                ErrorKind::Permanent,
                "sessions run on a managed row",
            ));
        };
        let fit: Option<CheckoutFit<R>> = match R::BINDING {
            SessionBinding::Connection => Some(built_at_pinned_epoch::<R>),
            SessionBinding::Session => None,
        };
        let (host, shared) = (self.host, self.shared);
        let admitted = self.admit_row(row, &cost, fit).await;
        host.record_attempt(admitted.is_ok());
        let (mut checkout, pin) = admitted?;
        checkout.taint_on_abandon = true;
        checkout.session = Some(SessionWatch {
            metrics: row.link.metrics().cloned(),
        });
        Ok(Attempt {
            target: AttemptTarget::Checkout(Box::new(checkout)),
            managed: host.managed(),
            pinned: pin.pinned(),
            shared,
            cost,
            settled: false,
        })
    }

    /// Steps 1–6 of [`attempt`](Self::attempt); on a grant, what the
    /// attempt runs on and the unit's pin.
    async fn admit(
        &mut self,
        cost: &Cost,
    ) -> Result<(AttemptTarget<'_, R>, &UnitPin<R::Pinned>), OpError> {
        match self.host {
            UnitHost::Lease(lease) => {
                self.admit_local(cost, None).await?;
                let cancel = self.shared.cancel_before_grant();
                let reading = lease.read_credentials(self.deadline, cancel).await?;
                // No await from here on: the pin is never lost to a dropped
                // future. The first grant pins the slots; a refused first
                // attempt keeps no pin, so the next attempt pins afresh.
                let (pin, fresh) = match self.pin.take() {
                    Some(pin) => (pin, false),
                    None => (capture_pin(&lease.managed), true),
                };
                let granted = lease.register(reading.as_ref(), &pin, self.shared);
                match granted {
                    Ok(()) => Ok((AttemptTarget::Lease(lease), &*self.pin.insert(pin))),
                    Err(refusal) => {
                        if !fresh {
                            self.pin = Some(pin);
                        }
                        Err(refusal)
                    },
                }
            },
            UnitHost::Row(row) => {
                let (checkout, pin) = self.admit_row(row, cost, None).await?;
                Ok((AttemptTarget::Checkout(Box::new(checkout)), pin))
            },
        }
    }

    /// Budget, the host's admission and the quota booking: every step
    /// before anything is checked out or pinned. A lease host books nothing
    /// for [`Cost::FREE`]; a row host (`row`) still honours the limit's
    /// pauses for it, and its waits also end when the manager shuts down.
    pub(super) async fn admit_local(
        &self,
        cost: &Cost,
        row: Option<&RowShared<R>>,
    ) -> Result<(), OpError> {
        if self.shared.attempts() >= self.max_attempts.get() {
            return Err(OpError::new(
                ErrorKind::Permanent,
                "attempt budget exhausted",
            ));
        }
        let managed = self.host.managed();
        let generation = self.generation;
        let precheck = || match row {
            Some(row) => row.precheck(generation),
            None => generation_refusal(managed, generation),
        };
        precheck()?;
        if cost.is_free() && row.is_none() {
            return Ok(());
        }
        let limiter = &managed.rate_limiter;
        let deadline = Some(self.deadline.into_std());
        let booking = async {
            if cost.is_free() {
                return limiter.ready_to_acquire(deadline).await;
            }
            match cost.key() {
                Some((dimension, value)) => {
                    limiter
                        .ready_for_weighted(dimension, value, cost.permits(), deadline)
                        .await
                },
                None => limiter.ready_weighted(cost.permits(), deadline).await,
            }
        };
        let shutdown = async {
            match row {
                Some(row) => row.link.admission().cancel().cancelled().await,
                None => std::future::pending().await,
            }
        };
        let booked = tokio::select! {
            biased;
            () = shutdown => Err(Error::cancelled()),
            booked = limiter.wait_under(generation, self.shared.cancel_before_grant(), booking) => booked,
        };
        if let Err(error) = booked {
            precheck()?;
            return Err(quota_refusal(&error));
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
        match self.host {
            UnitHost::Lease(lease) => lease.guard.closing(),
            UnitHost::Row(_) => LeaseClosing::of(self.generation),
        }
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
    target: AttemptTarget<'a, R>,
    managed: &'a Arc<ManagedResource<R>>,
    pinned: &'a R::Pinned,
    shared: &'a UnitShared,
    cost: Cost,
    settled: bool,
}

/// The instance one granted attempt runs on.
pub(super) enum AttemptTarget<'a, R: Provider> {
    /// The facade's lease, shared by every attempt of every unit.
    Lease(&'a ManagedLease<R>),
    /// The instance a row attempt checked out for itself.
    Checkout(Box<Checkout<R>>),
}

/// One row attempt's checkout: the lease on its instance (with the row-gate
/// permit), released when the attempt ends.
///
/// Dropping it releases the lease; an attempt that
/// [`taint`](Attempt::taint)ed it, or a session abandoned mid-way
/// (`taint_on_abandon` still set), destroys the instance instead of
/// recycling it.
pub(super) struct Checkout<R: Provider> {
    pub(super) guard: ResourceGuard<R>,
    tainted: AtomicBool,
    /// Set for a session's checkout until the session closed cleanly: an
    /// instance whose session was cut off (deadline, panic, drop) is in an
    /// unknown state.
    taint_on_abandon: bool,
    /// Set while a session runs on the checkout; a checkout dropped with it
    /// set was abandoned mid-session and is counted so.
    session: Option<SessionWatch>,
}

/// A session in progress on a checkout, for its outcome counter.
struct SessionWatch {
    metrics: Option<ResourceOpsMetrics>,
}

impl<R: Provider> Checkout<R> {
    pub(super) fn new(guard: ResourceGuard<R>) -> Self {
        Self {
            guard,
            tainted: AtomicBool::new(false),
            taint_on_abandon: false,
            session: None,
        }
    }
}

impl<R: Provider> Drop for Checkout<R> {
    fn drop(&mut self) {
        if *self.tainted.get_mut() || self.taint_on_abandon {
            self.guard.taint();
        }
        if let Some(SessionWatch {
            metrics: Some(metrics),
        }) = self.session.take()
        {
            metrics.record_session(SessionOutcome::Abandoned);
        }
    }
}

impl<R: Provider + PinSlots> fmt::Debug for Attempt<'_, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Attempt")
            .field("resource_key", &R::key())
            .field("cost", &self.cost)
            .field("settled", &self.settled)
            .finish_non_exhaustive()
    }
}

impl<R: Provider + PinSlots> Attempt<'_, R> {
    /// The instance the attempt runs on: the lease's for a [`Managed`]
    /// facade, the attempt's own checkout for a
    /// [`ManagedRow`](super::ManagedRow).
    #[must_use]
    pub fn instance(&self) -> &R::Instance {
        match &self.target {
            AttemptTarget::Lease(lease) => &lease.guard,
            AttemptTarget::Checkout(checkout) => &checkout.guard,
        }
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
        self.managed
            .rate_limiter
            .report(verdict, self.cost.key())
            .await;
    }

    /// Marks the instance tainted: when its lease is released it is
    /// destroyed, not recycled. Use it when the instance is left in an
    /// unknown state.
    pub fn taint(&self) {
        match &self.target {
            AttemptTarget::Lease(lease) => lease.tainted.store(true, Ordering::Release),
            AttemptTarget::Checkout(checkout) => checkout.tainted.store(true, Ordering::Release),
        }
    }

    /// Settles the attempt with what happened to the request.
    pub fn settle(self, sent: SentState) {
        let mut attempt = self;
        attempt.shared.record(sent);
        attempt.settled = true;
    }
}

/// Whether a pooled checkout was built at the credential slot epoch its
/// unit pinned ([`SessionBinding::Connection`]).
fn built_at_pinned_epoch<R>(guard: &ResourceGuard<R>, pinned_epoch: u64) -> bool
where
    R: PoolProvider + Provider<Topology = Pooled<R>> + Clone,
{
    guard.built_slot_epoch() == Some(pinned_epoch)
}

impl<'a, R> Attempt<'a, R>
where
    R: SessionProvider + PoolProvider + Provider<Topology = Pooled<R>> + Clone,
{
    /// What a session opens on: the provider, the checked-out instance
    /// (mutably — the checkout holds it exclusively), the unit's pinned
    /// slots and the checkout's closing notice. `None` for an attempt that
    /// has no checkout of its own.
    pub(super) fn session_parts(
        &mut self,
    ) -> Option<(&'a R, &mut R::Instance, &'a R::Pinned, LeaseClosing)> {
        let (managed, pinned) = (self.managed, self.pinned);
        let AttemptTarget::Checkout(checkout) = &mut self.target else {
            return None;
        };
        let closing = checkout.guard.closing();
        let instance = checkout.guard.pooled_instance_mut()?;
        Some((&managed.resource, instance, pinned, closing))
    }

    /// The session ended as `outcome`; with `keep` its instance goes back
    /// to the pool when the checkout is released, otherwise it is destroyed.
    pub(super) fn end_session(&mut self, outcome: SessionOutcome, keep: bool) {
        if let AttemptTarget::Checkout(checkout) = &mut self.target {
            if keep {
                checkout.taint_on_abandon = false;
            }
            if let Some(SessionWatch {
                metrics: Some(metrics),
            }) = checkout.session.take()
            {
                metrics.record_session(outcome);
            }
        }
    }
}

impl<R: Provider + PinSlots> Drop for Attempt<'_, R> {
    fn drop(&mut self) {
        if !self.settled {
            self.shared.record(SentState::MaybeSent);
        }
    }
}
