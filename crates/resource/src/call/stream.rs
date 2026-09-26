//! Streaming units: an operation that hands items to its caller while the
//! unit runs, run on the ordinary unit runtime through a private
//! [`Operation`] adapter.

use std::{
    fmt,
    future::Future,
    num::{NonZeroU32, NonZeroUsize},
    time::Instant,
};

use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

use super::{
    Operation,
    cost::Effect,
    error::OpError,
    managed::{Managed, OpCx, Unit},
    pin::PinSlots,
};
use crate::{error::ErrorKind, resource::Provider};

/// A provider call that yields items while it runs, submitted with
/// [`Managed::submit_streaming`].
///
/// For a response read in chunks, a subscription, a long poll. It runs as
/// one ordinary [`Unit`]: the same lazy start, unit slot, deadline, attempt
/// admission, pinned credential slots and settled outcome as
/// [`Managed::submit`]. Every provider request still goes through
/// [`OpCx::attempt`], and the unit settles once, with [`run`](Self::run)'s
/// result. The only difference is the [`StreamSink`] it sends items into,
/// and the [`Streaming`] handle the caller pulls them from.
///
/// # Delivery
///
/// - The buffer between them is bounded: [`StreamSink::send`] waits while
///   the caller holds `capacity` unread items, so a slow consumer slows the
///   operation down — and through it the provider (for HTTP, TCP
///   backpressure) — instead of buffering without bound. The wait is still
///   bounded by the unit's deadline.
/// - Mid-stream failures are never items. An operation that fails returns
///   its [`OpError`]; the caller first receives every item already sent,
///   then that error once, stamped with the unit's settled state, then
///   `None`.
/// - Only the operation's return ends the stream. Items sent before the
///   unit settled are delivered; nothing is accepted after.
///
/// # Ending early
///
/// - Dropped before its first [`next`](Streaming::next), the unit never
///   ran, as for any unit.
/// - [`Streaming::cancel`] before the first grant settles the unit
///   `Cancelled` / `NotSent`. After a grant it tells the operation to stop:
///   [`StreamSink::closed`] resolves and every later send fails.
/// - Dropping the [`Streaming`] handle mid-stream does the same: the
///   operation sees [`ConsumerGone`] on its next send, or
///   [`StreamSink::closed`] while it waits on the provider, and ends. The
///   runtime still settles the unit, and the lease is released after it
///   ends (Design DX-API.md:114).
/// - Honouring the lease closing is the operation's choice, as for any
///   unit (Design CONTRACT.md:57): the facade never aborts a granted
///   attempt. A long-lived stream selects on [`OpCx::closing`] so a
///   removal or a shutdown drain does not wait for it.
///
/// The 5-minute [`UNIT_DEADLINE_CAP`](super::UNIT_DEADLINE_CAP) applies to
/// a streaming unit too; a stream that must outlive it is out of scope
/// until the package fixes an interval profile.
///
/// ```
/// use nebula_resource::{
///     PinSlots, Provider,
///     call::{Cost, Effect, OpCx, OpError, SentState, StreamOperation, StreamSink},
/// };
///
/// /// Counts down from the instance's value, one item per step.
/// struct CountDown;
///
/// impl<R> StreamOperation<R> for CountDown
/// where
///     R: Provider<Instance = u64> + PinSlots,
/// {
///     type Item = u64;
///     type Output = ();
///     const EFFECT: Effect = Effect::Read;
///
///     async fn run(self, cx: &mut OpCx<'_, R>, mut sink: StreamSink<u64>) -> Result<(), OpError> {
///         let attempt = cx.attempt(Cost::ONE).await?;
///         let start = *attempt.instance();
///         attempt.settle(SentState::Sent);
///         for value in (0..=start).rev() {
///             // `ConsumerGone` converts into a `Cancelled` error.
///             sink.send(value).await?;
///         }
///         Ok(())
///     }
/// }
/// ```
pub trait StreamOperation<R: Provider + PinSlots>: Send + 'static {
    /// What the stream yields.
    type Item: Send + 'static;

    /// What a successful unit yields once the stream ends.
    type Output: Send + 'static;

    /// What repeating the call does to the provider. `Write` unless the
    /// operation declares otherwise.
    const EFFECT: Effect = Effect::Write;

    /// How many attempts one unit may be granted; one by default.
    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::MIN
    }

    /// Runs the call, sending items into `sink`.
    fn run(
        self,
        cx: &mut OpCx<'_, R>,
        sink: StreamSink<Self::Item>,
    ) -> impl Future<Output = Result<Self::Output, OpError>> + Send;
}

/// The consumer of a stream is gone: its [`Streaming`] handle was dropped or
/// cancelled. Converts into a `Cancelled` [`OpError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsumerGone;

impl fmt::Display for ConsumerGone {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the stream's consumer is gone")
    }
}

impl std::error::Error for ConsumerGone {}

impl From<ConsumerGone> for OpError {
    fn from(_: ConsumerGone) -> Self {
        OpError::new(ErrorKind::Cancelled, "the stream's consumer is gone")
    }
}

/// Where a [`StreamOperation`] sends its items: a bounded channel to the
/// unit's [`Streaming`] handle.
pub struct StreamSink<T> {
    items: mpsc::Sender<T>,
    cancel: CancellationToken,
}

impl<T> fmt::Debug for StreamSink<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamSink")
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

impl<T> StreamSink<T> {
    /// Sends one item, waiting while the consumer's buffer is full.
    ///
    /// # Errors
    ///
    /// [`ConsumerGone`] once the [`Streaming`] handle was dropped or
    /// cancelled; the item is dropped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it resolves drops the item unsent.
    pub async fn send(&mut self, item: T) -> Result<(), ConsumerGone> {
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => Err(ConsumerGone),
            sent = self.items.send(item) => sent.map_err(|_| ConsumerGone),
        }
    }

    /// Resolves once the consumer is gone. Select on it while waiting on the
    /// provider, so a dropped stream ends the unit promptly.
    pub async fn closed(&self) {
        tokio::select! {
            () = self.cancel.cancelled() => {},
            () = self.items.closed() => {},
        }
    }

    /// Whether the consumer is gone.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.cancel.is_cancelled() || self.items.is_closed()
    }
}

/// A [`StreamOperation`] run as an ordinary [`Operation`]: the sink travels
/// with the intent, so the unit runtime needs no streaming variant.
struct Streamed<O, T> {
    operation: O,
    sink: StreamSink<T>,
}

impl<R, O, T> Operation<R> for Streamed<O, T>
where
    R: Provider + PinSlots,
    O: StreamOperation<R, Item = T>,
    T: Send + 'static,
{
    type Output = O::Output;
    const EFFECT: Effect = O::EFFECT;

    fn max_attempts(&self) -> NonZeroU32 {
        self.operation.max_attempts()
    }

    async fn run(self, cx: &mut OpCx<'_, R>) -> Result<O::Output, OpError> {
        self.operation.run(cx, self.sink).await
    }
}

impl<R: Provider + PinSlots> Managed<R> {
    /// Submits `operation` as one streaming unit whose items reach the
    /// returned [`Streaming`] through a buffer of `capacity` items.
    ///
    /// The unit is as lazy as [`submit`](Self::submit)'s: nothing happens
    /// until the first [`Streaming::next`] or [`Streaming::finish`].
    pub fn submit_streaming<O: StreamOperation<R>>(
        &self,
        operation: O,
        capacity: NonZeroUsize,
    ) -> Streaming<O::Item, O::Output> {
        // Tokio's bounded channel refuses a buffer above its permit limit.
        let capacity = capacity.get().min(Semaphore::MAX_PERMITS);
        let (items, receiver) = mpsc::channel(capacity);
        let cancel = CancellationToken::new();
        let unit = self.submit(Streamed {
            operation,
            sink: StreamSink {
                items,
                cancel: cancel.clone(),
            },
        });
        Streaming {
            unit: Some(unit),
            items: receiver,
            cancel,
            outcome: None,
            error_yielded: false,
        }
    }
}

/// The caller's side of a streaming unit: its items, then its outcome.
///
/// Pull items with [`next`](Self::next); it also drives the lazy unit. A
/// failed unit's error arrives once, after every item it sent, stamped with
/// the unit's settled state. [`finish`](Self::finish) discards the remaining
/// items and returns the unit's output.
#[must_use = "a streaming unit does nothing until polled; dropped before its first poll it never runs"]
pub struct Streaming<T, O = ()> {
    /// `None` once the unit settled.
    unit: Option<Unit<O>>,
    items: mpsc::Receiver<T>,
    cancel: CancellationToken,
    outcome: Option<Result<O, OpError>>,
    error_yielded: bool,
}

impl<T, O> fmt::Debug for Streaming<T, O> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Streaming")
            .field("unit", &self.unit)
            .field("buffered", &self.items.len())
            .field("settled", &self.outcome.is_some())
            .finish_non_exhaustive()
    }
}

impl<T, O> Streaming<T, O> {
    /// The next item, or the unit's error once every item before it was
    /// delivered; `None` when the stream ended.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe: dropping the future loses no item and leaves the unit
    /// running.
    pub async fn next(&mut self) -> Option<Result<T, OpError>> {
        if let Some(unit) = self.unit.as_mut() {
            let settled = tokio::select! {
                biased;
                item = self.items.recv() => match item {
                    Some(item) => return Some(Ok(item)),
                    // The sink is gone with the operation: the unit is
                    // settling.
                    None => unit.await,
                },
                settled = &mut *unit => settled,
            };
            self.unit = None;
            // Nothing is accepted after the unit settled; buffered items are
            // still delivered.
            self.items.close();
            self.outcome = Some(settled);
        }
        if let Some(item) = self.items.recv().await {
            return Some(Ok(item));
        }
        match &self.outcome {
            Some(Err(error)) if !self.error_yielded => {
                self.error_yielded = true;
                Some(Err(error.clone()))
            },
            _ => None,
        }
    }

    /// Runs the stream to its end, discarding the items not yet read, and
    /// returns the unit's output.
    ///
    /// # Errors
    ///
    /// The unit's error, even when [`next`](Self::next) already yielded it.
    pub async fn finish(mut self) -> Result<O, OpError> {
        while let Some(item) = self.next().await {
            item?;
        }
        self.outcome.take().unwrap_or_else(|| {
            Err(OpError::new(
                ErrorKind::Permanent,
                "streaming unit ended without an outcome",
            ))
        })
    }

    /// Cancels the stream. Before the unit's first grant it settles
    /// `Cancelled` and `NotSent`, as [`Unit::cancel`]; after it, the
    /// operation's sink closes and the operation ends at its next send or
    /// [`StreamSink::closed`] check. Idempotent.
    pub fn cancel(&self) {
        if let Some(unit) = &self.unit {
            unit.cancel();
        }
        self.cancel.cancel();
    }

    /// Shortens the unit's deadline, as [`Unit::with_deadline`]; call it
    /// before the first [`next`](Self::next).
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.unit = self.unit.take().map(|unit| unit.with_deadline(deadline));
        self
    }
}

#[cfg(test)]
#[path = "../call_stream_tests.rs"]
mod tests;
