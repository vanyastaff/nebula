//! Integration tests for PollAction DX trait + PollTriggerAdapter.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_action::{
    Action, ActionDependencies, ActionError, ActionMetadata, PollAction, PollConfig, PollResult,
    PollTriggerAdapter, TestContextBuilder, TriggerContext, TriggerHandler,
};

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

    fn poll_config(&self) -> PollConfig {
        PollConfig::fixed(Duration::from_millis(100))
    }

    async fn poll(
        &self,
        cursor: &mut u32,
        _ctx: &TriggerContext,
    ) -> Result<PollResult<serde_json::Value>, ActionError> {
        *cursor += 1;
        self.poll_count.fetch_add(1, Ordering::Relaxed);
        Ok(vec![serde_json::json!({"tick": *cursor})].into())
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

    let result = poller.poll(&mut cursor, &ctx).await.unwrap();
    assert_eq!(cursor, 1);
    assert_eq!(result.events.len(), 1);

    let result = poller.poll(&mut cursor, &ctx).await.unwrap();
    assert_eq!(cursor, 2);
    assert_eq!(result.events.len(), 1);
}

// ── Double-start rejection (A2) ───────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn poll_adapter_rejects_concurrent_start() {
    let (poller, _) = make_poller();
    let adapter = Arc::new(PollTriggerAdapter::new(poller));
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    let cancel = ctx.cancellation.clone();
    let adapter1 = Arc::clone(&adapter);
    let ctx1 = ctx.clone();
    let handle = tokio::spawn(async move { adapter1.start(&ctx1).await });

    for _ in 0..5 {
        tokio::task::yield_now().await;
    }

    let err = adapter
        .start(&ctx)
        .await
        .expect_err("second start must fail while first is running");
    assert!(err.is_fatal());
    assert!(err.to_string().contains("already started"));

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

    fn poll_config(&self) -> PollConfig {
        PollConfig::fixed(Duration::ZERO)
    }

    async fn poll(
        &self,
        _cursor: &mut u32,
        _ctx: &TriggerContext,
    ) -> Result<PollResult<serde_json::Value>, ActionError> {
        self.poll_count.fetch_add(1, Ordering::Relaxed);
        Ok(vec![].into())
    }
}

#[tokio::test(start_paused = true)]
async fn poll_adapter_clamps_zero_interval_to_floor() {
    let poll_count = Arc::new(AtomicU32::new(0));
    let poller = ZeroIntervalPoller {
        meta: ActionMetadata::new(
            nebula_core::action_key!("test.tick.zero"),
            "Zero Interval",
            "Returns Duration::ZERO from poll_config",
        ),
        poll_count: poll_count.clone(),
    };
    let adapter = Arc::new(PollTriggerAdapter::new(poller));
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    let cancel = ctx.cancellation.clone();
    let adapter1 = Arc::clone(&adapter);
    let ctx1 = ctx.clone();
    let handle = tokio::spawn(async move { adapter1.start(&ctx1).await });

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

    // Past the floor — at least one poll.
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

    let (ctx2, _, _) = TestContextBuilder::minimal().build_trigger();
    let cancel2 = ctx2.cancellation.clone();
    let handle2 = tokio::spawn(async move { adapter2.start(&ctx2).await });
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    cancel2.cancel();
    handle2.await.unwrap().unwrap();
}

// ── PollConfig constructors ───────────────────────────────────────────────

#[test]
fn poll_config_fixed_sets_equal_intervals() {
    let config = PollConfig::fixed(Duration::from_secs(5));
    assert_eq!(config.base_interval, Duration::from_secs(5));
    assert_eq!(config.max_interval, Duration::from_secs(5));
    assert_eq!(config.backoff_factor, 1.0);
    assert_eq!(config.jitter, 0.0);
}

#[test]
fn poll_config_with_backoff_includes_jitter() {
    let config = PollConfig::with_backoff(Duration::from_secs(10), Duration::from_secs(600), 2.0);
    assert_eq!(config.base_interval, Duration::from_secs(10));
    assert_eq!(config.max_interval, Duration::from_secs(600));
    assert_eq!(config.backoff_factor, 2.0);
    assert!(config.jitter > 0.0);
}

#[test]
fn poll_config_jitter_clamped() {
    let config = PollConfig::default().with_jitter(0.9);
    assert_eq!(config.jitter, 0.5);

    let config = PollConfig::default().with_jitter(-1.0);
    assert_eq!(config.jitter, 0.0);
}

#[test]
fn poll_config_backoff_factor_clamped_to_one() {
    let config = PollConfig::with_backoff(Duration::from_secs(1), Duration::from_secs(60), 0.5);
    assert_eq!(config.backoff_factor, 1.0);
}

// ── PollResult ergonomics ─────────────────────────────────────────────────

#[test]
fn poll_result_from_vec() {
    let result: PollResult<i32> = vec![1, 2, 3].into();
    assert_eq!(result.events.len(), 3);
    assert!(result.override_next.is_none());
}

#[test]
fn poll_result_with_override() {
    let result: PollResult<i32> =
        PollResult::new(vec![1]).with_override_next(Duration::from_secs(60));
    assert_eq!(result.override_next, Some(Duration::from_secs(60)));
}
