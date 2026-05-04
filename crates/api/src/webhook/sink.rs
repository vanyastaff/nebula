//! Engine-facing sink for slug-routed webhook events.
//!
//! The dispatcher does not own the engine. Instead it pushes each
//! validated [`TriggerEvent`] through a [`WebhookTriggerSink`] —
//! whoever wires the dispatcher into the running runtime decides what
//! "the engine" actually means.
//!
//! Two impls ship here:
//!
//! - [`MpscSink`] — backed by a `tokio::sync::mpsc` channel. Used by integration tests so they can
//!   `recv()` the dispatched event and assert "engine received the trigger event" without booting a
//!   real runtime.
//! - [`NoopSink`] — accepts and drops every event. Default for the in-memory dispatcher when no
//!   sink has been wired.
//!
//! Production wiring (out of scope for M3.3) will provide an
//! engine-backed sink that hands the event to a `TriggerHandler` or
//! enqueues a `trigger_events` row for the storage-layer dispatcher.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::TriggerEvent;
use tokio::sync::{Mutex, mpsc};

use super::error::{TriggerCoordinates, WebhookEnqueueError};

/// Boundary the dispatcher uses to hand events to the engine layer.
///
/// Implementations must not hold a transient lock across `await` for
/// longer than necessary — the dispatcher calls `enqueue` on the HTTP
/// request path and a slow sink delays the 202 response.
#[async_trait]
pub trait WebhookTriggerSink: Send + Sync + std::fmt::Debug {
    /// Push an event into the engine-side queue.
    ///
    /// `coordinates` accompany the event for diagnostics; the engine
    /// already has them via the registration but threading them in
    /// keeps log spans complete when the sink itself emits.
    ///
    /// # Errors
    ///
    /// Returns [`WebhookEnqueueError`] when the sink rejects the event
    /// (closed receiver, saturated bounded queue). The dispatcher maps
    /// these to 5xx responses; the caller is not expected to retry.
    async fn enqueue(
        &self,
        coordinates: &TriggerCoordinates,
        event: TriggerEvent,
    ) -> Result<(), WebhookEnqueueError>;
}

/// Sink that drops every event after recording its arrival count.
///
/// Default for the in-memory dispatcher when no engine wiring has been
/// installed yet. The dispatch path still validates auth and increments
/// the counter, so observability dashboards can detect "registered but
/// unwired" triggers without inspecting code.
#[derive(Debug, Default)]
pub struct NoopSink {
    accepted: Arc<std::sync::atomic::AtomicU64>,
}

impl NoopSink {
    /// Build a fresh sink with a zeroed counter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of events the sink has accepted since construction.
    /// Useful in tests and for observability checks.
    #[must_use]
    pub fn accepted(&self) -> u64 {
        self.accepted.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[async_trait]
impl WebhookTriggerSink for NoopSink {
    async fn enqueue(
        &self,
        _coordinates: &TriggerCoordinates,
        _event: TriggerEvent,
    ) -> Result<(), WebhookEnqueueError> {
        self.accepted
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(())
    }
}

/// Mpsc-backed sink used by integration tests.
///
/// The producer half is the [`WebhookTriggerSink`]; the consumer half
/// is exposed via [`MpscSink::take_receiver`] (single-consumer
/// semantics) so the test can `recv()` to assert delivery.
#[derive(Debug)]
pub struct MpscSink {
    sender: mpsc::Sender<DispatchedEvent>,
    receiver: Mutex<Option<mpsc::Receiver<DispatchedEvent>>>,
}

/// Pair of `(coordinates, event)` delivered through [`MpscSink`].
#[derive(Debug)]
pub struct DispatchedEvent {
    /// Slug tuple associated with the trigger that fired.
    pub coordinates: TriggerCoordinates,
    /// Event handed to the sink by the dispatcher.
    pub event: TriggerEvent,
}

impl MpscSink {
    /// Build a sink with the given channel capacity. `capacity` of `0`
    /// is rejected by the underlying `mpsc::channel`, so callers should
    /// pass at least `1`.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        Self {
            sender: tx,
            receiver: Mutex::new(Some(rx)),
        }
    }

    /// Take the receiver out of the sink. Returns `None` on subsequent
    /// calls — the receiver is single-consumer by construction.
    pub async fn take_receiver(&self) -> Option<mpsc::Receiver<DispatchedEvent>> {
        self.receiver.lock().await.take()
    }
}

#[async_trait]
impl WebhookTriggerSink for MpscSink {
    async fn enqueue(
        &self,
        coordinates: &TriggerCoordinates,
        event: TriggerEvent,
    ) -> Result<(), WebhookEnqueueError> {
        let dispatched = DispatchedEvent {
            coordinates: coordinates.clone(),
            event,
        };
        // `try_send` distinguishes "closed" (permanent) from "full"
        // (transient back-pressure) so the dispatcher can pick the
        // right HTTP status. Using `send().await` would collapse both
        // into a single `SendError`, hiding the difference.
        match self.sender.try_send(dispatched) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(WebhookEnqueueError::SinkUnavailable {
                    trigger: coordinates.clone(),
                })
            },
            Err(mpsc::error::TrySendError::Full(_)) => Err(WebhookEnqueueError::SinkBackpressure {
                trigger: coordinates.clone(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use nebula_action::TriggerEvent;

    use super::*;

    fn coords() -> TriggerCoordinates {
        TriggerCoordinates::new("acme", "main", "github")
    }

    #[tokio::test]
    async fn noop_sink_accepts_and_counts() {
        let sink = NoopSink::new();
        sink.enqueue(&coords(), TriggerEvent::new(None, 7_u32))
            .await
            .unwrap();
        sink.enqueue(&coords(), TriggerEvent::new(None, 8_u32))
            .await
            .unwrap();
        assert_eq!(sink.accepted(), 2);
    }

    #[tokio::test]
    async fn mpsc_sink_delivers_event_to_receiver() {
        let sink = MpscSink::new(2);
        let mut rx = sink.take_receiver().await.expect("first take returns rx");

        sink.enqueue(
            &coords(),
            TriggerEvent::new(Some("delivery-1".into()), 42_u32),
        )
        .await
        .unwrap();

        let dispatched = rx.recv().await.expect("event delivered");
        assert_eq!(dispatched.coordinates, coords());
        assert_eq!(dispatched.event.id(), Some("delivery-1"));
    }

    #[tokio::test]
    async fn mpsc_sink_take_receiver_is_single_consumer() {
        let sink = MpscSink::new(1);
        assert!(sink.take_receiver().await.is_some());
        assert!(sink.take_receiver().await.is_none());
    }

    #[tokio::test]
    async fn mpsc_sink_reports_backpressure_when_full() {
        let sink = MpscSink::new(1);
        // Don't take the receiver — leave the channel buffered.
        sink.enqueue(&coords(), TriggerEvent::new(None, 1_u32))
            .await
            .unwrap();
        let err = sink
            .enqueue(&coords(), TriggerEvent::new(None, 2_u32))
            .await
            .unwrap_err();
        assert!(matches!(err, WebhookEnqueueError::SinkBackpressure { .. }));
    }

    #[tokio::test]
    async fn mpsc_sink_reports_unavailable_when_receiver_dropped() {
        let sink = MpscSink::new(1);
        let rx = sink.take_receiver().await.unwrap();
        drop(rx);
        let err = sink
            .enqueue(&coords(), TriggerEvent::new(None, 1_u32))
            .await
            .unwrap_err();
        assert!(matches!(err, WebhookEnqueueError::SinkUnavailable { .. }));
    }
}
