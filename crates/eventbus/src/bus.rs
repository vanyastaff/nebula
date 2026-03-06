//! Generic broadcast event bus.

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::broadcast;

use crate::EventFilter;
use crate::FilteredSubscriber;
use crate::PublishOutcome;
use crate::policy::BackPressurePolicy;
use crate::scope::ScopedEvent;
use crate::scope::SubscriptionScope;
use crate::stats::EventBusStats;
use crate::subscriber::Subscriber;

/// Generic broadcast event bus parameterized by event type `E`.
///
/// Domain crates (e.g. telemetry, resource) own their event types and construct
/// `EventBus<ExecutionEvent>`, `EventBus<ResourceEvent>`, etc. This crate does
/// not define any domain event types.
///
/// # Performance
///
/// Hot path ([`send`](Self::send)): one `tokio::sync::broadcast::send` plus two
/// relaxed atomic increments for stats. No allocations; subscribers receive by clone.
/// Use [`send`](Self::send) (or [`emit`](Self::emit)) from engine/runtime hot paths.
///
/// # Back-pressure
///
/// When the buffer is full, behaviour is determined by [`BackPressurePolicy`]:
/// - **DropOldest** (default): send overwrites oldest; lagging subscribers skip events.
/// - **DropNewest**: the new event is dropped when the buffer is full.
/// - **Block**: use [`send_async`](Self::send_async) to wait up to a timeout for space.
///
/// # Delivery semantics
///
/// Best-effort only. No guaranteed delivery or global ordering across subscribers.
/// Emit path is non-blocking for `send()`; producers never block on subscriber speed.
#[derive(Debug)]
pub struct EventBus<E> {
    sender: broadcast::Sender<E>,
    policy: BackPressurePolicy,
    buffer_size: usize,
    sent_count: AtomicU64,
    dropped_count: AtomicU64,
}

impl<E: Clone + Send> EventBus<E> {
    /// Creates a new event bus with the given buffer size and default
    /// [`BackPressurePolicy::DropOldest`] policy.
    ///
    /// # Panics
    ///
    /// Panics if `buffer_size` is zero.
    #[must_use]
    pub fn new(buffer_size: usize) -> Self {
        Self::with_policy(buffer_size, BackPressurePolicy::default())
    }

    /// Creates a new event bus with the given buffer size and back-pressure policy.
    ///
    /// # Panics
    ///
    /// Panics if `buffer_size` is zero.
    #[must_use]
    pub fn with_policy(buffer_size: usize, policy: BackPressurePolicy) -> Self {
        assert!(buffer_size > 0, "EventBus buffer_size must be > 0");
        let (sender, _) = broadcast::channel(buffer_size);
        Self {
            sender,
            policy,
            buffer_size,
            sent_count: AtomicU64::new(0),
            dropped_count: AtomicU64::new(0),
        }
    }

    /// Sends an event to all current subscribers (non-blocking).
    ///
    /// When the buffer is full:
    /// - **DropOldest**: event is sent (oldest overwritten).
    /// - **DropNewest**: event is dropped and counted in stats.
    /// - **Block**: behaves as DropOldest; use [`send_async`](Self::send_async) to block.
    #[inline(always)]
    pub fn send(&self, event: E) -> PublishOutcome {
        let outcome = match &self.policy {
            BackPressurePolicy::DropOldest | BackPressurePolicy::Block { .. } => {
                self.publish_drop_oldest(event)
            }
            BackPressurePolicy::DropNewest => self.publish_drop_newest(event),
        };
        self.record_outcome(outcome);
        outcome
    }

    #[inline(always)]
    fn publish_drop_oldest(&self, event: E) -> PublishOutcome {
        match self.sender.send(event) {
            Ok(_) => PublishOutcome::Sent,
            Err(_) => PublishOutcome::DroppedNoSubscribers,
        }
    }

    #[inline(always)]
    fn publish_drop_newest(&self, event: E) -> PublishOutcome {
        if self.sender.receiver_count() == 0 {
            return PublishOutcome::DroppedNoSubscribers;
        }

        if self.sender.len() >= self.buffer_size {
            return PublishOutcome::DroppedByPolicy;
        }

        match self.sender.send(event) {
            Ok(_) => PublishOutcome::Sent,
            Err(_) => PublishOutcome::DroppedNoSubscribers,
        }
    }

    #[inline(always)]
    fn record_outcome(&self, outcome: PublishOutcome) {
        match outcome {
            PublishOutcome::Sent => {
                self.sent_count.fetch_add(1, Ordering::Relaxed);
            }
            PublishOutcome::DroppedNoSubscribers
            | PublishOutcome::DroppedByPolicy
            | PublishOutcome::DroppedTimeout => {
                self.dropped_count.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Alias for [`send`](Self::send). Use from engine/runtime when the term "emit" is preferred (matches INTERACTIONS).
    #[inline(always)]
    pub fn emit(&self, event: E) -> PublishOutcome {
        self.send(event)
    }

    /// Sends an event asynchronously, respecting [`BackPressurePolicy::Block`].
    ///
    /// For `DropOldest` and `DropNewest`, behaves like [`send`](Self::send).
    /// For `Block { timeout }`, waits up to `timeout` for buffer space before dropping.
    pub async fn send_async(&self, event: E) -> PublishOutcome
    where
        E: Clone,
    {
        match &self.policy {
            BackPressurePolicy::Block { timeout } => {
                let outcome = self.send_blocking(event, *timeout).await;
                self.record_outcome(outcome);
                outcome
            }
            _ => self.send(event),
        }
    }

    /// Alias for [`send_async`](Self::send_async) with emit naming.
    pub async fn emit_async(&self, event: E) -> PublishOutcome
    where
        E: Clone,
    {
        self.send_async(event).await
    }

    async fn send_blocking(&self, event: E, timeout: std::time::Duration) -> PublishOutcome
    where
        E: Clone,
    {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut event = Some(event);

        loop {
            if self.sender.receiver_count() == 0 {
                return PublishOutcome::DroppedNoSubscribers;
            }

            if self.sender.len() < self.buffer_size {
                let event = event
                    .take()
                    .expect("event should only be consumed once when capacity is available");
                return match self.sender.send(event) {
                    Ok(_) => PublishOutcome::Sent,
                    Err(_) => PublishOutcome::DroppedNoSubscribers,
                };
            }

            if tokio::time::Instant::now() >= deadline {
                return PublishOutcome::DroppedTimeout;
            }

            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }

    /// Returns `true` when at least one active subscriber exists.
    #[must_use]
    pub fn has_subscribers(&self) -> bool {
        self.sender.receiver_count() > 0
    }

    /// Subscribes to events.
    ///
    /// Returns a [`Subscriber`] that receives all events emitted after this call.
    /// If the subscriber falls behind by more than `buffer_size` events, it
    /// skips to the latest (handles `Lagged` internally).
    #[must_use]
    pub fn subscribe(&self) -> Subscriber<E> {
        Subscriber::new(self.sender.subscribe())
    }

    /// Subscribes with a custom filter predicate.
    ///
    /// Returned subscriber only yields events matching `filter`.
    #[must_use]
    pub fn subscribe_filtered(&self, filter: EventFilter<E>) -> FilteredSubscriber<E> {
        FilteredSubscriber::new(self.subscribe(), filter)
    }

    /// Subscribes to a specific scope.
    ///
    /// Requires event type metadata via [`ScopedEvent`].
    #[must_use]
    pub fn subscribe_scoped(&self, scope: SubscriptionScope) -> FilteredSubscriber<E>
    where
        E: ScopedEvent,
    {
        self.subscribe_filtered(EventFilter::by_scope(scope))
    }

    /// Returns a snapshot of event bus statistics.
    #[must_use]
    pub fn stats(&self) -> EventBusStats {
        EventBusStats {
            sent_count: self.sent_count.load(Ordering::Relaxed),
            dropped_count: self.dropped_count.load(Ordering::Relaxed),
            subscriber_count: self.sender.receiver_count(),
        }
    }

    /// Returns the configured buffer size.
    #[must_use]
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    /// Returns the configured back-pressure policy.
    #[must_use]
    pub fn policy(&self) -> &BackPressurePolicy {
        &self.policy
    }
}

impl<E: Clone + Send> Default for EventBus<E> {
    /// Default buffer size is 1024 (matches common resource/telemetry usage).
    fn default() -> Self {
        Self::new(1024)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestEvent(u64);

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ScopedTestEvent {
        workflow_id: Option<String>,
        execution_id: Option<String>,
        resource_id: Option<String>,
        payload: u64,
    }

    impl ScopedEvent for ScopedTestEvent {
        fn workflow_id(&self) -> Option<&str> {
            self.workflow_id.as_deref()
        }

        fn execution_id(&self) -> Option<&str> {
            self.execution_id.as_deref()
        }

        fn resource_id(&self) -> Option<&str> {
            self.resource_id.as_deref()
        }
    }

    #[test]
    fn send_without_subscribers_does_not_panic() {
        let bus = EventBus::<TestEvent>::new(16);
        bus.send(TestEvent(1));
        let stats = bus.stats();
        assert_eq!(stats.sent_count, 0);
        assert_eq!(stats.dropped_count, 1);
        assert_eq!(stats.subscriber_count, 0);
    }

    #[test]
    fn subscriber_receives_via_try_recv() {
        let bus = EventBus::<TestEvent>::new(16);
        let mut sub = bus.subscribe();
        bus.send(TestEvent(42));
        let event = sub.try_recv().expect("should receive event");
        assert_eq!(event, TestEvent(42));
        assert_eq!(bus.stats().sent_count, 1);
    }

    #[test]
    fn emit_is_alias_for_send() {
        let bus = EventBus::<TestEvent>::new(16);
        let mut sub = bus.subscribe();
        bus.emit(TestEvent(99));
        assert_eq!(sub.try_recv(), Some(TestEvent(99)));
        assert_eq!(bus.stats().sent_count, 1);
    }

    #[tokio::test]
    async fn subscriber_receives_via_recv() {
        let bus = EventBus::<TestEvent>::new(16);
        let mut sub = bus.subscribe();
        bus.send(TestEvent(7));
        let event = sub.recv().await.expect("should receive event");
        assert_eq!(event, TestEvent(7));
    }

    #[test]
    fn multiple_subscribers_each_get_copy() {
        let bus = EventBus::<TestEvent>::new(16);
        let mut sub1 = bus.subscribe();
        let mut sub2 = bus.subscribe();
        bus.send(TestEvent(1));
        assert_eq!(sub1.try_recv(), Some(TestEvent(1)));
        assert_eq!(sub2.try_recv(), Some(TestEvent(1)));
        assert_eq!(bus.stats().subscriber_count, 2);
    }

    #[test]
    fn stats_initial() {
        let bus = EventBus::<TestEvent>::new(8);
        let stats = bus.stats();
        assert_eq!(stats.sent_count, 0);
        assert_eq!(stats.dropped_count, 0);
        assert_eq!(stats.subscriber_count, 0);
    }

    #[test]
    fn drop_newest_no_subscribers_drops() {
        let bus = EventBus::<TestEvent>::with_policy(4, BackPressurePolicy::DropNewest);
        let outcome = bus.send(TestEvent(1));
        assert_eq!(outcome, PublishOutcome::DroppedNoSubscribers);
        let stats = bus.stats();
        assert_eq!(stats.dropped_count, 1);
        assert_eq!(stats.sent_count, 0);
    }

    #[tokio::test]
    async fn drop_newest_with_subscriber_sends() {
        let bus = EventBus::<TestEvent>::with_policy(4, BackPressurePolicy::DropNewest);
        let mut sub = bus.subscribe();
        bus.send(TestEvent(1));
        let event = sub.recv().await.expect("should receive");
        assert_eq!(event, TestEvent(1));
        assert_eq!(bus.stats().sent_count, 1);
        assert_eq!(bus.stats().dropped_count, 0);
    }

    #[tokio::test]
    async fn block_policy_send_async_succeeds_with_subscriber() {
        let bus = EventBus::<TestEvent>::with_policy(
            4,
            BackPressurePolicy::Block {
                timeout: Duration::from_millis(100),
            },
        );
        let mut sub = bus.subscribe();
        let outcome = bus.send_async(TestEvent(99)).await;
        assert_eq!(outcome, PublishOutcome::Sent);
        let event = sub.recv().await.expect("should receive");
        assert_eq!(event, TestEvent(99));
    }

    #[tokio::test]
    async fn block_policy_send_async_drops_after_timeout_no_receivers() {
        let bus = EventBus::<TestEvent>::with_policy(
            4,
            BackPressurePolicy::Block {
                timeout: Duration::from_millis(10),
            },
        );
        let outcome = bus.send_async(TestEvent(1)).await;
        assert_eq!(outcome, PublishOutcome::DroppedNoSubscribers);
        assert_eq!(bus.stats().dropped_count, 1);
    }

    #[test]
    fn send_reports_sent_for_active_subscriber() {
        let bus = EventBus::<TestEvent>::new(8);
        let _sub = bus.subscribe();

        let outcome = bus.send(TestEvent(5));

        assert_eq!(outcome, PublishOutcome::Sent);
        assert_eq!(bus.stats().sent_count, 1);
        assert_eq!(bus.stats().dropped_count, 0);
    }

    #[test]
    fn has_subscribers_reflects_runtime_state() {
        let bus = EventBus::<TestEvent>::new(8);
        assert!(!bus.has_subscribers());

        let sub = bus.subscribe();
        assert!(bus.has_subscribers());

        drop(sub);
        assert!(!bus.has_subscribers());
    }

    #[test]
    fn subscriber_tracks_lagged_count() {
        let bus = EventBus::<TestEvent>::new(2);
        let mut sub = bus.subscribe();

        let _ = bus.send(TestEvent(1));
        let _ = bus.send(TestEvent(2));
        let _ = bus.send(TestEvent(3));

        assert_eq!(sub.try_recv(), Some(TestEvent(2)));
        assert_eq!(sub.lagged_count(), 1);
    }

    #[test]
    fn filtered_subscription_only_receives_matching_events() {
        let bus = EventBus::<TestEvent>::new(16);
        let mut filtered =
            bus.subscribe_filtered(EventFilter::custom(|event: &TestEvent| event.0 % 2 == 0));

        let _ = bus.send(TestEvent(1));
        let _ = bus.send(TestEvent(2));

        assert_eq!(filtered.try_recv(), Some(TestEvent(2)));
    }

    #[test]
    fn scoped_subscription_filters_by_workflow() {
        let bus = EventBus::<ScopedTestEvent>::new(16);
        let mut sub = bus.subscribe_scoped(SubscriptionScope::workflow("wf-1"));

        let _ = bus.send(ScopedTestEvent {
            workflow_id: Some("wf-2".to_string()),
            execution_id: None,
            resource_id: None,
            payload: 1,
        });
        let _ = bus.send(ScopedTestEvent {
            workflow_id: Some("wf-1".to_string()),
            execution_id: None,
            resource_id: None,
            payload: 2,
        });

        assert_eq!(
            sub.try_recv(),
            Some(ScopedTestEvent {
                workflow_id: Some("wf-1".to_string()),
                execution_id: None,
                resource_id: None,
                payload: 2,
            })
        );
    }

    #[test]
    fn policy_default_is_drop_oldest() {
        let policy = BackPressurePolicy::default();
        assert!(matches!(policy, BackPressurePolicy::DropOldest));
    }

    #[test]
    fn buffer_size_zero_panics() {
        let result = std::panic::catch_unwind(|| {
            let _ = EventBus::<TestEvent>::new(0);
        });
        assert!(result.is_err());
    }
}
