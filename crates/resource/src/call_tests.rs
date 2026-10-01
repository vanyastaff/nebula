//! Managed call facade: attempt admission and booking, the settled outcome
//! of a unit, cancellation, deadlines, the per-lease unit cap, pinned slots
//! and the unit's observability.

use std::{
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_core::{ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_metrics::MetricsRegistry;
use tokio::{sync::Notify, time::Instant};
use tokio_util::sync::CancellationToken;

use super::{
    Attempt, Cost, Effect, Lease, OPERATION_DEADLINE_CAP, Operation, OperationCx, OperationError,
    PinSlots, SentState, Submission,
};
use crate::{
    AcquireOptions, CredentialUnavailableReason, Error, ErrorKind, Manager, ManagerConfig,
    PoolConfig, Pooled, Provider, RateLimitProfile, RegistrationSpec, Resident, ResidentConfig,
    ResourceConfig, ResourceContext, ResourceEvent, ResourceGuard, SlotCell, SlotIdentity,
    rate_limit::{Rate, ResiliencePolicy, RowLimit},
    resource::{HasCredentialSlots, ResourceMetadataDraft},
    runtime::managed::ManagedResource,
    topology::{
        PoolProvider, ResidentProvider,
        pooled::{RecycleDecision, config::WarmupStrategy},
    },
};

// ── fixtures ─────────────────────────────────────────────────────────────

#[derive(Clone, nebula_schema::Schema)]
struct Config {
    version: u64,
}

impl ResourceConfig for Config {
    fn fingerprint(&self) -> u64 {
        self.version
    }
}

fn per_second(requests: u32, burst: u32) -> Rate {
    Rate::per_second(NonZeroU32::new(requests).expect("non-zero"))
        .with_burst(NonZeroU32::new(burst).expect("non-zero"))
        .expect("valid rate")
}

/// A provider per topology with one `db` credential slot pinned per unit
/// and a one-per-second `chat_id` key limit.
macro_rules! facade_row {
    ($ty:ident, $key:literal, $topology:ty) => {
        #[derive(Clone)]
        struct $ty {
            token: Arc<SlotCell<String>>,
            creates: Arc<AtomicU64>,
        }

        impl $ty {
            fn new() -> Self {
                Self {
                    token: Arc::new(SlotCell::empty()),
                    creates: Arc::default(),
                }
            }
        }

        #[async_trait::async_trait]
        impl Provider for $ty {
            type Config = Config;
            type Instance = u64;
            type Topology = $topology;

            fn key() -> ResourceKey {
                resource_key!($key)
            }

            fn metadata() -> ResourceMetadataDraft {
                ResourceMetadataDraft::new(Self::key(), crate::metadata_name!($key), "")
            }

            fn resilience() -> ResiliencePolicy {
                ResiliencePolicy::new().keyed("chat_id", per_second(1, 1))
            }

            async fn create(&self, _: &Config, _: &ResourceContext) -> Result<u64, Error> {
                Ok(self.creates.fetch_add(1, Ordering::SeqCst))
            }
        }

        impl HasCredentialSlots for $ty {
            fn credential_slot_epoch(&self) -> u64 {
                self.token.generation()
            }
            fn declares_credential_slots() -> bool {
                true
            }
            fn credential_slot_names() -> &'static [&'static str] {
                &["db"]
            }
        }

        impl PinSlots for $ty {
            type Pinned = Option<Arc<String>>;

            fn pin_slots(&self) -> Self::Pinned {
                self.token.load()
            }
        }
    };
}

facade_row!(Api, "call-resident", Resident<Self>);
facade_row!(PooledApi, "call-pooled", Pooled<Self>);

impl ResidentProvider for Api {}

impl PoolProvider for PooledApi {
    async fn recycle(&self, _: &u64, _: &crate::InstanceMetrics) -> Result<RecycleDecision, Error> {
        Ok(RecycleDecision::Keep)
    }
}

fn metered_manager() -> Manager {
    Manager::with_config(
        ManagerConfig::default().with_metrics_registry(Arc::new(MetricsRegistry::new())),
    )
}

fn context() -> ResourceContext {
    ResourceContext::minimal(Scope::default(), CancellationToken::new())
}

fn register<R>(manager: &Manager, resource: R, topology: R::Topology, limit: Option<RowLimit>) -> R
where
    R: Provider<Config = Config> + Clone,
{
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: Config { version: 1 },
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology,
            recovery_gate: None,
            rate_limit: limit,
        })
        .expect("register");
    resource
}

fn resident(manager: &Manager, limit: Option<RowLimit>) -> Api {
    register(
        manager,
        Api::new(),
        Resident::new(ResidentConfig::default()),
        limit,
    )
}

fn pooled(manager: &Manager) -> PooledApi {
    let config = PoolConfig {
        min_size: 0,
        max_size: 2,
        idle_timeout: None,
        max_lifetime: None,
        warmup: WarmupStrategy::None,
        maintenance_interval: Duration::from_hours(1),
        ..PoolConfig::default()
    };
    register(
        manager,
        PooledApi::new(),
        Pooled::new(config, Config { version: 1 }.fingerprint()),
        None,
    )
}

async fn acquire<R: Provider>(manager: &Manager) -> ResourceGuard<R> {
    manager
        .acquire::<R>(&context(), &AcquireOptions::default())
        .await
        .expect("acquire")
}

async fn managed<R: Provider + PinSlots>(manager: &Manager) -> Lease<R> {
    acquire::<R>(manager).await.into_lease()
}

fn read_only_row<R: Provider + PinSlots>(
    manager: &Manager,
    ctx: &ResourceContext,
) -> crate::call::ResourceHandle<R> {
    manager
        .handle_any_read_only(
            &R::key(),
            ctx,
            &AcquireOptions::default(),
            &SlotIdentity::Unbound,
        )
        .expect("read-only row")
        .downcast::<crate::call::ResourceHandle<R>>()
        .map(|row| *row)
        .expect("typed read-only row")
}

fn row<R: Provider>(manager: &Manager) -> Arc<ManagedResource<R>> {
    manager
        .lookup_any_for_slot_identity_structural(
            &R::key(),
            &ScopeLevel::Global,
            &SlotIdentity::Unbound,
        )
        .expect("row is registered")
        .as_any_arc()
        .downcast::<ManagedResource<R>>()
        .expect("row type")
}

fn suspend<R: Provider>(manager: &Manager) {
    manager
        .suspend_credential_row(
            &R::key(),
            &ScopeLevel::Global,
            &SlotIdentity::Unbound,
            "db",
            CredentialUnavailableReason::ReauthRequired,
            None,
        )
        .expect("suspend");
}

fn snapshot(manager: &Manager) -> crate::ResourceOpsSnapshot {
    manager.metrics().expect("metrics configured").snapshot()
}

/// Lets every task run until the runtime is idle (paused time only).
async fn settle_tasks() {
    tokio::time::sleep(Duration::from_millis(1)).await;
}

fn in_one(after: Duration) -> std::time::Instant {
    Instant::now().into_std() + after
}

// ── operations ───────────────────────────────────────────────────────────

// The operations here are never journaled: fields that are not intent are
// skipped, and a deserialized operation takes these defaults.
fn free() -> Cost {
    Cost::FREE
}

fn rejected() -> OperationError {
    OperationError::rejected("provider refused")
}

/// Finishes `attempt` as answered: `Sent`, the limit told it passed.
async fn answered<R: Provider + PinSlots>(attempt: Attempt<'_, R>) {
    attempt.finish(&Ok::<(), OperationError>(())).await;
}

/// One answered call at `cost`; yields the attempts granted.
#[derive(serde::Serialize, serde::Deserialize)]
struct Once {
    #[serde(skip, default = "free")]
    cost: Cost,
}

impl Once {
    fn sent(cost: Cost) -> Self {
        Self { cost }
    }
}

impl<R: Provider + PinSlots> Operation<R> for Once {
    type Output = u32;
    const KEY: &'static str = "test.once";

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<u32, OperationError> {
        cx.call(self.cost, async |_, _| Ok(())).await?;
        Ok(cx.attempts())
    }
}

/// One call that the provider answers with `error`.
#[derive(serde::Serialize, serde::Deserialize)]
struct Refused {
    #[serde(skip, default = "rejected")]
    error: OperationError,
}

impl<R: Provider + PinSlots> Operation<R> for Refused {
    type Output = ();
    const KEY: &'static str = "test.refused";

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<(), OperationError> {
        let error = self.error;
        cx.call(Cost::ONE, async move |_, _| Err(error.clone()))
            .await
    }
}

/// A granted attempt parked until `release` fires; it records whether the
/// lease was closing once released.
#[derive(serde::Serialize, serde::Deserialize)]
struct Gated {
    #[serde(skip)]
    entered: Arc<Notify>,
    #[serde(skip)]
    release: Arc<Notify>,
    #[serde(skip)]
    closed_seen: Arc<AtomicUsize>,
}

struct Gate {
    entered: Arc<Notify>,
    release: Arc<Notify>,
    closed_seen: Arc<AtomicUsize>,
}

fn gated() -> (Gated, Gate) {
    let gate = Gate {
        entered: Arc::new(Notify::new()),
        release: Arc::new(Notify::new()),
        closed_seen: Arc::new(AtomicUsize::new(0)),
    };
    (
        Gated {
            entered: Arc::clone(&gate.entered),
            release: Arc::clone(&gate.release),
            closed_seen: Arc::clone(&gate.closed_seen),
        },
        gate,
    )
}

impl<R: Provider + PinSlots> Operation<R> for Gated {
    type Output = ();
    const KEY: &'static str = "test.gated";

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<(), OperationError> {
        let closing = cx.closing();
        let attempt = cx.attempt(Cost::FREE).await?;
        self.entered.notify_one();
        self.release.notified().await;
        if closing.is_closing() {
            self.closed_seen.fetch_add(1, Ordering::SeqCst);
        }
        answered(attempt).await;
        Ok(())
    }
}

/// A granted attempt that never answers: the unit hits its deadline.
#[derive(serde::Serialize, serde::Deserialize)]
struct Hang<const READ: bool>;

impl<R: Provider + PinSlots, const READ: bool> Operation<R> for Hang<READ> {
    type Output = ();
    const KEY: &'static str = "test.hang";
    const EFFECT: Effect = if READ { Effect::Read } else { Effect::Write };

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<(), OperationError> {
        let _attempt = cx.attempt(Cost::FREE).await?;
        std::future::pending::<()>().await;
        Ok(())
    }
}

/// A write whose body records an authority leak if it is ever polled.
#[derive(serde::Serialize, serde::Deserialize)]
struct NeverWrite {
    #[serde(skip)]
    polled: Arc<AtomicUsize>,
}

impl<R: Provider + PinSlots> Operation<R> for NeverWrite {
    type Output = ();
    const KEY: &'static str = "test.never_write";
    const EFFECT: Effect = Effect::Write;

    async fn run(self, _cx: &mut OperationCx<'_, R>) -> Result<(), OperationError> {
        self.polled.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Panics after its attempt was granted and answered.
#[derive(serde::Serialize, serde::Deserialize)]
struct PanicAfterGrant;

impl<R: Provider + PinSlots> Operation<R> for PanicAfterGrant {
    type Output = ();
    const KEY: &'static str = "test.panic_after_grant";

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<(), OperationError> {
        let attempt = cx.attempt(Cost::FREE).await?;
        answered(attempt).await;
        panic!("operation bug after the grant");
    }
}

/// Asks for three attempts with a budget of two.
#[derive(serde::Serialize, serde::Deserialize)]
struct OverBudget;

impl<R: Provider + PinSlots> Operation<R> for OverBudget {
    type Output = ();
    const KEY: &'static str = "test.over_budget";
    const EFFECT: Effect = Effect::Idempotent;

    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::new(2).expect("two")
    }

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<(), OperationError> {
        for _ in 0..3 {
            let attempt = cx.attempt(Cost::FREE).await?;
            answered(attempt).await;
        }
        Ok(())
    }
}

/// Reads the pinned token on two attempts, rotating the live slot between
/// them when asked.
#[derive(serde::Serialize, serde::Deserialize)]
struct PinnedTwice {
    #[serde(skip)]
    rotate: Option<(Arc<SlotCell<String>>, &'static str)>,
}

impl Operation<Api> for PinnedTwice {
    type Output = (Option<String>, Option<String>);
    const KEY: &'static str = "test.pinned_twice";
    const EFFECT: Effect = Effect::Read;

    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::new(2).expect("two")
    }

    async fn run(self, cx: &mut OperationCx<'_, Api>) -> Result<Self::Output, OperationError> {
        let first = cx.attempt(Cost::FREE).await?;
        let seen_first = pinned_token(&first);
        answered(first).await;
        if let Some((cell, next)) = self.rotate {
            cell.store(Arc::new(next.to_owned()));
        }
        let second = cx.attempt(Cost::FREE).await?;
        let seen_second = pinned_token(&second);
        answered(second).await;
        Ok((seen_first, seen_second))
    }
}

fn pinned_token(attempt: &Attempt<'_, Api>) -> Option<String> {
    attempt.credentials().as_deref().cloned()
}

/// Parks after the unit started and before its first attempt, then yields
/// the token its first attempt was pinned on.
#[derive(serde::Serialize, serde::Deserialize)]
struct PinAfterRelease {
    #[serde(skip)]
    entered: Arc<Notify>,
    #[serde(skip)]
    release: Arc<Notify>,
}

impl Operation<Api> for PinAfterRelease {
    type Output = Option<String>;
    const KEY: &'static str = "test.pin_after_release";
    const EFFECT: Effect = Effect::Read;

    async fn run(self, cx: &mut OperationCx<'_, Api>) -> Result<Self::Output, OperationError> {
        self.entered.notify_one();
        self.release.notified().await;
        let attempt = cx.attempt(Cost::FREE).await?;
        let token = pinned_token(&attempt);
        answered(attempt).await;
        Ok(token)
    }
}

// ── compile gates ────────────────────────────────────────────────────────

#[test]
fn the_facade_and_its_units_cross_threads() {
    fn send_sync_clone<T: Send + Sync + Clone>() {}
    fn send<T: Send + Unpin>() {}
    fn gates<R: Provider + PinSlots, O: Operation<R>>() {
        send_sync_clone::<Lease<R>>();
        send::<Submission<O::Output>>();
        send::<OperationError>();
    }
    gates::<Api, Once>();
    gates::<PooledApi, Gated>();

    // An operation's future is `Send` for any borrowed context.
    fn run_is_send<'a, R: Provider + PinSlots, O: Operation<R>>(
        operation: O,
        cx: &'a mut OperationCx<'a, R>,
    ) -> impl Future<Output = Result<O::Output, OperationError>> + Send + 'a {
        operation.run(cx)
    }
    let _ = run_is_send::<Api, Once>;
}

// ── profile and booking ──────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn into_lease_latches_per_attempt_and_acquires_stop_booking() {
    let manager = Manager::new();
    resident(&manager, Some(RowLimit::rate(per_second(1, 2))));
    let health = manager
        .health_check::<Api>(&ScopeLevel::Global)
        .expect("row");
    assert_eq!(health.rate_limit_profile, RateLimitProfile::PerAcquire);

    let facade = managed::<Api>(&manager).await;
    let health = manager
        .health_check::<Api>(&ScopeLevel::Global)
        .expect("row");
    assert_eq!(health.rate_limit_profile, RateLimitProfile::PerAttempt);

    let started = Instant::now();
    for _ in 0..3 {
        drop(acquire::<Api>(&manager).await);
    }
    assert_eq!(
        started.elapsed(),
        Duration::ZERO,
        "acquires after the latch book nothing"
    );
    facade
        .submit(Once::sent(Cost::ONE))
        .await
        .expect("the permit the first acquire left");
    assert_eq!(started.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn attempts_book_their_cost_and_free_books_nothing() {
    let manager = Manager::new();
    // The first acquire books one permit before the latch; three remain.
    resident(&manager, Some(RowLimit::rate(per_second(1, 4))));
    let facade = managed::<Api>(&manager).await;

    let started = Instant::now();
    let three = Cost::units(NonZeroU32::new(3).expect("three"));
    facade
        .submit(Once::sent(three))
        .await
        .expect("fits the burst");
    assert_eq!(started.elapsed(), Duration::ZERO);

    facade
        .submit(Once::sent(Cost::FREE))
        .await
        .expect("free passes a drained quota");
    assert_eq!(started.elapsed(), Duration::ZERO, "free never waits");

    facade
        .submit(Once::sent(Cost::ONE))
        .await
        .expect("one permit, a second later");
    assert_eq!(started.elapsed(), Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn a_cost_above_the_burst_is_refused_permanently_and_not_sent() {
    let manager = metered_manager();
    resident(&manager, Some(RowLimit::rate(per_second(1, 2))));
    let facade = managed::<Api>(&manager).await;

    let error = facade
        .submit(Once::sent(Cost::units(NonZeroU32::new(5).expect("five"))))
        .await
        .expect_err("5 permits exceed a burst of 2");
    assert_eq!(*error.kind(), ErrorKind::Permanent);
    assert_eq!(error.sent(), SentState::NotSent);
    assert!(!error.is_retryable());
    assert_eq!(error.resource_key(), Some(&Api::key()));
    let metrics = snapshot(&manager);
    assert_eq!(metrics.call_attempts.refused, 1);
    assert_eq!(metrics.call_attempts.granted, 0);
    assert_eq!(metrics.call_units.not_sent, 1);
}

#[tokio::test(start_paused = true)]
async fn a_keyed_cost_books_its_key() {
    let manager = Manager::new();
    resident(&manager, Some(RowLimit::rate(per_second(10, 10))));
    let facade = managed::<Api>(&manager).await;
    let started = Instant::now();
    facade
        .submit(Once::sent(Cost::keyed("chat_id", 7)))
        .await
        .expect("first message to the chat");
    facade
        .submit(Once::sent(Cost::keyed("chat_id", 8)))
        .await
        .expect("another chat is not held back");
    assert_eq!(started.elapsed(), Duration::ZERO);
    facade
        .submit(Once::sent(Cost::keyed("chat_id", 7)))
        .await
        .expect("the chat's next slot");
    assert_eq!(started.elapsed(), Duration::from_secs(1));
}

// ── admission ────────────────────────────────────────────────────────────

async fn refused_after(
    close: impl FnOnce(&Manager),
) -> (OperationError, crate::ResourceOpsSnapshot) {
    let manager = metered_manager();
    resident(&manager, None);
    let facade = managed::<Api>(&manager).await;
    close(&manager);
    assert!(facade.is_closing());
    let error = facade
        .submit(Once::sent(Cost::ONE))
        .await
        .expect_err("a closed lease admits no attempt");
    (error, snapshot(&manager))
}

#[tokio::test]
async fn a_closed_lease_refuses_attempts_and_nothing_is_sent() {
    let (removed, metrics) = refused_after(|manager| {
        manager.remove(&Api::key()).expect("remove");
    })
    .await;
    assert_eq!(*removed.kind(), ErrorKind::Cancelled);
    assert_eq!(removed.sent(), SentState::NotSent);
    assert_eq!(metrics.call_attempts.granted, 0);

    let (suspended, metrics) = refused_after(suspend::<Api>).await;
    assert!(
        matches!(
            suspended.kind(),
            ErrorKind::CredentialUnavailable {
                reason: CredentialUnavailableReason::ReauthRequired
            }
        ),
        "{suspended}"
    );
    assert_eq!(suspended.sent(), SentState::NotSent);
    assert!(suspended.is_retryable());
    assert_eq!(suspended.retry_after(), Some(Duration::from_secs(30)));
    let as_error = Error::from(suspended);
    assert_eq!(as_error.retry_after(), Some(Duration::from_secs(30)));
    assert_eq!(metrics.call_units.not_sent, 1);

    let (tainted, _) = refused_after(|manager| {
        let _tainted = manager
            .taint_slot(&Api::key(), ScopeLevel::Global, "db")
            .expect("taint");
    })
    .await;
    assert_eq!(*tainted.kind(), ErrorKind::Revoked);
    assert_eq!(tainted.sent(), SentState::NotSent);
}

#[tokio::test(start_paused = true)]
async fn a_new_attempt_is_refused_once_the_lease_closes() {
    let manager = metered_manager();
    resident(&manager, None);
    let facade = managed::<Api>(&manager).await;
    let (operation, gate) = gated();
    let running = tokio::spawn(facade.submit(operation));
    gate.entered.notified().await;
    suspend::<Api>(&manager);

    let error = facade
        .submit(Once::sent(Cost::FREE))
        .await
        .expect_err("closed");
    assert!(matches!(
        error.kind(),
        ErrorKind::CredentialUnavailable { .. }
    ));
    assert_eq!(error.sent(), SentState::NotSent);
    let metrics = snapshot(&manager);
    assert_eq!(metrics.call_attempts.granted, 1);
    assert_eq!(
        metrics.call_attempts.refused, 0,
        "a unit submitted on a closed lease is refused before it asks for an attempt"
    );
    assert_eq!(metrics.call_units.not_sent, 1);

    gate.release.notify_one();
    running
        .await
        .expect("unit task")
        .expect("closing never aborts a granted attempt");
    assert_eq!(
        gate.closed_seen.load(Ordering::SeqCst),
        1,
        "the notice fired"
    );
}

#[tokio::test(start_paused = true)]
async fn a_suspension_during_the_quota_wait_ends_it_credential_unavailable() {
    let manager = Manager::new();
    // The acquire takes the only permit; the attempt waits a second for one.
    resident(&manager, Some(RowLimit::rate(per_second(1, 1))));
    let facade = managed::<Api>(&manager).await;
    let started = Instant::now();
    let unit = tokio::spawn(facade.submit(Once::sent(Cost::ONE)));
    settle_tasks().await;

    suspend::<Api>(&manager);
    let error = unit
        .await
        .expect("unit task")
        .expect_err("the wait ends with the lease");
    assert!(
        matches!(error.kind(), ErrorKind::CredentialUnavailable { .. }),
        "{error}"
    );
    assert_eq!(error.sent(), SentState::NotSent);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "refused before the slot came"
    );
}

// ── cancellation ─────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn a_cancel_before_the_first_grant_settles_cancelled_not_sent() {
    let manager = Manager::new();
    resident(&manager, Some(RowLimit::rate(per_second(1, 1))));
    let facade = managed::<Api>(&manager).await;

    // Before the first poll.
    let unit = facade.submit(Once::sent(Cost::FREE));
    unit.cancel();
    unit.cancel();
    let error = unit.await.expect_err("cancelled before it started");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::NotSent);

    // During the first attempt's quota wait (the acquire took the permit).
    let started = Instant::now();
    let mut unit = facade.submit(Once::sent(Cost::ONE));
    assert!(futures::poll!(&mut unit).is_pending());
    settle_tasks().await;
    unit.cancel();
    let error = unit.await.expect_err("the quota wait ends on cancel");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::NotSent);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn a_denied_unit_honours_pre_grant_cancellation_and_records_every_settlement() {
    let manager = metered_manager();
    resident(&manager, None);
    let parent = CancellationToken::new();
    let ctx = ResourceContext::minimal(Scope::default(), parent.clone());
    let row = read_only_row::<Api>(&manager, &ctx);
    let ran = Arc::new(AtomicUsize::new(0));

    let unit = row.submit(NeverWrite {
        polled: Arc::clone(&ran),
    });
    unit.cancel();
    let cancelled = unit
        .await
        .expect_err("unit cancellation wins before a grant");
    assert_eq!(*cancelled.kind(), ErrorKind::Cancelled);
    assert_eq!(cancelled.sent(), SentState::NotSent);

    let denied = row
        .submit(NeverWrite {
            polled: Arc::clone(&ran),
        })
        .await
        .expect_err("a write needs execution-owner authority");
    assert_eq!(*denied.kind(), ErrorKind::Permanent);
    assert_eq!(denied.sent(), SentState::NotSent);

    parent.cancel();
    let parent_cancelled = row
        .submit(NeverWrite {
            polled: Arc::clone(&ran),
        })
        .await
        .expect_err("parent cancellation wins before a grant");
    assert_eq!(*parent_cancelled.kind(), ErrorKind::Cancelled);
    assert_eq!(parent_cancelled.sent(), SentState::NotSent);

    assert_eq!(ran.load(Ordering::SeqCst), 0, "provider code never ran");
    let metrics = snapshot(&manager);
    assert_eq!(metrics.call_units.not_sent, 3);
    assert_eq!(metrics.call_attempts.granted, 0);
}

#[tokio::test(start_paused = true)]
async fn a_cancel_after_the_grant_is_ignored() {
    let manager = Manager::new();
    resident(&manager, None);
    let facade = managed::<Api>(&manager).await;
    let (operation, gate) = gated();
    let mut unit = facade.submit(operation);
    assert!(futures::poll!(&mut unit).is_pending());
    gate.entered.notified().await;
    unit.cancel();
    gate.release.notify_one();
    unit.await.expect("the runtime settles a granted unit");
}

// ── settled outcome ──────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn a_deadline_after_a_grant_is_maybe_sent_and_unknown_for_a_write() {
    let manager = metered_manager();
    resident(&manager, None);
    let facade = managed::<Api>(&manager).await;
    let mut events = manager.subscribe_events();

    let started = Instant::now();
    let error = facade
        .submit(Hang::<false>)
        .with_deadline(in_one(Duration::from_secs(2)))
        .await
        .expect_err("the deadline stops the unit");
    assert_eq!(started.elapsed(), Duration::from_secs(2));
    assert_eq!(*error.kind(), ErrorKind::Transient);
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert_eq!(error.effect(), Effect::Write);
    assert!(!error.is_retryable());
    assert_eq!(error.retry_after(), None);
    let as_error = Error::from(error);
    assert_eq!(*as_error.kind(), ErrorKind::OutcomeUnknown);
    assert!(!as_error.is_retryable());

    let mut unknown = 0;
    while let Some(event) = events.try_recv() {
        if matches!(&event, ResourceEvent::OperationOutcomeUnknown { key } if *key == Api::key()) {
            unknown += 1;
        }
    }
    assert_eq!(unknown, 1);
    let metrics = snapshot(&manager);
    assert_eq!(metrics.call_units.maybe_sent, 1);
    assert_eq!(metrics.call_attempts.granted, 1);

    let error = facade
        .submit(Hang::<true>)
        .with_deadline(in_one(Duration::from_secs(1)))
        .await
        .expect_err("the deadline stops the read");
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert!(error.is_retryable(), "a read may be replayed");
    assert_eq!(*Error::from(error).kind(), ErrorKind::Transient);
    while let Some(event) = events.try_recv() {
        assert!(
            !matches!(event, ResourceEvent::OperationOutcomeUnknown { .. }),
            "a replay-safe unit publishes no unknown outcome"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_deadline_can_only_shorten_the_host_cap() {
    let manager = Manager::new();
    resident(&manager, None);
    let facade = managed::<Api>(&manager).await;
    let started = Instant::now();
    let error = facade
        .submit(Hang::<true>)
        .with_deadline(in_one(OPERATION_DEADLINE_CAP * 2))
        .await
        .expect_err("capped");
    assert_eq!(started.elapsed(), OPERATION_DEADLINE_CAP);
    assert_eq!(error.sent(), SentState::MaybeSent);
}

#[tokio::test(start_paused = true)]
async fn a_panic_after_a_grant_is_maybe_sent() {
    let manager = Manager::new();
    resident(&manager, None);
    let facade = managed::<Api>(&manager).await;
    let error = facade
        .submit(PanicAfterGrant)
        .await
        .expect_err("the panic settles the unit");
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert_eq!(*Error::from(error).kind(), ErrorKind::OutcomeUnknown);
    facade
        .submit(Once::sent(Cost::FREE))
        .await
        .expect("the lease survives a panicking unit");
}

#[tokio::test(start_paused = true)]
async fn the_attempt_budget_bounds_grants() {
    let manager = metered_manager();
    resident(&manager, None);
    let facade = managed::<Api>(&manager).await;
    let error = facade
        .submit(OverBudget)
        .await
        .expect_err("the third attempt is refused");
    assert_eq!(*error.kind(), ErrorKind::Permanent);
    assert_eq!(error.detail(), "attempt budget exhausted");
    assert_eq!(error.sent(), SentState::Sent, "two attempts were answered");
    let metrics = snapshot(&manager);
    assert_eq!(metrics.call_attempts.granted, 2);
    assert_eq!(metrics.call_attempts.refused, 1);
    assert_eq!(metrics.call_units.sent, 1);
}

#[tokio::test(start_paused = true)]
async fn a_provider_refusal_is_sent_and_retryable_with_a_capped_hint() {
    let manager = Manager::new();
    resident(&manager, None);
    let facade = managed::<Api>(&manager).await;
    let error = facade
        .submit(Refused {
            error: OperationError::throttled(Some(Duration::from_mins(10))),
        })
        .await
        .expect_err("throttled");
    assert_eq!(error.sent(), SentState::Sent);
    assert!(error.is_retryable(), "a refusal applied nothing");
    assert_eq!(
        error.retry_after(),
        Some(crate::rate_limit::DEFAULT_MAX_PENALTY),
        "the hint is capped"
    );
}

#[test]
fn the_retry_safety_table_holds() {
    let key = resource_key!("table");
    let transient = ErrorKind::Transient;
    let exhausted = ErrorKind::Exhausted { retry_after: None };
    let unknown = ErrorKind::OutcomeUnknown;
    let permanent = ErrorKind::Permanent;
    let cases = [
        // (kind, sent, effect, retryable, kind as `Error`)
        (
            &permanent,
            SentState::NotSent,
            Effect::Read,
            false,
            &permanent,
        ),
        (
            &transient,
            SentState::NotSent,
            Effect::Write,
            true,
            &transient,
        ),
        (&exhausted, SentState::Sent, Effect::Write, true, &exhausted),
        (&transient, SentState::Sent, Effect::Read, true, &transient),
        (
            &transient,
            SentState::Sent,
            Effect::Idempotent,
            true,
            &transient,
        ),
        (&transient, SentState::Sent, Effect::Write, false, &unknown),
        (
            &transient,
            SentState::MaybeSent,
            Effect::Read,
            true,
            &transient,
        ),
        (
            &transient,
            SentState::MaybeSent,
            Effect::Idempotent,
            true,
            &transient,
        ),
        (
            &transient,
            SentState::MaybeSent,
            Effect::Write,
            false,
            &unknown,
        ),
        (
            &exhausted,
            SentState::MaybeSent,
            Effect::Write,
            false,
            &unknown,
        ),
        (
            &permanent,
            SentState::MaybeSent,
            Effect::Write,
            false,
            &permanent,
        ),
    ];
    for (kind, sent, effect, retryable, as_error) in cases {
        let error = OperationError::new(kind.clone(), "case").settled(sent, effect, &key);
        assert_eq!(
            error.is_retryable(),
            retryable,
            "{kind:?} {sent:?} {effect:?}"
        );
        let converted = Error::from(error);
        assert_eq!(converted.kind(), as_error, "{kind:?} {sent:?} {effect:?}");
        assert_eq!(converted.resource_key(), Some(&key));
    }
}

#[test]
fn a_resource_error_converts_by_kind_only() {
    let error = Error::transient("upstream said: token=hunter2")
        .with_resource_key(resource_key!("secretive"))
        .with_source(std::io::Error::other("hunter2"));
    let op = OperationError::from(error);
    assert_eq!(*op.kind(), ErrorKind::Transient);
    assert_eq!(op.resource_key(), Some(&resource_key!("secretive")));
    assert!(!op.to_string().contains("hunter2"));
    assert!(!format!("{op:?}").contains("hunter2"));
    assert!(std::error::Error::source(&op).is_none());
    assert!(!format!("{:?}", Cost::keyed("chat_id", "hunter2")).contains("hunter2"));
}

// ── unit cap, lease sharing and drain ────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn one_unit_runs_at_a_time_on_a_pooled_lease() {
    let manager = Manager::new();
    pooled(&manager);
    let facade = managed::<PooledApi>(&manager).await;
    let (first, first_gate) = gated();
    let (second, second_gate) = gated();
    let first = tokio::spawn(facade.submit(first));
    let second = tokio::spawn(facade.submit(second));
    first_gate.entered.notified().await;
    settle_tasks().await;
    assert!(
        futures::poll!(Box::pin(second_gate.entered.notified())).is_pending(),
        "the second unit waits for the lease's unit slot"
    );

    first_gate.release.notify_one();
    first.await.expect("task").expect("first");
    second_gate.entered.notified().await;
    second_gate.release.notify_one();
    second.await.expect("task").expect("second");
}

#[tokio::test(start_paused = true)]
async fn units_share_a_resident_lease() {
    let manager = Manager::new();
    resident(&manager, None);
    let facade = managed::<Api>(&manager).await;
    let (first, first_gate) = gated();
    let (second, second_gate) = gated();
    let first = tokio::spawn(facade.submit(first));
    let second = tokio::spawn(facade.submit(second));
    first_gate.entered.notified().await;
    second_gate.entered.notified().await;
    first_gate.release.notify_one();
    second_gate.release.notify_one();
    first.await.expect("task").expect("first");
    second.await.expect("task").expect("second");
}

#[tokio::test(start_paused = true)]
async fn unit_slots_full_until_the_deadline_is_backpressure() {
    let manager = Manager::new();
    pooled(&manager);
    let facade = managed::<PooledApi>(&manager).await;
    let (operation, gate) = gated();
    let holder = tokio::spawn(facade.submit(operation));
    gate.entered.notified().await;
    let error = facade
        .submit(Once::sent(Cost::FREE))
        .with_deadline(in_one(Duration::from_secs(1)))
        .await
        .expect_err("no slot before the deadline");
    assert_eq!(*error.kind(), ErrorKind::Backpressure);
    assert_eq!(error.sent(), SentState::NotSent);
    gate.release.notify_one();
    holder.await.expect("task").expect("holder");
}

#[tokio::test]
async fn clones_share_the_lease_and_the_drain_waits_for_running_units() {
    let manager = Arc::new(Manager::new());
    resident(&manager, None);
    let facade = managed::<Api>(&manager).await;
    let clone = facade.clone();
    let closing = facade.closing();
    let (operation, gate) = gated();
    let running = tokio::spawn(clone.submit(operation));
    gate.entered.notified().await;
    drop(clone);
    drop(facade);
    assert_eq!(
        row::<Api>(&manager).in_flight.0.load(Ordering::SeqCst),
        1,
        "the running unit keeps the lease"
    );

    let shutdown = tokio::spawn({
        let manager = Arc::clone(&manager);
        async move {
            manager
                .graceful_shutdown(crate::ShutdownConfig::default())
                .await
        }
    });
    closing.closed().await;
    assert!(!shutdown.is_finished(), "the drain waits for the unit");

    gate.release.notify_one();
    running.await.expect("task").expect("the unit settles");
    assert_eq!(gate.closed_seen.load(Ordering::SeqCst), 1);
    shutdown
        .await
        .expect("shutdown task")
        .expect("the drain completes once the unit released the lease");
}

#[tokio::test(start_paused = true)]
async fn a_dropped_waiter_after_the_grant_still_settles() {
    let manager = metered_manager();
    resident(&manager, None);
    let facade = managed::<Api>(&manager).await;
    let (operation, gate) = gated();
    let mut unit = facade.submit(operation);
    assert!(futures::poll!(&mut unit).is_pending());
    gate.entered.notified().await;
    drop(unit);
    drop(facade);
    let row = row::<Api>(&manager);
    assert_eq!(row.in_flight.0.load(Ordering::SeqCst), 1);

    gate.release.notify_one();
    settle_tasks().await;
    assert_eq!(
        snapshot(&manager).call_units.sent,
        1,
        "the runtime settled it"
    );
    assert_eq!(
        row.in_flight.0.load(Ordering::SeqCst),
        0,
        "the lease was released after the unit ended"
    );
}

// ── pinned slots ─────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn a_unit_keeps_its_pinned_slot_and_the_next_unit_sees_the_rotation() {
    let manager = Manager::new();
    let api = resident(&manager, None);
    api.token.store(Arc::new("v1".to_owned()));
    let facade = managed::<Api>(&manager).await;

    let (first, second) = facade
        .submit(PinnedTwice {
            rotate: Some((Arc::clone(&api.token), "v2")),
        })
        .await
        .expect("both attempts");
    assert_eq!(first.as_deref(), Some("v1"));
    assert_eq!(second.as_deref(), Some("v1"), "no rotation mid-unit");

    let (next, _) = facade
        .submit(PinnedTwice { rotate: None })
        .await
        .expect("next unit");
    assert_eq!(next.as_deref(), Some("v2"));
}

#[tokio::test(start_paused = true)]
async fn a_rotation_before_the_first_grant_reaches_the_unit() {
    let manager = Manager::new();
    let api = resident(&manager, None);
    api.token.store(Arc::new("v1".to_owned()));
    let facade = managed::<Api>(&manager).await;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());

    let unit = tokio::spawn(facade.submit(PinAfterRelease {
        entered: Arc::clone(&entered),
        release: Arc::clone(&release),
    }));
    entered.notified().await;
    api.token.store(Arc::new("v2".to_owned()));
    release.notify_one();

    let pinned = unit.await.expect("joined").expect("granted");
    assert_eq!(
        pinned.as_deref(),
        Some("v2"),
        "the slots are pinned at the first grant, not when the unit starts"
    );
}

// ── observability ────────────────────────────────────────────────────────

pub(super) type CapturedSpans = Vec<(&'static str, Vec<(String, String)>)>;

/// Captures span names and recorded fields on this thread.
#[derive(Clone, Default)]
pub(super) struct SpanCapture {
    pub(super) spans: Arc<Mutex<CapturedSpans>>,
}

impl SpanCapture {
    /// The last value recorded for `field` on the first span named `name`.
    pub(super) fn field(&self, name: &str, field: &str) -> Option<String> {
        let spans = self.spans.lock().expect("spans");
        let (_, fields) = spans.iter().find(|(span, _)| *span == name)?;
        fields
            .iter()
            .rev()
            .find(|(recorded, _)| recorded == field)
            .map(|(_, value)| value.clone())
    }
}

struct FieldVisitor<'a>(&'a mut Vec<(String, String)>);

impl tracing::field::Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push((field.name().to_owned(), format!("{value:?}")));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.push((field.name().to_owned(), value.to_owned()));
    }
}

impl tracing::Subscriber for SpanCapture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, attributes: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let mut spans = self.spans.lock().expect("spans");
        let mut fields = Vec::new();
        attributes.record(&mut FieldVisitor(&mut fields));
        spans.push((attributes.metadata().name(), fields));
        tracing::span::Id::from_u64(spans.len() as u64)
    }

    fn record(&self, span: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        let mut spans = self.spans.lock().expect("spans");
        let index = usize::try_from(span.into_u64()).expect("index") - 1;
        values.record(&mut FieldVisitor(&mut spans[index].1));
    }

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, _: &tracing::Event<'_>) {}

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test(start_paused = true)]
async fn each_unit_runs_in_a_span_with_its_outcome() {
    let capture = SpanCapture::default();
    let _default = tracing::subscriber::set_default(capture.clone());
    let manager = Manager::new();
    resident(&manager, None);
    let facade = managed::<Api>(&manager).await;
    facade
        .submit(Once::sent(Cost::FREE))
        .await
        .expect("settled");

    let spans = capture.spans.lock().expect("spans");
    let (_, fields) = spans
        .iter()
        .find(|(name, _)| *name == "nebula.resource.unit")
        .expect("a unit span");
    let field = |name: &str| {
        fields
            .iter()
            .rev()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.clone())
    };
    assert_eq!(field("key").as_deref(), Some("call-resident"));
    assert_eq!(
        field("operation").as_deref(),
        Some("test.once"),
        "the operation key, not the Rust type"
    );
    assert_eq!(field("attempts").as_deref(), Some("1"));
    assert_eq!(field("sent").as_deref(), Some("sent"));
    assert_eq!(field("outcome").as_deref(), Some("ok"));
}

// ── the first grant races the parent cancellation, nothing after it ─────

#[tokio::test]
async fn a_parent_cancel_refuses_the_first_grant_and_is_ignored_after_it() {
    use super::{UnitScope, managed::UnitShared};

    let parent = CancellationToken::new();
    let scope = UnitScope {
        cancel: Some(parent.clone()),
        deadline: None,
        ..UnitScope::default()
    };
    let unit = UnitShared::new(&scope);
    parent.cancel();
    let error = unit.grant().expect_err("cancelled before the first grant");
    assert_eq!(*error.kind(), ErrorKind::Cancelled);
    assert_eq!(unit.attempts(), 0);
    assert!(unit.grant().is_err(), "latched cancelled");
    assert!(
        UnitShared::new(&scope).grant().is_err(),
        "a unit under a fired parent is refused"
    );

    let parent = CancellationToken::new();
    let unit = UnitShared::new(&UnitScope {
        cancel: Some(parent.clone()),
        deadline: None,
        ..UnitScope::default()
    });
    unit.grant().expect("granted");
    parent.cancel();
    assert!(
        unit.cancel_before_grant().is_none(),
        "no wait races a cancel after the grant"
    );
    unit.grant()
        .expect("a later attempt ignores the parent cancel");
    assert_eq!(unit.attempts(), 2);
}

// ── classified calls ─────────────────────────────────────────────────────

const READ: u8 = 0;
const IDEMPOTENT: u8 = 1;
const WRITE: u8 = 2;

const fn effect_of(effect: u8) -> Effect {
    match effect {
        READ => Effect::Read,
        IDEMPOTENT => Effect::Idempotent,
        _ => Effect::Write,
    }
}

fn one() -> Cost {
    Cost::ONE
}

fn single() -> NonZeroU32 {
    NonZeroU32::MIN
}

/// One [`OperationCx::call`] whose attempts answer the scripted results in
/// order (`Ok(0)` once the script ran out), within `budget` attempts.
#[derive(serde::Serialize, serde::Deserialize)]
struct Scripted<const EFFECT: u8> {
    #[serde(skip)]
    script: Vec<Result<u32, OperationError>>,
    #[serde(skip, default = "one")]
    cost: Cost,
    #[serde(skip, default = "single")]
    budget: NonZeroU32,
    #[serde(skip)]
    calls: Arc<AtomicUsize>,
}

impl<const EFFECT: u8> Scripted<EFFECT> {
    fn new(script: Vec<Result<u32, OperationError>>) -> Self {
        Self {
            script,
            cost: Cost::ONE,
            budget: NonZeroU32::MIN,
            calls: Arc::default(),
        }
    }

    fn budget(mut self, attempts: u32) -> Self {
        self.budget = NonZeroU32::new(attempts).expect("non-zero");
        self
    }

    fn cost(mut self, cost: Cost) -> Self {
        self.cost = cost;
        self
    }

    fn calls(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.calls)
    }
}

impl<R: Provider + PinSlots, const EFFECT: u8> Operation<R> for Scripted<EFFECT> {
    type Output = u32;
    const KEY: &'static str = "test.scripted";
    const EFFECT: Effect = effect_of(EFFECT);

    fn max_attempts(&self) -> NonZeroU32 {
        self.budget
    }

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<u32, OperationError> {
        let mut script = std::collections::VecDeque::from(self.script);
        let calls = self.calls;
        cx.call(self.cost, async move |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            script.pop_front().unwrap_or(Ok(0))
        })
        .await
    }
}

async fn scripted<const EFFECT: u8>(
    facade: &Lease<Api>,
    operation: Scripted<EFFECT>,
) -> (Result<u32, OperationError>, usize) {
    let calls = operation.calls();
    let result = facade.submit(operation).await;
    (result, calls.load(Ordering::SeqCst))
}

#[test]
fn each_constructor_classifies_its_call() {
    let cases = [
        // (error, kind, attempt sent, retried for Read, Idempotent, Write)
        (
            OperationError::throttled(Some(Duration::from_secs(2))),
            ErrorKind::Exhausted {
                retry_after: Some(Duration::from_secs(2)),
            },
            SentState::Sent,
            [true, true, true],
        ),
        (
            OperationError::throttled_key(None),
            ErrorKind::Exhausted { retry_after: None },
            SentState::Sent,
            [true, true, true],
        ),
        (
            OperationError::unreachable("no route"),
            ErrorKind::Transient,
            SentState::NotSent,
            [true, true, true],
        ),
        (
            OperationError::unreachable_as(ErrorKind::Backpressure, "buffer full"),
            ErrorKind::Backpressure,
            SentState::NotSent,
            [true, true, true],
        ),
        (
            OperationError::unreachable_as(ErrorKind::Cancelled, "client closed"),
            ErrorKind::Cancelled,
            SentState::NotSent,
            [false, false, false],
        ),
        (
            OperationError::interrupted("reset"),
            ErrorKind::Transient,
            SentState::MaybeSent,
            [true, true, false],
        ),
        (
            OperationError::rejected("bad request"),
            ErrorKind::Permanent,
            SentState::Sent,
            [false, false, false],
        ),
        (
            OperationError::rejected_as(ErrorKind::NotFound, "no such row"),
            ErrorKind::NotFound,
            SentState::Sent,
            [false, false, false],
        ),
        (
            OperationError::rejected_as(ErrorKind::Transient, "declined"),
            ErrorKind::Permanent,
            SentState::Sent,
            [false, false, false],
        ),
        (
            OperationError::new(ErrorKind::Transient, "unclassified"),
            ErrorKind::Transient,
            SentState::MaybeSent,
            [true, true, false],
        ),
        (
            OperationError::new(ErrorKind::Permanent, "unclassified"),
            ErrorKind::Permanent,
            SentState::MaybeSent,
            [false, false, false],
        ),
        (
            OperationError::from(Error::transient("converted")),
            ErrorKind::Transient,
            SentState::MaybeSent,
            [true, true, false],
        ),
    ];
    let effects = [Effect::Read, Effect::Idempotent, Effect::Write];
    for (error, kind, sent, retried) in cases {
        assert_eq!(*error.kind(), kind, "{error:?}");
        assert_eq!(error.attempt_sent(), sent, "{error:?}");
        for (effect, retried) in effects.into_iter().zip(retried) {
            assert_eq!(
                error.retried_in_call(effect),
                retried,
                "{error:?} for {effect:?}"
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_called_unit_folds_its_classified_sent_state() {
    let manager = Manager::new();
    resident(&manager, None);
    let facade = managed::<Api>(&manager).await;
    let cases = [
        (Ok(1), SentState::Sent),
        (Err(OperationError::throttled(None)), SentState::Sent),
        (
            Err(OperationError::unreachable("no route")),
            SentState::NotSent,
        ),
        (
            Err(OperationError::interrupted("reset")),
            SentState::MaybeSent,
        ),
        (
            Err(OperationError::rejected("bad request")),
            SentState::Sent,
        ),
        (
            Err(OperationError::new(ErrorKind::Permanent, "unclassified")),
            SentState::MaybeSent,
        ),
    ];
    for (reply, sent) in cases {
        let expected_ok = reply.is_ok();
        let operation = Scripted::<WRITE>::new(vec![reply]).cost(Cost::FREE);
        let (result, calls) = scripted(&facade, operation).await;
        assert_eq!(calls, 1);
        match result {
            Ok(_) => assert!(expected_ok),
            Err(error) => {
                assert!(!expected_ok);
                assert_eq!(error.sent(), sent, "{error:?}");
            },
        }
    }
    let operation =
        Scripted::<WRITE>::new(vec![Err(OperationError::interrupted("reset"))]).cost(Cost::FREE);
    let error = scripted(&facade, operation)
        .await
        .0
        .expect_err("interrupted");
    assert_eq!(
        *Error::from(error).kind(),
        ErrorKind::OutcomeUnknown,
        "an interrupted write has an unknown outcome"
    );
}

#[tokio::test(start_paused = true)]
async fn a_throttle_pauses_the_quota_and_the_next_attempt_waits_it_out() {
    let manager = Manager::new();
    resident(&manager, Some(RowLimit::rate(per_second(10, 10))));
    let facade = managed::<Api>(&manager).await;
    let started = Instant::now();
    let operation = Scripted::<WRITE>::new(vec![
        Err(OperationError::throttled(Some(Duration::from_secs(3)))),
        Ok(7),
    ])
    .budget(2);
    let (result, calls) = scripted(&facade, operation).await;
    assert_eq!(result.expect("the retry was answered"), 7);
    assert_eq!(calls, 2, "a throttle is retried, even for a write");
    assert_eq!(
        started.elapsed(),
        Duration::from_secs(3),
        "the next attempt's booking waited the pause out; nothing slept"
    );
    // The pause holds the whole quota: another unit waits too.
    let operation = Scripted::<READ>::new(vec![Err(OperationError::throttled(Some(
        Duration::from_secs(2),
    )))]);
    let error = scripted(&facade, operation)
        .await
        .0
        .expect_err("no attempt left");
    assert_eq!(error.sent(), SentState::Sent);
    assert!(error.is_retryable(), "a throttle applied nothing");
    let paused = Instant::now();
    facade
        .submit(Once::sent(Cost::ONE))
        .await
        .expect("after the pause");
    assert_eq!(paused.elapsed(), Duration::from_secs(2));
}

#[tokio::test(start_paused = true)]
async fn a_key_throttle_pauses_only_its_key() {
    let manager = Manager::new();
    resident(&manager, Some(RowLimit::rate(per_second(10, 10))));
    let facade = managed::<Api>(&manager).await;
    let operation = Scripted::<WRITE>::new(vec![Err(OperationError::throttled_key(Some(
        Duration::from_secs(5),
    )))])
    .cost(Cost::keyed("chat_id", 7));
    assert!(scripted(&facade, operation).await.0.is_err());
    let started = Instant::now();
    facade
        .submit(Once::sent(Cost::keyed("chat_id", 8)))
        .await
        .expect("another chat");
    facade
        .submit(Once::sent(Cost::ONE))
        .await
        .expect("the account quota");
    assert_eq!(started.elapsed(), Duration::ZERO, "only chat 7 paused");
    facade
        .submit(Once::sent(Cost::keyed("chat_id", 7)))
        .await
        .expect("the chat after its pause");
    assert_eq!(started.elapsed(), Duration::from_secs(5));
}

/// Throttles without a hint, then answers `between`, then throttles again:
/// how long the next booking waits after the second throttle.
async fn second_backoff(between: OperationError) -> Duration {
    let manager = Manager::new();
    resident(&manager, Some(RowLimit::rate(per_second(10, 10))));
    let facade = managed::<Api>(&manager).await;
    for reply in [
        OperationError::throttled(None),
        between,
        OperationError::throttled(None),
    ] {
        let (result, _) = scripted(&facade, Scripted::<READ>::new(vec![Err(reply)])).await;
        assert!(result.is_err());
    }
    let started = Instant::now();
    facade
        .submit(Once::sent(Cost::ONE))
        .await
        .expect("after the backoff");
    started.elapsed()
}

#[tokio::test(start_paused = true)]
async fn an_unreachable_or_interrupted_call_never_resets_the_backoff() {
    // The second consecutive refusal backs off 1 s to 2 s, a first one
    // half a second to 1 s.
    for between in [
        OperationError::unreachable("no route"),
        OperationError::interrupted("reset"),
    ] {
        let waited = second_backoff(between.clone()).await;
        assert!(
            waited >= Duration::from_secs(1),
            "{between:?} kept the streak: {waited:?}"
        );
    }
    let waited = second_backoff(OperationError::rejected("bad request")).await;
    assert!(
        waited <= Duration::from_secs(1),
        "an answer resets the streak: {waited:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn retries_inside_a_call_follow_the_classification_within_the_budget() {
    let manager = Manager::new();
    resident(&manager, None);
    let facade = managed::<Api>(&manager).await;
    let interrupted = || Err(OperationError::interrupted("reset"));

    // An interrupted write is never sent twice.
    let operation = Scripted::<WRITE>::new(vec![interrupted(), Ok(1)]).budget(3);
    let (result, calls) = scripted(&facade, operation).await;
    assert_eq!(calls, 1);
    let error = result.expect_err("not retried");
    assert_eq!(error.sent(), SentState::MaybeSent);

    // A replay-safe one is, until the budget runs out.
    let operation =
        Scripted::<IDEMPOTENT>::new(vec![interrupted(), interrupted(), interrupted(), Ok(1)])
            .budget(3);
    let (result, calls) = scripted(&facade, operation).await;
    assert_eq!(calls, 3, "bounded by max_attempts");
    let error = result.expect_err("the budget ran out");
    assert_eq!(error.detail(), "reset", "the last attempt's error");

    // An unreachable provider is retried for a write, and the unit counts
    // only what may have been applied.
    let operation = Scripted::<WRITE>::new(vec![
        Err(OperationError::throttled(None)),
        Err(OperationError::unreachable("no route")),
    ])
    .cost(Cost::FREE)
    .budget(2);
    let (result, calls) = scripted(&facade, operation).await;
    assert_eq!(calls, 2);
    let error = result.expect_err("unreachable at last");
    assert_eq!(
        error.sent(),
        SentState::NotSent,
        "a throttle applied nothing"
    );
    assert!(error.is_retryable());

    // Rejections and unclassified permanent errors end the call.
    for reply in [
        OperationError::rejected("bad request"),
        OperationError::new(ErrorKind::Permanent, "unclassified"),
    ] {
        let operation = Scripted::<READ>::new(vec![Err(reply), Ok(1)]).budget(3);
        assert_eq!(scripted(&facade, operation).await.1, 1);
    }

    // An unclassified retryable error is retried for a replay-safe effect.
    let operation = Scripted::<READ>::new(vec![
        Err(OperationError::new(ErrorKind::Transient, "unclassified")),
        Ok(4),
    ])
    .budget(3);
    let (result, calls) = scripted(&facade, operation).await;
    assert_eq!((result.expect("retried"), calls), (4, 2));

    // Without a budget there is no hidden retry.
    let operation =
        Scripted::<READ>::new(vec![Err(OperationError::unreachable("no route")), Ok(1)]);
    assert_eq!(scripted(&facade, operation).await.1, 1);
}

#[tokio::test(start_paused = true)]
async fn a_pause_past_the_deadline_ends_the_call_with_the_throttle() {
    let manager = Manager::new();
    resident(&manager, Some(RowLimit::rate(per_second(10, 10))));
    let facade = managed::<Api>(&manager).await;
    let operation = Scripted::<READ>::new(vec![
        Err(OperationError::throttled(Some(Duration::from_mins(1)))),
        Ok(1),
    ])
    .budget(3);
    let calls = operation.calls();
    let started = Instant::now();
    let error = facade
        .submit(operation)
        .with_deadline(in_one(Duration::from_secs(5)))
        .await
        .expect_err("the pause outlasts the deadline");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *error.kind(),
        ErrorKind::Exhausted {
            retry_after: Some(Duration::from_mins(1))
        },
        "the throttle, not the refused booking"
    );
    assert_eq!(error.sent(), SentState::Sent);
    assert!(started.elapsed() < Duration::from_secs(5), "nothing slept");
}

/// A shared limit store whose penalty writes never finish.
struct StalledPenalties(crate::rate_limit::MemoryLimitStore);

impl crate::rate_limit::ErasedLimitStore for StalledPenalties {
    fn reserve_boxed<'a>(
        &'a self,
        key: &'a crate::rate_limit::LimitKey,
        rate: &'a Rate,
        request: crate::rate_limit::ReserveRequest,
    ) -> std::pin::Pin<
        Box<
            dyn Future<
                    Output = Result<
                        Result<crate::rate_limit::Grant, crate::rate_limit::Denied>,
                        crate::rate_limit::LimitStoreError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        self.0.reserve_boxed(key, rate, request)
    }

    fn penalize_boxed<'a>(
        &'a self,
        _key: &'a crate::rate_limit::LimitKey,
        _rate: &'a Rate,
        _retry_after: Duration,
        _max_penalty: Duration,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<(), crate::rate_limit::LimitStoreError>> + Send + 'a>,
    > {
        Box::pin(std::future::pending())
    }

    fn cancel_boxed<'a>(
        &'a self,
        key: &'a crate::rate_limit::LimitKey,
        rate: &'a Rate,
        grant: &'a crate::rate_limit::Grant,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<bool, crate::rate_limit::LimitStoreError>> + Send + 'a>,
    > {
        self.0.cancel_boxed(key, rate, grant)
    }

    fn penalty_boxed<'a>(
        &'a self,
        key: &'a crate::rate_limit::LimitKey,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<Duration, crate::rate_limit::LimitStoreError>> + Send + 'a>,
    > {
        self.0.penalty_boxed(key)
    }
}

#[tokio::test(start_paused = true)]
async fn a_throttle_whose_report_outlasts_the_deadline_settles_as_the_throttle() {
    let store = Arc::new(StalledPenalties(crate::rate_limit::MemoryLimitStore::new()));
    let manager = Manager::with_config(ManagerConfig::default().with_shared_limit_store(store));
    let shared = crate::rate_limit::LimitKey::new("acct:stalled").expect("limit key");
    resident(
        &manager,
        Some(RowLimit::rate(per_second(10, 10)).with_key(shared)),
    );
    let facade = managed::<Api>(&manager).await;
    // The deadline lands inside the report's own budget: it cuts the
    // stalled penalty write off after the attempt finished as throttled.
    let operation = Scripted::<WRITE>::new(vec![Err(OperationError::throttled(Some(
        Duration::from_secs(30),
    )))]);
    let calls = operation.calls();
    let started = Instant::now();
    let error = facade
        .submit(operation)
        .with_deadline(in_one(Duration::from_secs(1)))
        .await
        .expect_err("throttled");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        started.elapsed(),
        Duration::from_secs(1),
        "the deadline cut the report off"
    );
    assert_eq!(
        *error.kind(),
        ErrorKind::Exhausted {
            retry_after: Some(Duration::from_secs(30))
        },
        "the throttle, not an abnormal end: {error}"
    );
    assert_eq!(error.sent(), SentState::Sent);
    assert!(error.is_retryable(), "the provider applied nothing");
    assert!(!matches!(
        Error::from(error).kind(),
        ErrorKind::OutcomeUnknown
    ));

    // A deadline during the provider call itself is still an abnormal end.
    let operation = Hang::<false>;
    let error = facade
        .submit(operation)
        .with_deadline(in_one(Duration::from_secs(1)))
        .await
        .expect_err("cut off");
    assert_eq!(error.sent(), SentState::MaybeSent);
}
