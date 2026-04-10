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
use nebula_action::testing::TestContextBuilder;
use nebula_action::trigger::PollAction;

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
        Duration::from_millis(10)
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
    // and register the sleep deadline, then advance time past the poll interval.
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_millis(15)).await;
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
