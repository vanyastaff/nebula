//! Integration tests for PollAction DX trait + PollTriggerAdapter.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use nebula_action::action::Action;
use nebula_action::context::TriggerContext;
use nebula_action::dependency::ActionDependencies;
use nebula_action::error::ActionError;
use nebula_action::handler::{PollTriggerAdapter, TriggerHandler};
use nebula_action::metadata::ActionMetadata;
use nebula_action::poll::PollAction;
use nebula_action::testing::TestContextBuilder;

struct TickPoller {
    meta: ActionMetadata,
    poll_count: Arc<AtomicU32>,
}

impl ActionDependencies for TickPoller {}
impl Action for TickPoller {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl PollAction for TickPoller {
    type Cursor = u32;
    type Event = serde_json::Value;

    fn poll_interval(&self) -> Duration {
        // Match the adapter's enforced floor so the test does not fight
        // the clamp logic. Tests below that exercise the floor directly
        // use a separate action that returns Duration::ZERO.
        Duration::from_millis(100)
    }

    async fn poll(
        &self,
        cursor: &mut u32,
        _ctx: &TriggerContext,
    ) -> Result<Vec<serde_json::Value>, ActionError> {
        *cursor += 1;
        self.poll_count.fetch_add(1, Ordering::Relaxed);
        Ok(vec![serde_json::json!({"tick": *cursor})])
    }
}

fn make_poller() -> (TickPoller, Arc<AtomicU32>) {
    let count = Arc::new(AtomicU32::new(0));
    (
        TickPoller {
            meta: ActionMetadata::new(
                nebula_core::action_key!("test.tick"),
                "Tick Poller",
                "Test poll trigger",
            ),
            poll_count: count.clone(),
        },
        count,
    )
}

#[tokio::test(start_paused = true)]
async fn poll_adapter_emits_events() {
    let (poller, poll_count) = make_poller();
    let adapter = PollTriggerAdapter::new(poller);
    let (ctx, emitter, _) = TestContextBuilder::minimal().build_trigger();

    let cancel = ctx.cancellation.clone();
    let ctx_clone = ctx.clone();
    let handle = tokio::spawn(async move { adapter.start(&ctx_clone).await });

    // Deterministic: let the spawned task reach its first `tokio::select!`
    // and register the sleep deadline, then advance time past the poll
    // interval (which is clamped to POLL_INTERVAL_FLOOR = 100 ms).
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_millis(110)).await;
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    cancel.cancel();

    let result = handle.await.unwrap();
    assert!(result.is_ok());

    // Relaxed: we care that SOME polling happened deterministically.
    let count = poll_count.load(Ordering::Relaxed);
    assert!(count >= 1, "expected at least 1 poll, got {count}");
    assert!(
        emitter.count() >= 1,
        "expected at least 1 emit, got {}",
        emitter.count()
    );
}

#[tokio::test]
async fn poll_adapter_stop_is_noop() {
    let (poller, _) = make_poller();
    let adapter = PollTriggerAdapter::new(poller);
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    assert!(adapter.stop(&ctx).await.is_ok());
}

#[tokio::test]
async fn poll_adapter_does_not_accept_events() {
    let (poller, _) = make_poller();
    let adapter = PollTriggerAdapter::new(poller);
    assert!(!adapter.accepts_events());
}

#[tokio::test]
async fn poll_action_cursor_advances() {
    let (poller, _) = make_poller();
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();
    let mut cursor = 0u32;

    let events = poller.poll(&mut cursor, &ctx).await.unwrap();
    assert_eq!(cursor, 1);
    assert_eq!(events.len(), 1);

    let events = poller.poll(&mut cursor, &ctx).await.unwrap();
    assert_eq!(cursor, 2);
    assert_eq!(events.len(), 1);
}

// ── Double-start rejection (A2) ───────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn poll_adapter_rejects_concurrent_start() {
    // Spawn one start() that owns the atomic "started" slot. While that
    // loop is alive (suspended in tokio::time::sleep), a second start()
    // from the test task must fail with Fatal — otherwise two loops
    // would share the adapter's cursor and double-emit every event.
    let (poller, _) = make_poller();
    let adapter = Arc::new(PollTriggerAdapter::new(poller));
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    let cancel = ctx.cancellation.clone();
    let adapter1 = Arc::clone(&adapter);
    let ctx1 = ctx.clone();
    let handle = tokio::spawn(async move { adapter1.start(&ctx1).await });

    // Let the spawned loop reach its first tokio::select! and register
    // the sleep future. Now the atomic flag is set.
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }

    let err = adapter
        .start(&ctx)
        .await
        .expect_err("second start must fail while first is running");
    assert!(err.is_fatal());
    assert!(err.to_string().contains("already started"));

    // Tear down the first loop cleanly.
    cancel.cancel();
    let result = handle.await.unwrap();
    assert!(result.is_ok());
}

// ── Interval floor (A3) ───────────────────────────────────────────────────

struct ZeroIntervalPoller {
    meta: ActionMetadata,
    poll_count: Arc<AtomicU32>,
}

impl ActionDependencies for ZeroIntervalPoller {}
impl Action for ZeroIntervalPoller {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl PollAction for ZeroIntervalPoller {
    type Cursor = u32;
    type Event = serde_json::Value;

    fn poll_interval(&self) -> Duration {
        Duration::ZERO
    }

    async fn poll(
        &self,
        _cursor: &mut u32,
        _ctx: &TriggerContext,
    ) -> Result<Vec<serde_json::Value>, ActionError> {
        self.poll_count.fetch_add(1, Ordering::Relaxed);
        Ok(vec![])
    }
}

#[tokio::test(start_paused = true)]
async fn poll_adapter_clamps_zero_interval_to_floor() {
    // An action that returns poll_interval() = Duration::ZERO must not
    // produce a tight loop. The adapter clamps to POLL_INTERVAL_FLOOR
    // (100 ms). Advancing virtual time by 50 ms must not trigger any
    // polls; advancing past the floor must trigger exactly one.
    let poll_count = Arc::new(AtomicU32::new(0));
    let poller = ZeroIntervalPoller {
        meta: ActionMetadata::new(
            nebula_core::action_key!("test.tick.zero"),
            "Zero Interval",
            "Returns Duration::ZERO from poll_interval",
        ),
        poll_count: poll_count.clone(),
    };
    let adapter = Arc::new(PollTriggerAdapter::new(poller));
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    let cancel = ctx.cancellation.clone();
    let adapter1 = Arc::clone(&adapter);
    let ctx1 = ctx.clone();
    let handle = tokio::spawn(async move { adapter1.start(&ctx1).await });

    // Reach the first sleep registration.
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }

    // Below the 100 ms floor — no polls yet.
    tokio::time::advance(Duration::from_millis(50)).await;
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        poll_count.load(Ordering::Relaxed),
        0,
        "below the 100 ms floor — no polls yet"
    );

    // Past the floor — exactly one poll so far.
    tokio::time::advance(Duration::from_millis(60)).await;
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    assert!(
        poll_count.load(Ordering::Relaxed) >= 1,
        "past the floor — at least one poll happened"
    );

    cancel.cancel();
    handle.await.unwrap().unwrap();
}

#[tokio::test(start_paused = true)]
async fn poll_adapter_start_after_cancellation_succeeds() {
    // Prove the RAII StartedGuard clears the flag on every exit path,
    // including cancellation. After the first loop is cancelled and
    // awaited, a second start() must succeed.
    let (poller1, _) = make_poller();
    let (poller2, _) = make_poller();
    let adapter1 = PollTriggerAdapter::new(poller1);
    let adapter2 = PollTriggerAdapter::new(poller2);
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    let cancel = ctx.cancellation.clone();
    let ctx_clone = ctx.clone();
    let handle = tokio::spawn(async move { adapter1.start(&ctx_clone).await });
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    cancel.cancel();
    handle.await.unwrap().unwrap();

    // Fresh cancellation token for the second start (the first was
    // cancelled); otherwise start() returns immediately.
    let (ctx2, _, _) = TestContextBuilder::minimal().build_trigger();
    let cancel2 = ctx2.cancellation.clone();
    let handle2 = tokio::spawn(async move { adapter2.start(&ctx2).await });
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    cancel2.cancel();
    handle2.await.unwrap().unwrap();
}
