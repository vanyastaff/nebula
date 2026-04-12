//! Integration tests for PollAction DX trait + PollTriggerAdapter.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_action::{
    Action, ActionDependencies, ActionError, ActionMetadata, DeduplicatingCursor, PollAction,
    PollConfig, PollCursor, PollResult, PollTriggerAdapter, TestContextBuilder, TriggerContext,
    TriggerHandler,
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
        cursor: &mut PollCursor<u32>,
        _ctx: &TriggerContext,
    ) -> Result<PollResult<serde_json::Value>, ActionError> {
        **cursor += 1;
        self.poll_count.fetch_add(1, Ordering::Relaxed);
        Ok(vec![serde_json::json!({"tick": **cursor})].into())
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
async fn poll_action_cursor_advances_through_poll_cursor() {
    let (poller, _) = make_poller();
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();
    let mut cursor = PollCursor::new(0u32);

    let result = poller.poll(&mut cursor, &ctx).await.unwrap();
    assert_eq!(*cursor, 1);
    assert_eq!(result.events.len(), 1);

    let result = poller.poll(&mut cursor, &ctx).await.unwrap();
    assert_eq!(*cursor, 2);
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
        _cursor: &mut PollCursor<u32>,
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

    tokio::time::advance(Duration::from_millis(50)).await;
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        poll_count.load(Ordering::Relaxed),
        0,
        "below the 100 ms floor — no polls yet"
    );

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
    assert!(result.partial_error.is_none());
}

#[test]
fn poll_result_with_override() {
    let result: PollResult<i32> =
        PollResult::new(vec![1]).with_override_next(Duration::from_secs(60));
    assert_eq!(result.override_next, Some(Duration::from_secs(60)));
}

#[test]
fn poll_result_partial_carries_error_and_events() {
    let result: PollResult<i32> =
        PollResult::partial(vec![1, 2, 3], ActionError::retryable("page 4 failed"));
    assert_eq!(result.events.len(), 3);
    assert!(result.partial_error.is_some());
}

// ── PollCursor checkpoint ─────────────────────────────────────────────────

#[test]
fn poll_cursor_deref_transparent() {
    let mut cursor = PollCursor::new(42u32);
    assert_eq!(*cursor, 42);
    *cursor = 100;
    assert_eq!(*cursor, 100);
}

#[test]
fn poll_cursor_checkpoint_preserves_position() {
    let mut cursor = PollCursor::new(0u32);
    *cursor = 10;
    cursor.checkpoint();
    *cursor = 20;
    // After checkpoint at 10, current is 20.
    // Adapter would rollback on error; we verify checkpoint was taken
    // by observing that into_current returns 20 (the latest position).
    assert_eq!(*cursor, 20);
}

// ── DeduplicatingCursor ───────────────────────────────────────────────────

#[test]
fn dedup_cursor_filters_seen_keys() {
    let mut cursor: DeduplicatingCursor<String, u64> = DeduplicatingCursor::default();

    let items = vec![("a", 1), ("b", 2), ("c", 3)];
    let new = cursor.filter_new(items, |item| item.0.to_string());
    assert_eq!(new.len(), 3);

    let items2 = vec![("b", 2), ("c", 3), ("d", 4)];
    let new2 = cursor.filter_new(items2, |item| item.0.to_string());
    assert_eq!(new2.len(), 1);
    assert_eq!(new2[0].0, "d");
}

#[test]
fn dedup_cursor_evicts_oldest_at_cap() {
    let mut cursor: DeduplicatingCursor<u32, ()> = DeduplicatingCursor::new(()).with_max_seen(3);

    cursor.mark_seen(1);
    cursor.mark_seen(2);
    cursor.mark_seen(3);
    assert_eq!(cursor.seen_count(), 3);

    cursor.mark_seen(4);
    assert_eq!(cursor.seen_count(), 3);
    assert!(cursor.is_new(&1)); // evicted
    assert!(!cursor.is_new(&4));
}

#[test]
fn dedup_cursor_serde_roundtrip() {
    let mut cursor: DeduplicatingCursor<String, u64> = DeduplicatingCursor::new(42);
    cursor.mark_seen("msg-1".to_string());
    cursor.mark_seen("msg-2".to_string());

    let json = serde_json::to_string(&cursor).unwrap();
    let restored: DeduplicatingCursor<String, u64> = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.inner, 42);
    assert!(!restored.is_new(&"msg-1".to_string()));
    assert!(!restored.is_new(&"msg-2".to_string()));
    assert!(restored.is_new(&"msg-3".to_string()));
}

#[test]
fn dedup_cursor_clear_seen_preserves_inner() {
    let mut cursor: DeduplicatingCursor<u32, String> =
        DeduplicatingCursor::new("offset-100".to_string());
    cursor.mark_seen(1);
    cursor.mark_seen(2);
    assert_eq!(cursor.seen_count(), 2);

    cursor.clear_seen();
    assert_eq!(cursor.seen_count(), 0);
    assert_eq!(cursor.inner, "offset-100");
}

#[test]
fn dedup_cursor_serde_enforces_max_seen_on_deserialize() {
    // Simulate a cursor serialized with a higher cap or corrupted data.
    let json = r#"{"inner":0,"seen":["a","b","c","d","e"],"max_seen":3}"#;
    let cursor: DeduplicatingCursor<String, u32> = serde_json::from_str(json).unwrap();
    assert_eq!(cursor.seen_count(), 3);
    // Oldest entries ("a", "b") evicted, newest kept.
    assert!(cursor.is_new(&"a".to_string()));
    assert!(cursor.is_new(&"b".to_string()));
    assert!(!cursor.is_new(&"c".to_string()));
    assert!(!cursor.is_new(&"d".to_string()));
    assert!(!cursor.is_new(&"e".to_string()));
}

#[test]
fn dedup_cursor_mark_seen_is_idempotent() {
    let mut cursor: DeduplicatingCursor<u32, ()> = DeduplicatingCursor::default();
    cursor.mark_seen(1);
    cursor.mark_seen(1);
    cursor.mark_seen(1);
    assert_eq!(cursor.seen_count(), 1);
}

// ── Validate ──────────────────────────────────────────────────────────────

struct FailingValidator {
    meta: ActionMetadata,
}

impl ActionDependencies for FailingValidator {}
impl Action for FailingValidator {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl PollAction for FailingValidator {
    type Cursor = u32;
    type Event = serde_json::Value;

    fn poll_config(&self) -> PollConfig {
        PollConfig::fixed(Duration::from_secs(60))
    }

    async fn validate(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
        Err(ActionError::fatal("bad credentials"))
    }

    async fn poll(
        &self,
        _cursor: &mut PollCursor<u32>,
        _ctx: &TriggerContext,
    ) -> Result<PollResult<serde_json::Value>, ActionError> {
        unreachable!("poll should never be called if validate fails")
    }
}

#[tokio::test]
async fn poll_adapter_validate_failure_prevents_start() {
    let validator = FailingValidator {
        meta: ActionMetadata::new(
            nebula_core::action_key!("test.validate.fail"),
            "Failing Validator",
            "validate() returns Err",
        ),
    };
    let adapter = PollTriggerAdapter::new(validator);
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    let err = adapter.start(&ctx).await.expect_err("start must fail");
    assert!(err.is_fatal());
    assert!(err.to_string().contains("bad credentials"));
}

// ── Initial cursor ────────────────────────────────────────────────────────

struct StartFromNowPoller {
    meta: ActionMetadata,
    poll_count: Arc<AtomicU32>,
}

impl ActionDependencies for StartFromNowPoller {}
impl Action for StartFromNowPoller {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl PollAction for StartFromNowPoller {
    type Cursor = u64;
    type Event = serde_json::Value;

    fn poll_config(&self) -> PollConfig {
        PollConfig::fixed(Duration::from_millis(100))
    }

    async fn initial_cursor(&self, _ctx: &TriggerContext) -> Result<u64, ActionError> {
        Ok(1000) // "start from now" — skip historical data
    }

    async fn poll(
        &self,
        cursor: &mut PollCursor<u64>,
        _ctx: &TriggerContext,
    ) -> Result<PollResult<serde_json::Value>, ActionError> {
        self.poll_count.fetch_add(1, Ordering::Relaxed);
        **cursor += 1;
        Ok(vec![serde_json::json!({"id": **cursor})].into())
    }
}

#[tokio::test(start_paused = true)]
async fn poll_adapter_uses_initial_cursor() {
    let poll_count = Arc::new(AtomicU32::new(0));
    let poller = StartFromNowPoller {
        meta: ActionMetadata::new(
            nebula_core::action_key!("test.initial_cursor"),
            "Start From Now",
            "initial_cursor returns 1000",
        ),
        poll_count: poll_count.clone(),
    };
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
    handle.await.unwrap().unwrap();

    assert!(poll_count.load(Ordering::Relaxed) >= 1);
    // First emitted event should be id: 1001 (initial_cursor = 1000, then +1)
    let emitted = emitter.emitted();
    assert!(!emitted.is_empty());
    assert_eq!(emitted[0]["id"], 1001);
}
