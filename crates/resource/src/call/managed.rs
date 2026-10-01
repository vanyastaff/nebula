//! The lease-owning facade ([`Lease`]), its units ([`Submission`]) and the
//! per-unit runtime: permit, spawn, deadline, attempt admission and the
//! settled outcome.

use std::{
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
    declaration::{
        IdempotencyKey, check_declaration, check_key_part, is_valid_declaration,
        local_idempotency_key,
    },
    error::{OperationError, Signal},
    journal::{EffectJournal, ErrorKindCode, JournalSlot, Recovery},
    owned::{self, CallNote, JournalDeclaration, OutputCodec, OwnedEffect, Prepared},
    pin::PinSlots,
    row::{CheckoutFit, RowShared},
    session::{SessionBinding, SessionProvider},
    strict::{UnitPin, capture_pin},
    work::{Plain, UnitWork},
};
use crate::{
    dedup::SlotIdentity,
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
/// [`Submission::with_deadline`] shortens it. Interim: equal to
/// [`DEFAULT_MAX_PENALTY`] until the package fixes a host budget.
pub const OPERATION_DEADLINE_CAP: std::time::Duration = DEFAULT_MAX_PENALTY;

/// Units that may run at once on one lease whose topology checks out an
/// instance exclusively ([`Pooled`], [`Bounded`](crate::Bounded)). A
/// [`ResourceHandle`](super::ResourceHandle) has no lease-wide cap: it checks out
/// per attempt.
const EXCLUSIVE_UNIT_CAP: usize = 1;

/// Units that may run at once on one lease of a shared instance
/// ([`Resident`](crate::Resident), custom topologies). Interim.
const SHARED_UNIT_CAP: usize = 64;

const PENDING: u8 = 0;
const GRANTED: u8 = 1;
const CANCELLED: u8 = 2;

/// The lease a [`Lease`] facade owns, shared by every unit it started.
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
    pub(super) fn admission_refusal(&self) -> Result<(), OperationError> {
        generation_refusal(&self.managed, &self.generation)
    }
}

/// The refusal of new work under `generation` on `managed`: `Revoked` for a
/// tainted row, then, once the generation closed, `CredentialUnavailable`
/// when a credential suspension closed it and `Cancelled` otherwise.
pub(super) fn generation_refusal<R: Provider>(
    managed: &ManagedResource<R>,
    generation: &AdmissionGeneration,
) -> Result<(), OperationError> {
    if managed.is_tainted() {
        return Err(OperationError::new(
            ErrorKind::Revoked,
            "resource tainted by a credential revoke; new attempts refused",
        ));
    }
    if !generation.is_closed() {
        return Ok(());
    }
    Err(match generation.close_cause() {
        Some(CloseCause::Credential(reason)) => OperationError::new(
            ErrorKind::CredentialUnavailable { reason },
            "bound credential unavailable; new attempts refused",
        ),
        None => OperationError::new(ErrorKind::Cancelled, "lease closing; new attempts refused"),
    })
}

/// What a unit runs against: the lease a [`Lease`] facade owns, or the
/// row a [`ResourceHandle`](super::ResourceHandle) checks out from per attempt.
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
    fn unit_generation(&self) -> Result<Arc<AdmissionGeneration>, OperationError> {
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

    /// Records an owned unit that replayed its recorded output: no attempt,
    /// nothing sent by this unit.
    fn record_replayed(&self, span: &tracing::Span) {
        span.record("attempts", 0);
        span.record("sent", SentState::NotSent.as_str());
        span.record("outcome", "replayed");
        tracing::debug!(
            parent: span,
            resource.key = %self.key(),
            "managed unit replayed its recorded effect output"
        );
    }

    /// Records a settled unit: span fields, counters, and the
    /// outcome-unknown event.
    fn record_settled<T>(
        &self,
        span: &tracing::Span,
        result: &Result<T, OperationError>,
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
                    let _ = events.emit(ResourceEvent::OperationOutcomeUnknown {
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
/// Built by [`ResourceGuard::into_lease`]. Provider calls go through
/// [`submit`](Self::submit), one [`Operation`] per [`Submission`]; each attempt of
/// a unit is admitted against the lease, booked on the row's rate limit and
/// settled. There is deliberately no `Deref` to the instance: a call cannot
/// skip admission and the limit by accident, and the instance is reached
/// only inside a granted [`Attempt`].
///
/// ```compile_fail
/// use nebula_resource::{PinSlots, Provider, call::Lease};
///
/// fn skip_the_facade<R: Provider + PinSlots>(managed: &Lease<R>) -> &R::Instance {
///     &**managed
/// }
/// ```
///
/// Cloning shares the lease: the lease is released when the last clone and
/// the last unit any clone started are gone, so a revoke or shutdown drain
/// waits for running units.
pub struct Lease<R: Provider> {
    lease: Arc<ManagedLease<R>>,
}

impl<R: Provider> Clone for Lease<R> {
    fn clone(&self) -> Self {
        Self {
            lease: Arc::clone(&self.lease),
        }
    }
}

impl<R: Provider> fmt::Debug for Lease<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Lease")
            .field("resource_key", &self.lease.key)
            .field("generation", &self.lease.guard.generation())
            .finish_non_exhaustive()
    }
}

impl<R: Provider + PinSlots> From<ResourceGuard<R>> for Lease<R> {
    fn from(guard: ResourceGuard<R>) -> Self {
        guard.into_lease()
    }
}

impl<R: Provider> Lease<R> {
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

impl<R: Provider + PinSlots> Lease<R> {
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
    /// settles it. Dropping the [`Submission`] before its first poll means the
    /// operation never ran; dropping it later only stops waiting — the
    /// runtime still settles the unit and the lease stays held until it
    /// ends. A lease has no execution owner: every effect runs unjournaled.
    ///
    /// A malformed declaration ([`Operation::KEY`], [`Operation::VERSION`],
    /// an `Idempotent` [`Operation::KEY_WINDOW`] of zero) fails the build;
    /// see [`Operation`].
    pub fn submit<O: Operation<R>>(&self, operation: O) -> Submission<O::Output> {
        assert_declaration::<R, O>();
        submit_unit(self.unit_host(), &UnitScope::default(), Plain(operation))
    }

    /// The host every unit of this facade runs against: its lease.
    pub(super) fn unit_host(&self) -> UnitHost<R> {
        UnitHost::Lease(Arc::clone(&self.lease))
    }
}

/// Fails the build for an operation whose constants break the declaration
/// rules. A post-monomorphization error: `cargo check` does not see it, so
/// the runtime refuses the same declaration at submit too.
pub(super) fn assert_declaration<R: Provider + PinSlots, O: Operation<R>>() {
    const {
        assert!(
            is_valid_declaration(O::KEY, O::VERSION, O::EFFECT, O::KEY_WINDOW),
            "Operation::KEY must be 1..=64 bytes of [A-Za-z0-9_.-] starting and ending \
             alphanumeric, VERSION at least 1, and an Idempotent KEY_WINDOW non-zero"
        );
    }
}

/// The refusal detail of an effect on a read-only row facade.
const NO_EFFECT_AUTHORITY: &str = "managed row effect requires execution-owner authority";

/// Which effects the caller that built a row facade may submit.
#[derive(Debug, Clone, Default)]
pub(crate) enum EffectAuthority {
    /// Library callers run every effect unjournaled.
    #[default]
    Unjournaled,
    /// Action execution without effect-owner authority may perform reads
    /// only; an effect is refused with `detail`.
    ReadOnly {
        /// Why the caller has no effect authority.
        detail: &'static str,
    },
    /// Action execution with effect-owner authority: reads as usual,
    /// `Idempotent` and `Write` units driven through `owner`.
    Journaled {
        /// The execution journal that records the row's effects.
        owner: Arc<dyn EffectJournal>,
        /// The row's credential slot identity, as the owner binds effects.
        binding: SlotIdentity,
    },
}

/// What every unit of a row facade inherits from the caller that built the
/// facade: its cancellation, its deadline and its effect authority.
///
/// A parent cancellation that fires before a unit's first grant cancels the
/// unit as [`Submission::cancel`] does (`Cancelled`, `NotSent`); after the first
/// grant it is ignored and the unit runs to its own deadline (Design
/// DX-API.md:114). The parent deadline bounds every unit's deadline, below
/// [`OPERATION_DEADLINE_CAP`]; [`Submission::with_deadline`] can only shorten it
/// further. A lease facade's units inherit nothing.
#[derive(Debug, Clone, Default)]
pub(crate) struct UnitScope {
    /// The parent cancellation: each unit's own cancel is a child of it.
    pub(crate) cancel: Option<CancellationToken>,
    /// The parent deadline.
    pub(crate) deadline: Option<Instant>,
    /// Effects this caller is authorized to submit.
    pub(crate) effect_authority: EffectAuthority,
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
            effect_authority: EffectAuthority::Unjournaled,
        }
    }

    /// Restricts the facade to operations that declare [`Effect::Read`].
    pub(crate) fn read_only(self) -> Self {
        self.read_only_because(NO_EFFECT_AUTHORITY)
    }

    /// Restricts the facade to operations that declare [`Effect::Read`],
    /// refusing an effect with `detail`.
    pub(crate) fn read_only_because(mut self, detail: &'static str) -> Self {
        self.effect_authority = EffectAuthority::ReadOnly { detail };
        self
    }

    /// Drives the facade's `Idempotent` and `Write` units through `owner`;
    /// `binding` is the row's credential slot identity.
    pub(crate) fn journaled(
        mut self,
        owner: Arc<dyn EffectJournal>,
        binding: SlotIdentity,
    ) -> Self {
        self.effect_authority = EffectAuthority::Journaled { owner, binding };
        self
    }

    /// Routes a unit at submit (the declaration, the developer key part,
    /// then the caller's authority) and, on an owned facade, takes the
    /// owner's side of the submit. `Idempotent` and `Write` units route:
    ///
    /// | Authority | `Read` | `Idempotent` / `Write` | streamed `Idempotent` / `Write` |
    /// |---|---|---|---|
    /// | unjournaled (library, lease) | plain | plain | plain |
    /// | read-only | plain | refused `Permanent` | refused `Permanent` |
    /// | journaled | plain, never prepared | through the owner | refused `Permanent` |
    ///
    /// A plain unit that declared a developer key part presents a local
    /// idempotency key; every refusal is unsent.
    fn admit<R, W>(
        &self,
        key: &ResourceKey,
        managed: &ManagedResource<R>,
        work: &W,
    ) -> Result<Route, OperationError>
    where
        R: Provider + PinSlots,
        W: UnitWork<R>,
    {
        if let Some(detail) = work.defect() {
            return Err(OperationError::new(ErrorKind::Permanent, detail));
        }
        let declared = work.declared();
        check_declaration(
            declared.name,
            declared.version,
            declared.effect,
            declared.key_window,
        )?;
        let key_part = work.key_part();
        if let Some(part) = &key_part {
            check_key_part(part)?;
        }
        let plain = |key_part: Option<String>| -> Result<Route, OperationError> {
            let local_key = key_part
                .map(|part| local_idempotency_key(key, declared.name, declared.version, &part))
                .transpose()?;
            Ok(Route::Plain { local_key })
        };
        if declared.effect == Effect::Read {
            return plain(key_part);
        }
        let (owner, binding) = match &self.effect_authority {
            EffectAuthority::Unjournaled => return plain(key_part),
            EffectAuthority::ReadOnly { detail } => {
                return Err(OperationError::new(ErrorKind::Permanent, detail));
            },
            EffectAuthority::Journaled { owner, binding } => (owner, binding),
        };
        if W::codec().is_none() {
            return Err(OperationError::new(
                ErrorKind::Permanent,
                "streaming effects are not journaled in v1",
            ));
        }
        let recovery = match declared.effect {
            Effect::Idempotent => Recovery::StableKey {
                window: declared.key_window,
            },
            _ => Recovery::Opaque,
        };
        let declaration = JournalDeclaration {
            kind: declared.kind,
            name: declared.name,
            version: declared.version,
            effect: declared.effect,
            recovery,
            record_output: declared.record_output,
            canonical_request: work.canonical_request()?,
            key_part,
        };
        let config_fingerprint = managed.config_fingerprint();
        OwnedEffect::submit(owner, binding, key, config_fingerprint, declaration)
            .map(|owned| Route::Owned(Box::new(owned)))
    }
}

/// How a unit runs, as [`UnitScope::admit`] routed it.
enum Route {
    /// Without an owner, presenting `local_key` when it declared a key part.
    Plain { local_key: Option<IdempotencyKey> },
    /// Driven through the row's owner.
    Owned(Box<OwnedEffect>),
}

/// Builds the lazy [`Submission`] of `work` on `host`, in a
/// `nebula.resource.unit` span naming its operation key (or session name),
/// under `scope`. A unit routed to the facade's owner is driven through it
/// (see the `owned` module).
pub(super) fn submit_unit<R, W>(
    host: UnitHost<R>,
    scope: &UnitScope,
    work: W,
) -> Submission<W::Output>
where
    R: Provider + PinSlots,
    W: UnitWork<R>,
{
    let key = host.key().clone();
    let declared = work.declared();
    let effect = declared.effect;
    let span = tracing::info_span!(
        "nebula.resource.unit",
        key = %key,
        operation = declared.name,
        occurrence = tracing::field::Empty,
        attempts = tracing::field::Empty,
        sent = tracing::field::Empty,
        outcome = tracing::field::Empty,
    );
    let (shared, refused) = match scope.admit(&key, host.managed(), &work) {
        Ok(Route::Owned(owned)) => (UnitShared::new(scope).with_effect(*owned), None),
        Ok(Route::Plain { local_key }) => (UnitShared::new(scope).with_local_key(local_key), None),
        Err(refusal) => (UnitShared::new(scope), Some(refusal)),
    };
    let shared = Arc::new(shared);
    let run: Pin<Box<dyn Future<Output = Result<W::Output, OperationError>> + Send>> = match refused
    {
        None => Box::pin(
            start_unit::<R, W>(host, Arc::clone(&shared), work, effect, span.clone())
                .instrument(span),
        ),
        Some(refusal) => {
            let denied_shared = Arc::clone(&shared);
            let denied_span = span.clone();
            Box::pin(
                async move {
                    // Keep the operation lazy and let cancellation win while
                    // the unit is still pending, exactly as it does before a
                    // normal unit's first grant.
                    let _work = work;
                    let result = match denied_shared.refuse_if_cancelled() {
                        Ok(()) => {
                            tracing::warn!(
                                target: "nebula.resource",
                                parent: &denied_span,
                                effect = effect.as_str(),
                                detail = refusal.detail(),
                                "managed row refused a unit at submit"
                            );
                            Err(refusal.settled(SentState::NotSent, effect, host.key()))
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
        },
    };
    Submission { shared, key, run }
}

/// State one unit shares between its handle, its runtime task and its
/// attempts.
pub(super) struct UnitShared {
    /// `PENDING` until the first grant or a cancel, whichever comes first.
    state: AtomicU8,
    /// Fired by [`Submission::cancel`] while no attempt was granted, or by the
    /// parent cancellation of the unit's [`UnitScope`] (a child token): the
    /// latter is honoured only until the first grant.
    cancel: CancellationToken,
    deadline: Mutex<tokio::time::Instant>,
    /// Woken when an owner's grant shrinks the deadline of a running unit.
    deadline_shrunk: tokio::sync::Notify,
    /// Attempts granted so far.
    granted: AtomicU32,
    /// Worst settled [`SentState`] rank across granted attempts, a
    /// throttled attempt's aside: the provider applied nothing.
    worst: AtomicU8,
    /// The latest settled attempt's [`SentState`] rank, throttled or not;
    /// `NotSent` once an execution owner refused an attempt after it.
    last: AtomicU8,
    /// The unit's owned effect, for an `Idempotent` or `Write` unit on a
    /// journaled row facade.
    effect: Option<OwnedEffect>,
    /// The local idempotency key of a plain unit that declared a
    /// developer key part.
    local_key: Option<IdempotencyKey>,
    /// The throttle of a finished attempt while it is reported to the rate
    /// limit: a unit deadline that cuts the report off settles the unit
    /// with it, not as ended abnormally. Cleared by the report's end and by
    /// the next grant.
    reporting: Mutex<Option<OperationError>>,
}

impl UnitShared {
    /// A pending unit under `scope`: its cancel a child of the parent's, its
    /// deadline the cap from now or the parent's, whichever is earlier.
    pub(super) fn new(scope: &UnitScope) -> Self {
        let now = tokio::time::Instant::now().into_std();
        let capped = crate::deadline::deadline_after(
            now,
            OPERATION_DEADLINE_CAP,
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
            deadline_shrunk: tokio::sync::Notify::new(),
            granted: AtomicU32::new(0),
            worst: AtomicU8::new(SentState::NotSent.rank()),
            last: AtomicU8::new(SentState::NotSent.rank()),
            effect: None,
            local_key: None,
            reporting: Mutex::new(None),
        }
    }

    /// The unit driven through its row's owner as `effect`.
    fn with_effect(mut self, effect: OwnedEffect) -> Self {
        self.effect = Some(effect);
        self
    }

    /// The plain unit presenting `local_key`.
    fn with_local_key(mut self, local_key: Option<IdempotencyKey>) -> Self {
        self.local_key = local_key;
        self
    }

    /// The provider idempotency key the unit presents: its owner's, or its
    /// local one.
    pub(super) fn idempotency_key(&self) -> Option<&IdempotencyKey> {
        self.effect
            .as_ref()
            .and_then(OwnedEffect::slot)
            .map(JournalSlot::idempotency_key)
            .or(self.local_key.as_ref())
    }

    /// The unit's owned effect, if it has one.
    pub(super) fn effect(&self) -> Option<&OwnedEffect> {
        self.effect.as_ref()
    }

    /// The unit's cancel: [`Submission::cancel`] or its parent's.
    pub(super) fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }

    fn deadline(&self) -> tokio::time::Instant {
        *self.deadline.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Shrinks the deadline of the running unit to `deadline` (never
    /// extends it): the unit is stopped there, as at its own deadline.
    pub(super) fn shrink_deadline(&self, deadline: tokio::time::Instant) {
        {
            let mut current = self.deadline.lock().unwrap_or_else(PoisonError::into_inner);
            if deadline >= *current {
                return;
            }
            *current = deadline;
        }
        // One watcher per unit: a stored permit wakes it even when it is
        // not waiting yet.
        self.deadline_shrunk.notify_one();
    }

    /// Runs `run` until the unit's deadline — re-read whenever a grant
    /// shrinks it; `None` when the deadline elapsed first.
    ///
    /// Takes the operation pinned by the caller: an `async fn` that moved
    /// its argument into a pinned local would hold the whole operation
    /// future twice (argument and local), doubling the unit task
    /// (<https://github.com/rust-lang/rust/issues/62958>).
    async fn run_until_deadline<F: Future>(&self, mut run: Pin<&mut F>) -> Option<F::Output> {
        loop {
            let deadline = self.deadline();
            tokio::select! {
                biased;
                output = &mut run => return Some(output),
                () = tokio::time::sleep_until(deadline) => {
                    if self.deadline() <= tokio::time::Instant::now() {
                        return None;
                    }
                },
                () = self.deadline_shrunk.notified() => {},
            }
        }
    }

    fn is_granted(&self) -> bool {
        self.state.load(Ordering::Acquire) == GRANTED
    }

    /// The cancel a wait of the unit races: [`Submission::cancel`] until the
    /// first grant, nothing after it.
    pub(super) fn cancel_before_grant(&self) -> Option<&CancellationToken> {
        (!self.is_granted()).then_some(&self.cancel)
    }

    /// Refuses a unit whose cancel fired while no attempt was granted —
    /// [`Submission::cancel`] or the parent cancellation — and latches it
    /// cancelled. A fired cancel is ignored once an attempt was granted.
    pub(super) fn refuse_if_cancelled(&self) -> Result<(), OperationError> {
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
    /// by [`Submission::cancel`], or by the parent cancellation having fired by
    /// now. The grant and a parent cancel race here, never after: a parent
    /// cancel that fires once the first attempt was granted is ignored.
    pub(super) fn grant(&self) -> Result<(), OperationError> {
        self.refuse_if_cancelled()?;
        match self
            .state
            .compare_exchange(PENDING, GRANTED, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) | Err(GRANTED) => {
                self.set_reporting(None);
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

    /// A granted attempt settled `sent`, as `note` says; an owned effect's
    /// pending call settles with it.
    fn record(&self, sent: SentState, note: CallNote) {
        self.last.store(sent.rank(), Ordering::Release);
        if note != CallNote::Throttled {
            self.worst.fetch_max(sent.rank(), Ordering::AcqRel);
        }
        if let Some(effect) = &self.effect {
            effect.record(sent, note);
        }
    }

    pub(super) fn attempts(&self) -> u32 {
        self.granted.load(Ordering::Acquire)
    }

    /// An execution owner refused the attempt after the latest settled one:
    /// its refusal, not that attempt, is now the unit's last word, so a
    /// throttle it followed no longer counts — the provider applied nothing.
    /// An attempt that may have crossed still counts through `worst`.
    fn owner_refused_next(&self) {
        self.last
            .store(SentState::NotSent.rank(), Ordering::Release);
    }

    fn set_reporting(&self, throttle: Option<OperationError>) {
        *self
            .reporting
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = throttle;
    }

    /// The throttle of a finished attempt whose report the unit deadline
    /// cut off, if that is where the unit was.
    fn take_reporting(&self) -> Option<OperationError> {
        self.reporting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// The unit's sent state: `NotSent` when no attempt was granted, however
    /// the author settled; otherwise the worst settled attempt — a throttled
    /// attempt only counts when it was the last and no execution owner
    /// refused an attempt after it, as the provider applied nothing —
    /// raised to `MaybeSent` when the unit ended abnormally (deadline,
    /// panic).
    fn fold(&self, abnormal: bool) -> SentState {
        if self.attempts() == 0 {
            return SentState::NotSent;
        }
        if abnormal {
            return SentState::MaybeSent;
        }
        let worst = self.worst.load(Ordering::Acquire);
        SentState::from_rank(worst.max(self.last.load(Ordering::Acquire)))
    }

    fn state_name(&self) -> &'static str {
        match self.state.load(Ordering::Acquire) {
            PENDING => "pending",
            GRANTED => "granted",
            _ => "cancelled",
        }
    }
}

pub(super) fn cancelled_before_grant() -> OperationError {
    OperationError::new(
        ErrorKind::Cancelled,
        "unit cancelled before its first attempt",
    )
}

/// One submitted [`Operation`]: a future of its settled outcome.
///
/// Lazy until first polled (see [`Lease::submit`]). Once started the
/// runtime owns the unit: dropping this handle stops waiting but never
/// aborts the operation — the runtime settles it, and the lease is released
/// only after it ends.
#[must_use = "a unit does nothing until awaited; dropped before its first poll it never runs"]
pub struct Submission<T> {
    shared: Arc<UnitShared>,
    key: ResourceKey,
    run: Pin<Box<dyn Future<Output = Result<T, OperationError>> + Send>>,
}

impl<T> Submission<T> {
    /// Shortens the unit's deadline to `deadline` (it can never exceed
    /// [`OPERATION_DEADLINE_CAP`] from submission). Call it before the first
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

impl<T> Future for Submission<T> {
    type Output = Result<T, OperationError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().run.as_mut().poll(cx)
    }
}

impl<T> fmt::Debug for Submission<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Submission")
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
) -> Result<OwnedSemaphorePermit, OperationError> {
    tokio::select! {
        biased;
        () = shared.cancel.cancelled() => Err(cancelled_before_grant()),
        () = lease.generation.token().cancelled() => {
            Err(lease.admission_refusal().err().unwrap_or_else(|| {
                OperationError::new(ErrorKind::Cancelled, "lease closing; new attempts refused")
            }))
        },
        permit = Arc::clone(&lease.units).acquire_owned() => permit.map_err(|_closed| {
            OperationError::new(ErrorKind::Cancelled, "lease closing; new attempts refused")
        }),
        () = tokio::time::sleep_until(deadline) => Err(OperationError::new(
            ErrorKind::Backpressure,
            "the lease's unit slots stayed full until the unit deadline",
        )),
    }
}

/// The caller's side of a unit: waits for a unit slot on a lease host,
/// prepares an owned unit's effect, then spawns the runtime task and waits
/// for its outcome. `effect` is what the unit's settled error reports.
async fn start_unit<R, W>(
    host: UnitHost<R>,
    shared: Arc<UnitShared>,
    work: W,
    effect: Effect,
    span: tracing::Span,
) -> Result<W::Output, OperationError>
where
    R: Provider + PinSlots,
    W: UnitWork<R>,
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
        UnitHost::Row(row) if super::session::in_session_of(row.marker()) => {
            Err(OperationError::new(
                ErrorKind::Permanent,
                "a unit of a row awaited inside a session of the same row; refused",
            ))
        },
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
    // An owned unit's first poll: the owner prepares its effect before
    // anything is spawned, booked, read or checked out.
    let codec = match (W::codec(), shared.effect()) {
        (Some(codec), Some(owned)) => {
            let max_invocations = work.max_attempts();
            let prepared = owned::prepare(&shared, owned, max_invocations, codec, deadline).await;
            // The position is assigned when preparing begins.
            span.record("occurrence", owned.occurrence());
            match prepared {
                Prepared::Run => Some(codec),
                Prepared::Replayed(output) => {
                    host.record_replayed(&span);
                    return Ok(output);
                },
                Prepared::Refused(refusal, sent) => {
                    let result = Err(refusal.settled(sent, effect, host.key()));
                    host.record_settled(&span, &result, sent, 0);
                    return result;
                },
            }
        },
        _ => None,
    };
    let key = host.key().clone();
    let runtime = tokio::spawn(
        run_unit::<R, W>(
            host,
            Arc::clone(&shared),
            work,
            (effect, codec),
            permit,
            deadline,
            span.clone(),
        )
        .instrument(span),
    );
    match runtime.await {
        Ok(outcome) => outcome,
        // Only a runtime shutdown aborts the task; the unit never settled.
        Err(_aborted) => Err(OperationError::new(
            ErrorKind::Cancelled,
            "the runtime stopped before the unit settled",
        )
        .settled(shared.fold(true), effect, &key)),
    }
}

/// The runtime's side of a unit: runs the operation under the deadline and
/// settles the outcome whether or not anyone waits; an owned unit's last
/// call is recorded with its owner (through `codec`) before the unit
/// settles. The slots are pinned at the unit's first grant, not here.
async fn run_unit<R, W>(
    host: UnitHost<R>,
    shared: Arc<UnitShared>,
    work: W,
    (effect, codec): (Effect, Option<OutputCodec<W::Output>>),
    _permit: Option<OwnedSemaphorePermit>,
    deadline: tokio::time::Instant,
    span: tracing::Span,
) -> Result<W::Output, OperationError>
where
    R: Provider + PinSlots,
    W: UnitWork<R>,
{
    let generation = match host.unit_generation() {
        Ok(generation) => generation,
        Err(refusal) => {
            let result = Err(refusal.settled(SentState::NotSent, effect, host.key()));
            host.record_settled(&span, &result, SentState::NotSent, 0);
            return result;
        },
    };
    let max_attempts = work.max_attempts();
    let outcome = {
        let mut cx = OperationCx {
            host: &host,
            generation: &generation,
            pin: None,
            shared: &shared,
            deadline,
            max_attempts,
            effect,
        };
        // Bounded by the unit's deadline. Only an owner's grant shrinks it
        // while the operation runs, so an unowned unit keeps the plain timer.
        // The operation future is pinned once here and both arms borrow it,
        // so the unit task holds it once: passing it by value into an
        // `async fn` that pins it again stores it twice in the state machine
        // (https://github.com/rust-lang/rust/issues/62958), doubling the
        // task and the bytes copied when it is spawned.
        let run = async {
            let operation = std::pin::pin!(work.run(&mut cx));
            if shared.effect().is_some() {
                shared.run_until_deadline(operation).await
            } else {
                tokio::time::timeout_at(deadline, operation).await.ok()
            }
        };
        AssertUnwindSafe(run).catch_unwind().await
    };
    let (result, abnormal) = match outcome {
        Ok(Some(result)) => (result, false),
        // The deadline cut off the report of a finished throttle: the
        // provider applied nothing, and the unit settles as the throttle.
        Ok(None) => match shared.take_reporting() {
            Some(throttle) => (Err(throttle), false),
            None => (
                Err(OperationError::new(
                    ErrorKind::Transient,
                    "unit deadline elapsed",
                )),
                true,
            ),
        },
        Err(_panic) => {
            let kind = if shared.attempts() == 0 {
                ErrorKind::Permanent
            } else {
                ErrorKind::Transient
            };
            (Err(OperationError::new(kind, "operation panicked")), true)
        },
    };
    let sent = shared.fold(abnormal);
    let result = match (shared.effect(), codec) {
        (Some(owned), Some(codec)) => owned.finish(result, abnormal, codec).await,
        _ => result,
    };
    let result = result.map_err(|error| error.settled(sent, effect, host.key()));
    host.record_settled(&span, &result, sent, shared.attempts());
    result
}

/// What an [`Operation`] runs against: the unit's deadline, its attempt
/// budget and the lease's closing notice, and the only way to reach the
/// provider — [`attempt`](Self::attempt).
pub struct OperationCx<'u, R: Provider + PinSlots> {
    pub(super) host: &'u UnitHost<R>,
    /// The admission generation the unit is refused under: the lease's, or
    /// the row's current one when the unit started.
    pub(super) generation: &'u Arc<AdmissionGeneration>,
    /// The unit's slots, pinned at its first grant.
    pub(super) pin: Option<UnitPin<R::Pinned>>,
    pub(super) shared: &'u UnitShared,
    pub(super) deadline: tokio::time::Instant,
    max_attempts: NonZeroU32,
    /// The unit's declared effect: whether [`call`](Self::call) may send
    /// an attempt that may have been applied again.
    effect: Effect,
}

impl<R: Provider + PinSlots> fmt::Debug for OperationCx<'_, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationCx")
            .field("resource_key", self.host.key())
            .field("attempts", &self.shared.attempts())
            .field("max_attempts", &self.max_attempts)
            .finish_non_exhaustive()
    }
}

impl<R: Provider + PinSlots> OperationCx<'_, R> {
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
    ///    raced against the lease's generation and [`Submission::cancel`]. Every
    ///    attempt reads, [`Cost::FREE`] included; concurrent attempts of a
    ///    credential share reads join-next (only a read issued after the
    ///    attempt arrived answers it). An interim manager or a slot-less row
    ///    reads nothing.
    /// 5. **Pin** — the unit's first grant pins its credential slots
    ///    ([`Attempt::credentials`]).
    /// 6. **Registration and grant** — on a strict read, under the manager's
    ///    admission lock: a revoke taint is `Revoked`, shutdown `Cancelled`;
    ///    the read is applied to the row (a blocked credential suspends it
    ///    and closes its leases, a usable one reopens or readmits it) and a
    ///    denying credential refuses `CredentialUnavailable` with the read's
    ///    reason; a suspended row or a closed lease refuses; a pin a rotation
    ///    superseded since the unit pinned it refuses `Rebinding`. Without a
    ///    strict read the lease's admission is re-checked, lock-free. The
    ///    attempt is then granted unless [`Submission::cancel`] won the race for
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
    pub async fn attempt(&mut self, cost: Cost) -> Result<Attempt<'_, R>, OperationError> {
        let (host, shared) = (self.host, self.shared);
        let admitted = self.admit(&cost).await;
        host.record_attempt(admitted.is_ok());
        if let Err(refusal) = &admitted
            && refusal.supersedes_retried()
        {
            shared.owner_refused_next();
        }
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

    /// Makes one provider call costing `cost` per attempt: `f` runs on each
    /// granted attempt's instance and the unit's pinned credential slots,
    /// and returns the call's result, its error classified with an
    /// [`OperationError`] constructor
    /// ([`throttled`](OperationError::throttled),
    /// [`unreachable`](OperationError::unreachable),
    /// [`interrupted`](OperationError::interrupted),
    /// [`rejected`](OperationError::rejected), …). Each attempt is
    /// [`finish`](Attempt::finish)ed from that result: its sent state, the
    /// rate limit's verdict and an execution journal's record all derive
    /// from it (see [`OperationError`] for the table).
    ///
    /// A failed attempt is taken again while the unit's attempt budget
    /// ([`Operation::max_attempts`], one by default: no hidden retry) and
    /// deadline allow, when the error says it is safe: a throttle or an
    /// unreachable provider always, an interrupted call only for a
    /// replay-safe [`Operation::EFFECT`], a rejection never — an
    /// interrupted `Write` is never sent twice. Nothing sleeps between
    /// attempts: a throttle's pause is waited out by the next attempt's
    /// quota booking, and a pause that lands past the deadline ends the
    /// call.
    ///
    /// The closure owns what it captures (write `async move`): each attempt
    /// borrows the captures from it, but a closure that borrows from the
    /// operation cannot prove its future `Send` (an async closure limit), so
    /// the bound asks for `'static`. Move the operation's fields in, and
    /// read [`idempotency_key`](Self::idempotency_key) before the call.
    ///
    /// For a call that needs the attempt itself — a stream past its head,
    /// several requests on one checkout — use [`attempt`](Self::attempt) and
    /// [`Attempt::finish`].
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use nebula_resource::{
    ///     PinSlots, Provider,
    ///     call::{Cost, Effect, Operation, OperationCx, OperationError},
    /// };
    /// use serde::{Deserialize, Serialize};
    ///
    /// /// What a client call can answer.
    /// enum Reply {
    ///     Value(u64),
    ///     SlowDown(Option<Duration>),
    ///     NoRoute,
    ///     Missing,
    /// }
    ///
    /// fn classify(reply: Reply) -> Result<u64, OperationError> {
    ///     match reply {
    ///         Reply::Value(value) => Ok(value),
    ///         Reply::SlowDown(after) => Err(OperationError::throttled(after)),
    ///         Reply::NoRoute => Err(OperationError::unreachable("no route to the provider")),
    ///         Reply::Missing => Err(OperationError::rejected("no such counter")),
    ///     }
    /// }
    ///
    /// #[derive(Serialize, Deserialize)]
    /// struct ReadCounter {
    ///     offset: u64,
    /// }
    ///
    /// impl<R> Operation<R> for ReadCounter
    /// where
    ///     R: Provider<Instance = u64> + PinSlots,
    /// {
    ///     type Output = u64;
    ///     const KEY: &'static str = "counter.read";
    ///     const EFFECT: Effect = Effect::Read;
    ///
    ///     fn max_attempts(&self) -> std::num::NonZeroU32 {
    ///         std::num::NonZeroU32::new(3).unwrap_or(std::num::NonZeroU32::MIN)
    ///     }
    ///
    ///     async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<u64, OperationError> {
    ///         let offset = self.offset;
    ///         cx.call(Cost::ONE, async move |counter, _credentials| {
    ///             classify(Reply::Value(*counter + offset))
    ///         })
    ///         .await
    ///     }
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// The first attempt's refusal (see [`attempt`](Self::attempt)); after
    /// that, the last attempt's error — a retry the runtime refused (the
    /// budget, the deadline, the lease closing) returns the error of the
    /// attempt it would have retried, but a retry an execution owner refused
    /// (an unknown outcome, a closed owner, a mismatch) returns that
    /// refusal: it is authoritative.
    pub async fn call<T, F>(&mut self, cost: Cost, f: F) -> Result<T, OperationError>
    where
        F: AsyncFnMut(&R::Instance, &R::Pinned) -> Result<T, OperationError> + Send + 'static,
    {
        let mut f = f;
        let effect = self.effect;
        let mut previous: Option<OperationError> = None;
        loop {
            if let Some(error) = previous.take_if(|_| self.attempts() >= self.max_attempts.get()) {
                return Err(error);
            }
            let attempt = match self.attempt(cost.clone()).await {
                Ok(attempt) => attempt,
                // A local refusal says nothing new: the attempt it would have
                // retried explains the call. An owner's refusal is final.
                Err(refusal) => {
                    return Err(match previous {
                        Some(error) if !refusal.supersedes_retried() => error,
                        _ => refusal,
                    });
                },
            };
            let result = f(attempt.instance(), attempt.credentials()).await;
            attempt.finish(&result).await;
            match result {
                Ok(output) => return Ok(output),
                Err(error) if error.retried_in_call(effect) => previous = Some(error),
                Err(error) => return Err(error),
            }
        }
    }

    /// A session's single attempt on a row host: admitted like a row
    /// attempt, on a checkout that fits the provider's
    /// [`SessionBinding`], and destroyed on release unless the session
    /// closed cleanly ([`Attempt::end_session`]).
    pub(super) async fn attempt_session(
        &mut self,
        cost: Cost,
    ) -> Result<Attempt<'_, R>, OperationError>
    where
        R: SessionProvider + PoolProvider + Provider<Topology = Pooled<R>> + Clone,
    {
        let UnitHost::Row(row) = self.host else {
            return Err(OperationError::new(
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
    ) -> Result<(AttemptTarget<'_, R>, &UnitPin<R::Pinned>), OperationError> {
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
    ) -> Result<(), OperationError> {
        if self.shared.attempts() >= self.max_attempts.get() {
            return Err(OperationError::new(
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

    /// The provider idempotency key to send, the same for every attempt,
    /// retry and resume of the unit:
    ///
    /// - on a journaled row, for an `Idempotent` or `Write` unit, the key
    ///   its owner derived and recorded before the first attempt;
    /// - otherwise, for an operation that declared a developer key part
    ///   ([`Operation::idempotency_key`]), a key derived locally from the
    ///   resource, the operation key and version, and the part;
    /// - `None` for any other unit.
    #[must_use]
    pub fn idempotency_key(&self) -> Option<&IdempotencyKey> {
        self.shared.idempotency_key()
    }
}

/// The refusal of an attempt whose quota booking failed.
fn quota_refusal(error: &Error) -> OperationError {
    let detail = match error.kind() {
        ErrorKind::Cancelled => "unit cancelled before its first attempt",
        ErrorKind::Exhausted { .. } => "rate limit slot lands past the unit deadline",
        ErrorKind::Backpressure => "rate limit store unavailable; attempt refused",
        ErrorKind::Permanent => "rate limit can never grant the attempt's cost",
        _ => "rate limit refused the attempt",
    };
    OperationError::new(error.kind().clone(), detail)
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
    /// The instance the attempt runs on: the lease's for a [`Lease`]
    /// facade, the attempt's own checkout for a
    /// [`ResourceHandle`](super::ResourceHandle).
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
    pub fn credentials(&self) -> &R::Pinned {
        self.pinned
    }

    /// The unit's provider idempotency key, as
    /// [`OperationCx::idempotency_key`]: the same for every attempt. For a
    /// helper that sends a request on a granted attempt.
    #[must_use]
    pub fn idempotency_key(&self) -> Option<&IdempotencyKey> {
        self.shared.idempotency_key()
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

    /// Finishes the attempt from its call's `result`: the low-level side of
    /// [`OperationCx::call`], for an operation that holds the attempt
    /// itself (a stream finished at its head, several steps on one
    /// checkout). Everything derives from the result, classified by its
    /// [`OperationError`] constructor (see its table):
    ///
    /// - a success, or a [`rejected`](OperationError::rejected) call, was
    ///   `Sent` and tells the rate limit the call passed, resetting its
    ///   backoff;
    /// - a [`throttled`](OperationError::throttled) call was `Sent` and
    ///   pauses the quota for the provider's hint, a
    ///   [`throttled_key`](OperationError::throttled_key) one only the
    ///   attempt's [`Cost::keyed`] key; neither counts as a call that may
    ///   have been applied;
    /// - an [`unreachable`](OperationError::unreachable) call was `NotSent`,
    ///   an [`interrupted`](OperationError::interrupted) or unclassified one
    ///   `MaybeSent`; neither tells the rate limit anything.
    ///
    /// Telling the rate limit is bounded and never fails the unit. An
    /// attempt dropped unfinished counts as `MaybeSent`.
    pub async fn finish<T>(self, result: &Result<T, OperationError>) {
        let mut attempt = self;
        let (sent, note, verdict) = match result {
            Ok(_) => (SentState::Sent, CallNote::Applied, Some(Verdict::Pass)),
            Err(error) => {
                let sent = error.attempt_sent();
                match error.signal() {
                    Signal::Throttled { per_key } => {
                        let retry_after = match error.kind() {
                            ErrorKind::Exhausted { retry_after } => *retry_after,
                            _ => None,
                        };
                        let verdict = if per_key {
                            Verdict::KeyThrottled { retry_after }
                        } else {
                            Verdict::Throttled { retry_after }
                        };
                        (sent, CallNote::Throttled, Some(verdict))
                    },
                    Signal::Rejected => (
                        sent,
                        CallNote::Rejected(ErrorKindCode::of(error.kind())),
                        Some(Verdict::Pass),
                    ),
                    Signal::Unreachable | Signal::Interrupted | Signal::Unclassified => {
                        (sent, CallNote::Plain, None)
                    },
                }
            },
        };
        // Recorded before the verdict's bounded wait: the attempt is over
        // even when its future is dropped mid-report, and a throttle whose
        // report the unit deadline cuts off still settles the unit as one.
        attempt.shared.record(sent, note);
        attempt.settled = true;
        if let Some(verdict) = verdict {
            let throttle = result
                .as_ref()
                .err()
                .filter(|_| note == CallNote::Throttled);
            attempt.shared.set_reporting(throttle.cloned());
            attempt
                .managed
                .rate_limiter
                .report(verdict, attempt.cost.key())
                .await;
            attempt.shared.set_reporting(None);
        }
    }

    /// Settles the attempt with what happened to the request, telling the
    /// rate limit nothing: a session's single attempt.
    pub(crate) fn settle(self, sent: SentState) {
        let mut attempt = self;
        // A session settled `Sent` committed: the provider applied it,
        // whatever the body returned.
        let note = if sent == SentState::Sent {
            CallNote::Applied
        } else {
            CallNote::Plain
        };
        attempt.shared.record(sent, note);
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
            self.shared.record(SentState::MaybeSent, CallNote::Plain);
        }
    }
}
