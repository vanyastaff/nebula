//! Rate limiting through the manager: the limiter is consumed on acquire,
//! waits within the caller's deadline, fails fast past it, never trips the
//! recovery gate, is reachable per call through the guard, shares a quota
//! across rows with one key, reaches `Provider::create` so a wrapped client
//! and the acquire path share one pause, and publishes transitions only. A
//! wrapped client's wait ends when its row stops admitting work (revoke,
//! shutdown) but not on a reload, and a detached limiter never ends one.

mod common;

use std::{num::NonZeroU32, sync::Arc, time::Duration};

use common::{ResidentTestResource, test_config, test_ctx};
use nebula_resource::{
    AcquireOptions, ErrorKind, GateState, Manager, RecoveryGate, RecoveryGateConfig,
    RegistrationSpec, Resident, ResidentConfig, ResourceContext, ResourceEvent, ScopeLevel,
    SlotIdentity,
    rate_limit::{LimitKey, Limited, LimitedError, Rate, RowLimit, Throttle, Verdict},
    resource::{Provider, ResourceMetadataDraft},
    topology::resident::ResidentProvider,
};
use tokio::time::Instant;

fn per_second(requests: u32, burst: u32) -> Rate {
    Rate::per_second(NonZeroU32::new(requests).unwrap())
        .with_burst(NonZeroU32::new(burst).unwrap())
        .unwrap()
}

fn register(
    manager: &Manager,
    scope: ScopeLevel,
    limit: RowLimit,
    recovery_gate: Option<Arc<RecoveryGate>>,
) {
    manager
        .register(RegistrationSpec {
            resource: ResidentTestResource::new(),
            config: test_config(),
            scope,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate,
            rate_limit: Some(limit),
        })
        .expect("registration succeeds");
}

fn deadline_in(after: Duration) -> AcquireOptions {
    AcquireOptions::default().with_deadline(std::time::Instant::now() + after)
}

#[tokio::test(start_paused = true)]
async fn acquire_waits_for_its_slot_within_the_deadline() {
    let manager = Manager::new();
    register(
        &manager,
        ScopeLevel::Global,
        RowLimit::rate(per_second(10, 1)),
        None,
    );

    drop(
        manager
            .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
            .await
            .expect("first acquire is free"),
    );
    let started = Instant::now();
    drop(
        manager
            .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
            .await
            .expect("second acquire waits for its slot"),
    );
    assert_eq!(started.elapsed(), Duration::from_millis(100));
}

#[tokio::test(start_paused = true)]
async fn acquire_fails_fast_when_the_slot_is_past_the_deadline() {
    let manager = Manager::new();
    let slow = Rate::new(NonZeroU32::MIN, Duration::from_secs(10)).unwrap();
    register(&manager, ScopeLevel::Global, RowLimit::rate(slow), None);

    drop(
        manager
            .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
            .await
            .expect("first acquire is free"),
    );
    let started = Instant::now();
    let error = manager
        .acquire::<ResidentTestResource>(&test_ctx(), &deadline_in(Duration::from_secs(1)))
        .await
        .expect_err("the next slot is 10 s away, past the 1 s deadline");
    assert_eq!(started.elapsed(), Duration::ZERO, "no pointless wait");
    assert!(matches!(
        error.kind(),
        ErrorKind::Exhausted { retry_after: Some(after) } if *after == Duration::from_secs(10)
    ));
}

#[tokio::test(start_paused = true)]
async fn rate_limit_denial_does_not_trip_the_recovery_gate() {
    let manager = Manager::new();
    let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig::default()));
    let slow = Rate::new(NonZeroU32::MIN, Duration::from_secs(10)).unwrap();
    register(
        &manager,
        ScopeLevel::Global,
        RowLimit::rate(slow),
        Some(Arc::clone(&gate)),
    );

    drop(
        manager
            .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
            .await
            .expect("first acquire is free"),
    );
    manager
        .acquire::<ResidentTestResource>(&test_ctx(), &deadline_in(Duration::from_millis(1)))
        .await
        .expect_err("rate limited");
    assert!(
        matches!(gate.state(), GateState::Idle),
        "our own rate limit is not backend ill health"
    );
}

#[tokio::test(start_paused = true)]
async fn guard_paces_calls_made_within_one_lease() {
    let manager = Manager::new();
    register(
        &manager,
        ScopeLevel::Global,
        RowLimit::rate(per_second(10, 2)),
        None,
    );

    let guard = manager
        .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .expect("acquire consumes one permit of the burst");
    let limits = guard.limits();
    limits
        .ready(None)
        .await
        .expect("the second burst permit is free");
    let started = Instant::now();
    limits
        .ready(None)
        .await
        .expect("the third call waits one emission interval");
    assert_eq!(started.elapsed(), Duration::from_millis(100));
}

/// Two rows drawing on one provider account share its quota: the key, not
/// the row, owns the limit.
#[tokio::test(start_paused = true)]
async fn rows_with_one_key_share_one_quota() {
    let manager = Manager::new();
    let account = LimitKey::new("account:shared").unwrap();
    let org = nebula_core::OrgId::new();
    for scope in [ScopeLevel::Organization(org), ScopeLevel::Global] {
        register(
            &manager,
            scope,
            RowLimit::rate(per_second(1, 1)).with_key(account.clone()),
            None,
        );
    }
    // The global row spends the account's permit…
    drop(
        manager
            .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
            .await
            .expect("first acquire on the account is free"),
    );
    // …so the organization's row, a different registry row, has none left.
    let org_ctx = ResourceContext::minimal(
        nebula_core::scope::Scope {
            org_id: Some(org),
            ..Default::default()
        },
        tokio_util::sync::CancellationToken::new(),
    );
    let error = manager
        .acquire::<ResidentTestResource>(&org_ctx, &deadline_in(Duration::ZERO))
        .await
        .expect_err("the account's one permit per second is spent");
    assert!(matches!(error.kind(), ErrorKind::Exhausted { .. }));
}

/// A third-party client in the shape of `teloxide::Bot`: no HTTP layer to
/// hook, a limit reported as an error variant.
#[derive(Clone)]
struct ChatClient;

#[derive(Debug)]
enum ChatError {
    RetryAfter(Duration),
}

impl ChatClient {
    async fn send(&self, fail: Option<ChatError>) -> Result<(), ChatError> {
        fail.map_or(Ok(()), Err)
    }
}

/// Written once by the resource author, next to the client.
#[derive(Clone)]
struct ChatThrottle;

impl<T> Throttle<T, ChatError> for ChatThrottle {
    fn check(&self, outcome: &Result<T, ChatError>) -> Verdict {
        match outcome {
            Err(ChatError::RetryAfter(after)) => Verdict::Throttled {
                retry_after: Some(*after),
            },
            Ok(_) => Verdict::Pass,
        }
    }
}

/// A resource that declares no rate: it only pauses when the provider says.
#[derive(Clone)]
struct ChatResource;

#[async_trait::async_trait]
impl Provider for ChatResource {
    type Config = common::TestConfig;
    type Instance = Limited<ChatClient, ChatThrottle>;
    type Topology = Resident<Self>;

    fn key() -> nebula_core::ResourceKey {
        nebula_core::resource_key!("test-chat")
    }

    /// No account rate, but the provider allows one message per second per
    /// chat.
    fn resilience() -> nebula_resource::rate_limit::ResiliencePolicy {
        nebula_resource::rate_limit::ResiliencePolicy::new()
            .keyed("chat_id", Rate::per_second(NonZeroU32::MIN))
    }

    async fn create(
        &self,
        _config: &common::TestConfig,
        ctx: &ResourceContext,
    ) -> Result<Self::Instance, nebula_resource::Error> {
        Ok(ctx.limits().wrap(ChatClient, ChatThrottle))
    }

    async fn destroy(
        &self,
        _instance: Self::Instance,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), nebula_resource::Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("test-chat"),
            "",
        )
    }
}

nebula_resource::no_credential_slots!(ChatResource);

impl ResidentProvider for ChatResource {}

#[tokio::test(start_paused = true)]
async fn a_provider_pause_reported_through_the_client_holds_every_acquire() {
    let manager = Manager::new();
    manager
        .register(RegistrationSpec {
            resource: ChatResource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("registration succeeds");

    let guard = manager
        .acquire::<ChatResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .expect("no rate: acquire is free");
    assert_eq!(guard.limits().rate(), None);
    for _ in 0..100 {
        guard
            .run(async |chat| chat.send(None).await)
            .await
            .expect("nothing paces an unlimited resource");
    }
    let error = guard
        .run(async |chat| {
            chat.send(Some(ChatError::RetryAfter(Duration::from_secs(30))))
                .await
        })
        .await
        .expect_err("the provider refused");
    assert!(matches!(
        error,
        LimitedError::Call(ChatError::RetryAfter(_))
    ));
    drop(guard);

    // The client built in `create` shares the row's limiter, so the pause
    // it recorded holds the next acquire too.
    let error = manager
        .acquire::<ChatResource>(&test_ctx(), &deadline_in(Duration::from_secs(1)))
        .await
        .expect_err("paused for 30 s, past the 1 s deadline");
    assert!(matches!(
        error.kind(),
        ErrorKind::Exhausted { retry_after: Some(after) } if *after == Duration::from_secs(30)
    ));
}

fn register_chat(manager: &Manager, limit: Option<RowLimit>) {
    manager
        .register(RegistrationSpec {
            resource: ChatResource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: limit,
        })
        .expect("registration succeeds");
}

#[tokio::test(start_paused = true)]
async fn a_declared_per_chat_limit_paces_calls_to_one_chat() {
    let manager = Manager::new();
    register_chat(&manager, None);
    let guard = manager
        .acquire::<ChatResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .expect("acquire");
    let send = async |chat: &ChatClient| chat.send(None).await;
    guard.run_for("chat_id", 1, send).await.expect("free");
    let started = Instant::now();
    guard
        .run_for("chat_id", 2, send)
        .await
        .expect("another chat");
    assert_eq!(started.elapsed(), Duration::ZERO);
    guard
        .run_for("chat_id", 1, send)
        .await
        .expect("waits for chat 1");
    assert_eq!(started.elapsed(), Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn a_row_override_slows_a_per_chat_limit() {
    let manager = Manager::new();
    let every_three_seconds = Rate::new(NonZeroU32::MIN, Duration::from_secs(3)).unwrap();
    register_chat(
        &manager,
        Some(RowLimit::default().with_keyed("chat_id", every_three_seconds)),
    );
    let guard = manager
        .acquire::<ChatResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .expect("acquire");
    let send = async |chat: &ChatClient| chat.send(None).await;
    guard.run_for("chat_id", 1, send).await.expect("free");
    let started = Instant::now();
    guard.run_for("chat_id", 1, send).await.expect("waits");
    assert_eq!(started.elapsed(), Duration::from_secs(3));

    // Faster than declared is refused at registration, before any call.
    let manager = Manager::new();
    let error = manager
        .register(RegistrationSpec {
            resource: ChatResource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: Some(RowLimit::default().with_keyed("chat_id", per_second(10, 1))),
        })
        .expect_err("tighten only");
    assert_eq!(error.kind(), &ErrorKind::Permanent);
}

#[tokio::test(start_paused = true)]
async fn limit_events_mark_transitions_and_penalties_only() {
    let manager = Manager::new();
    let mut events = manager.subscribe_events();
    register(
        &manager,
        ScopeLevel::Global,
        RowLimit::rate(per_second(10, 1)),
        None,
    );
    let guard = manager
        .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    let limits = guard.limits();
    let mut drain = || {
        let mut seen = Vec::new();
        while let Some(event) = events.try_recv() {
            match event {
                ResourceEvent::RateLimitEngaged { .. } => seen.push("engaged"),
                ResourceEvent::RateLimitCleared { .. } => seen.push("cleared"),
                ResourceEvent::RateLimitPenalized { .. } => seen.push("penalized"),
                _ => {},
            }
        }
        seen
    };

    // Three callers wait at once: the limit engages once. It stays engaged
    // after they are admitted: it clears only when a call passes without
    // waiting, so a caller kept at saturation reports nothing per call.
    let (first, second, third) =
        tokio::join!(limits.ready(None), limits.ready(None), limits.ready(None));
    first.and(second).and(third).unwrap();
    assert_eq!(
        drain(),
        ["engaged"],
        "concurrent waits publish one transition, not one per call"
    );
    for _ in 0..3 {
        limits.ready(None).await.unwrap();
    }
    assert!(
        drain().is_empty(),
        "a caller kept waiting at saturation reports nothing per call"
    );

    limits.penalize(Duration::from_secs(2)).await.unwrap();
    let started = Instant::now();
    limits.ready(None).await.unwrap();
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "a penalty blocks the next call until the provider's Retry-After"
    );
    assert_eq!(drain(), ["penalized"]);

    tokio::time::sleep(Duration::from_secs(1)).await;
    limits.ready(None).await.unwrap();
    assert_eq!(
        drain(),
        ["cleared"],
        "the first call that waits for nothing clears it"
    );
    tokio::time::sleep(Duration::from_secs(1)).await;
    limits.ready(None).await.unwrap();
    assert!(
        drain().is_empty(),
        "a free call on a clear limit says nothing"
    );
}

/// A caller already sleeping on its booked slot when a provider's
/// "slow down" arrives wakes no sooner than the pause ends.
#[tokio::test(start_paused = true)]
async fn a_pause_holds_callers_already_waiting_for_their_slot() {
    let manager = Manager::new();
    register(
        &manager,
        ScopeLevel::Global,
        RowLimit::rate(per_second(1, 1)),
        None,
    );
    let guard = manager
        .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    let limits = Arc::clone(guard.limits());
    let started = Instant::now();
    let waiter = {
        let limits = Arc::clone(&limits);
        tokio::spawn(async move { limits.ready(None).await })
    };
    // The waiter has booked the next slot, one second out.
    tokio::time::sleep(Duration::from_millis(100)).await;
    limits.penalize(Duration::from_secs(5)).await.unwrap();
    waiter.await.unwrap().expect("admitted after the pause");
    assert!(
        started.elapsed() >= Duration::from_millis(5_100),
        "woke {:?} after start, inside the pause",
        started.elapsed()
    );
}

// ---------------------------------------------------------------------------
// A `Limited` wait ends when the row stops admitting work.
// ---------------------------------------------------------------------------

/// [`ChatResource`] with a declared `db` credential slot, so the row can be
/// revoked.
#[derive(Clone)]
struct CredentialedChat;

#[async_trait::async_trait]
impl Provider for CredentialedChat {
    type Config = common::TestConfig;
    type Instance = Limited<ChatClient, ChatThrottle>;
    type Topology = Resident<Self>;

    fn key() -> nebula_core::ResourceKey {
        nebula_core::resource_key!("test-credentialed-chat")
    }

    fn resilience() -> nebula_resource::rate_limit::ResiliencePolicy {
        nebula_resource::rate_limit::ResiliencePolicy::new()
            .keyed("chat_id", Rate::per_second(NonZeroU32::MIN))
    }

    async fn create(
        &self,
        _config: &common::TestConfig,
        ctx: &ResourceContext,
    ) -> Result<Self::Instance, nebula_resource::Error> {
        Ok(ctx.limits().wrap(ChatClient, ChatThrottle))
    }

    async fn destroy(
        &self,
        _instance: Self::Instance,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), nebula_resource::Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("test-credentialed-chat"),
            "",
        )
    }
}

impl nebula_resource::HasCredentialSlots for CredentialedChat {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    fn declares_credential_slots() -> bool {
        true
    }

    fn credential_slot_names() -> &'static [&'static str] {
        &["db"]
    }
}

impl ResidentProvider for CredentialedChat {}

const PAUSE: Duration = Duration::from_mins(1);

type ChatCall = Result<(), LimitedError<ChatError>>;

/// Which `Limited` call the parked lease waits in.
#[derive(Clone, Copy)]
enum ParkedCall {
    Run,
    RunForChat,
}

/// Registers the row, records a 60 s provider pause through its client,
/// and parks a task holding a lease inside `call`'s wait.
async fn parked_behind_a_pause(
    manager: &Arc<Manager>,
    call: ParkedCall,
) -> tokio::task::JoinHandle<ChatCall> {
    manager
        .register(RegistrationSpec {
            resource: CredentialedChat,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("registration succeeds");
    let guard = manager
        .acquire::<CredentialedChat>(&test_ctx(), &AcquireOptions::default())
        .await
        .expect("acquire");
    let refused = guard
        .run(async |chat| chat.send(Some(ChatError::RetryAfter(PAUSE))).await)
        .await;
    assert!(matches!(refused, Err(LimitedError::Call(_))));
    let task = tokio::spawn(async move {
        let send = async |chat: &ChatClient| chat.send(None).await;
        let outcome = match call {
            ParkedCall::Run => guard.run(send).await,
            ParkedCall::RunForChat => guard.run_for("chat_id", 1, send).await,
        };
        drop(guard);
        outcome
    });
    // Let the task book and park on the pause; the clock stays well inside it.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        !task.is_finished(),
        "the call waits out the provider's pause"
    );
    task
}

fn assert_ended_as_cancelled(outcome: ChatCall) {
    match outcome {
        Err(LimitedError::Limit(error)) => assert_eq!(error.kind(), &ErrorKind::Cancelled),
        other => panic!("expected a cancelled limit wait, got {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn revoke_ends_a_limited_wait_and_drains_promptly() {
    let manager = Arc::new(Manager::new());
    let waiter = parked_behind_a_pause(&manager, ParkedCall::Run).await;

    let started = Instant::now();
    let outcome = manager
        .revoke_slot(&CredentialedChat::key(), ScopeLevel::Global, "db")
        .await
        .expect("revoke");
    assert!(matches!(
        outcome,
        nebula_resource::SlotDispatchOutcome::Completed {
            drain: nebula_resource::SlotDrainOutcome::Drained,
            ..
        }
    ));
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the drain waited {:?}: the lease stayed parked on the pause",
        started.elapsed()
    );
    assert_ended_as_cancelled(waiter.await.expect("waiter task"));
}

#[tokio::test(start_paused = true)]
async fn graceful_shutdown_ends_a_limited_wait() {
    let manager = Arc::new(Manager::new());
    let waiter = parked_behind_a_pause(&manager, ParkedCall::Run).await;

    let started = Instant::now();
    manager
        .graceful_shutdown(nebula_resource::ShutdownConfig::default())
        .await
        .expect("the parked lease releases once its wait ends");
    assert!(started.elapsed() < Duration::from_secs(30));
    assert_ended_as_cancelled(waiter.await.expect("waiter task"));
}

#[tokio::test(start_paused = true)]
async fn revoke_ends_a_per_key_limited_wait() {
    let manager = Arc::new(Manager::new());
    let waiter = parked_behind_a_pause(&manager, ParkedCall::RunForChat).await;

    let started = Instant::now();
    let outcome = manager
        .revoke_slot(&CredentialedChat::key(), ScopeLevel::Global, "db")
        .await
        .expect("revoke");
    assert!(matches!(
        outcome,
        nebula_resource::SlotDispatchOutcome::Completed {
            drain: nebula_resource::SlotDrainOutcome::Drained,
            ..
        }
    ));
    assert!(started.elapsed() < Duration::from_secs(30));
    assert_ended_as_cancelled(waiter.await.expect("waiter task"));
}

#[tokio::test(start_paused = true)]
async fn a_reload_does_not_interrupt_a_limited_wait() {
    let manager = Arc::new(Manager::new());
    let started = Instant::now();
    let waiter = parked_behind_a_pause(&manager, ParkedCall::Run).await;

    manager
        .reload_config::<CredentialedChat>(
            common::TestConfig {
                name: "reloaded".to_owned(),
            },
            &ScopeLevel::Global,
        )
        .expect("reload");
    waiter
        .await
        .expect("waiter task")
        .expect("a reload is benign: the call runs once the pause ends");
    assert!(started.elapsed() >= PAUSE);
}

#[tokio::test(start_paused = true)]
async fn a_detached_limiter_ignores_manager_shutdown() {
    let manager = Manager::new();
    register_chat(&manager, None);
    // Built outside any registry row: the limiter belongs to no row.
    let chat = test_ctx().limits().wrap(ChatClient, ChatThrottle);
    chat.limits().penalize(PAUSE).await.expect("pause");
    let started = Instant::now();
    let waiter = tokio::spawn(async move { chat.run(async |chat| chat.send(None).await).await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    manager.shutdown();
    waiter
        .await
        .expect("waiter task")
        .expect("a detached limiter waits its pause out");
    assert!(started.elapsed() >= PAUSE);
}

#[tokio::test(start_paused = true)]
async fn a_credential_suspension_ends_a_limited_wait_as_credential_unavailable() {
    let manager = Arc::new(Manager::new());
    let waiter = parked_behind_a_pause(&manager, ParkedCall::Run).await;

    let started = Instant::now();
    let outcome = manager
        .suspend_credential_row(
            &CredentialedChat::key(),
            &ScopeLevel::Global,
            &SlotIdentity::Unbound,
            "db",
            nebula_resource::CredentialUnavailableReason::ReauthRequired,
            None,
        )
        .expect("suspend");
    assert_eq!(
        outcome,
        nebula_resource::CredentialSuspendOutcome::Suspended
    );
    match waiter.await.expect("waiter task") {
        Err(LimitedError::Limit(error)) => assert!(
            matches!(
                error.kind(),
                ErrorKind::CredentialUnavailable {
                    reason: nebula_resource::CredentialUnavailableReason::ReauthRequired,
                    ..
                }
            ),
            "expected CredentialUnavailable, got {error:?}"
        ),
        other => panic!("expected a refused limit wait, got {other:?}"),
    }
    assert!(
        started.elapsed() < PAUSE,
        "the wait ended at the suspension, not at the end of the pause"
    );
}
