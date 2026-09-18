//! Timeout pattern — wraps futures with a deadline, returning `CallError::Timeout`.

use std::{fmt, future::Future, sync::Arc, time::Duration};

use tokio::time::timeout as tokio_timeout;

use crate::{
    CallContext, CallError, ConfigError,
    events::{EventSink, NoopSink, ResilienceEvent},
};

/// Execute `future` with a timeout.
///
/// - Operation success  -> `Ok(T)`
/// - Operation error    -> `Err(CallError::Operation(e))`
/// - Timeout elapsed    -> `Err(CallError::Timeout(duration))`
///
/// # Errors
///
/// Returns `Err(CallError::Timeout)` on timeout or `Err(CallError::Operation)` on operation error.
///
/// # Cancel safety
///
/// Cancel-safe with respect to this crate: dropping the returned future
/// drops the in-flight operation at its current `.await` and discards the
/// timeout bookkeeping — no crate-owned state is left partially mutated,
/// and no work is detached via `spawn`. Whether a *partially executed*
/// operation is safe to abandon is the supplied operation's own contract.
///
/// # Examples
///
/// ```rust,no_run
/// use std::time::Duration;
///
/// use nebula_resilience::{CallError, timeout};
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let value: Result<&str, CallError<&str>> =
///     timeout(Duration::from_millis(100), async { Ok("ready") }).await;
/// assert_eq!(value.unwrap(), "ready");
/// # Ok(())
/// # }
/// ```
pub async fn timeout<T, E, F>(duration: Duration, future: F) -> Result<T, CallError<E>>
where
    F: Future<Output = Result<T, E>>,
{
    timeout_with_sink(duration, future, &NoopSink).await
}

/// Like [`timeout`] but emits [`ResilienceEvent::TimeoutElapsed`] via `sink`.
///
/// # Errors
///
/// Returns `Err(CallError::Timeout)` on timeout or `Err(CallError::Operation)` on operation error.
///
/// # Cancel safety
///
/// Cancel-safe with respect to this crate: dropping the returned future
/// drops the in-flight operation at its current `.await` and discards the
/// timeout bookkeeping — no crate-owned state is left partially mutated,
/// and no work is detached via `spawn`. Whether a *partially executed*
/// operation is safe to abandon is the supplied operation's own contract.
pub async fn timeout_with_sink<T, E, F>(
    duration: Duration,
    future: F,
    sink: &dyn EventSink,
) -> Result<T, CallError<E>>
where
    F: Future<Output = Result<T, E>>,
{
    if duration.is_zero() {
        sink.record(ResilienceEvent::TimeoutElapsed { duration });
        return Err(CallError::Timeout(duration));
    }

    match tokio_timeout(duration, future).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(CallError::Operation(e)),
        Err(_) => {
            sink.record(ResilienceEvent::TimeoutElapsed { duration });
            Err(CallError::Timeout(duration))
        },
    }
}

/// Like [`timeout`] but also observes a shared [`CallContext`].
///
/// The effective deadline is the earlier of `duration` and the context deadline.
/// Context cancellation wins over timeout before and during the future.
///
/// # Errors
///
/// Returns `Err(CallError::Cancelled)` when the context is cancelled,
/// `Err(CallError::Timeout)` when either deadline expires, or
/// `Err(CallError::Operation)` if the operation itself fails.
///
/// # Cancel safety
///
/// Cancel-safe with respect to this crate: dropping the returned future
/// drops the in-flight operation at its current `.await` and discards the
/// timeout bookkeeping — no crate-owned state is left partially mutated,
/// and no work is detached via `spawn`. Whether a *partially executed*
/// operation is safe to abandon is the supplied operation's own contract.
pub async fn timeout_with_context<T, E, F>(
    context: &CallContext,
    duration: Duration,
    future: F,
) -> Result<T, CallError<E>>
where
    F: Future<Output = Result<T, E>> + Send,
    E: Send,
{
    timeout_with_context_and_sink(context, duration, future, &NoopSink).await
}

/// Like [`timeout_with_context`] but emits [`ResilienceEvent::TimeoutElapsed`]
/// via `sink` when the local timeout expires.
///
/// If the context deadline fires first, the returned error is still
/// `CallError::Timeout`, but this local timeout event is not emitted.
///
/// # Errors
///
/// Returns `Err(CallError::Cancelled)` when the context is cancelled,
/// `Err(CallError::Timeout)` when either deadline expires, or
/// `Err(CallError::Operation)` if the operation itself fails.
///
/// # Cancel safety
///
/// Cancel-safe with respect to this crate: dropping the returned future
/// drops the in-flight operation at its current `.await` and discards the
/// timeout bookkeeping — no crate-owned state is left partially mutated,
/// and no work is detached via `spawn`. Whether a *partially executed*
/// operation is safe to abandon is the supplied operation's own contract.
pub async fn timeout_with_context_and_sink<T, E, F>(
    context: &CallContext,
    duration: Duration,
    future: F,
    sink: &dyn EventSink,
) -> Result<T, CallError<E>>
where
    F: Future<Output = Result<T, E>> + Send,
    E: Send,
{
    context
        .run_result(timeout_with_sink(duration, future, sink))
        .await
}

/// A timeout executor with an injectable [`EventSink`].
///
/// Construct once and reuse across many calls when you want a stable timeout
/// budget plus uniform observability. For one-off use, prefer the free
/// [`timeout`] function.
///
/// # Examples
///
/// ```rust,no_run
/// use std::time::Duration;
///
/// use nebula_resilience::{CallError, RecordingSink, TimeoutExecutor};
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let sink = RecordingSink::new();
/// let executor = TimeoutExecutor::new(Duration::from_millis(50))
///     .expect("non-zero duration")
///     .with_sink(sink.clone());
///
/// let value: Result<&str, CallError<&str>> = executor.call(async { Ok("ready") }).await;
/// assert_eq!(value.unwrap(), "ready");
/// # Ok(())
/// # }
/// ```
pub struct TimeoutExecutor {
    duration: Duration,
    sink: Arc<dyn EventSink>,
}

impl fmt::Debug for TimeoutExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TimeoutExecutor")
            .field("duration", &self.duration)
            .finish_non_exhaustive()
    }
}

impl TimeoutExecutor {
    /// Creates a new executor with the given duration and a noop sink.
    ///
    /// A zero duration is rejected: it never polls the protected future, so it
    /// can only be a misconfigured workflow timeout. Immediate cancellation is
    /// what [`CallContext`] cancellation is for — this constructor refuses to
    /// express it as a timeout.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when `duration` is zero.
    pub fn new(duration: Duration) -> Result<Self, ConfigError> {
        if duration.is_zero() {
            return Err(ConfigError::new(
                "timeout.duration",
                "must be greater than zero",
            ));
        }

        Ok(Self {
            duration,
            sink: Arc::new(NoopSink),
        })
    }

    /// Inject a metrics sink.
    #[must_use]
    pub fn with_sink(mut self, sink: impl EventSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    /// Inject a shared metrics sink.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_shared_sink(mut self, sink: Arc<dyn EventSink>) -> Self {
        self.sink = sink;
        self
    }

    /// Execute `future` within the configured timeout.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Timeout)` on timeout or `Err(CallError::Operation)` on operation
    /// error.
    ///
    /// # Cancel safety
    ///
    /// Cancel-safe with respect to this crate: dropping the returned future
    /// drops the in-flight operation at its current `.await` and discards the
    /// timeout bookkeeping — no crate-owned state is left partially mutated,
    /// and no work is detached via `spawn`. Whether a *partially executed*
    /// operation is safe to abandon is the supplied operation's own contract.
    pub async fn call<T, E, F>(&self, future: F) -> Result<T, CallError<E>>
    where
        F: Future<Output = Result<T, E>>,
    {
        timeout_with_sink(self.duration, future, self.sink.as_ref()).await
    }

    /// Execute `future` within both the configured timeout and a shared policy context.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Cancelled)` when the context is cancelled,
    /// `Err(CallError::Timeout)` when either deadline expires, or
    /// `Err(CallError::Operation)` if the operation itself fails.
    ///
    /// # Cancel safety
    ///
    /// Cancel-safe with respect to this crate: dropping the returned future
    /// drops the in-flight operation at its current `.await` and discards the
    /// timeout bookkeeping — no crate-owned state is left partially mutated,
    /// and no work is detached via `spawn`. Whether a *partially executed*
    /// operation is safe to abandon is the supplied operation's own contract.
    pub async fn call_with_context<T, E, F>(
        &self,
        context: &CallContext,
        future: F,
    ) -> Result<T, CallError<E>>
    where
        F: Future<Output = Result<T, E>> + Send,
        E: Send,
    {
        timeout_with_context_and_sink(context, self.duration, future, self.sink.as_ref()).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use super::*;
    use crate::{CallError, CancellationContext, RecordingSink, ResilienceEventKind};

    #[tokio::test]
    async fn timeout_success() {
        let result = timeout(Duration::from_millis(100), async {
            Ok::<_, &str>("success")
        })
        .await;
        assert_eq!(result.unwrap(), "success");
    }

    #[tokio::test]
    async fn timeout_exceeded() {
        let result: Result<(), CallError<&str>> = timeout(Duration::from_millis(10), async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok(())
        })
        .await;

        assert!(matches!(result, Err(CallError::Timeout(d)) if d == Duration::from_millis(10)));
    }

    #[tokio::test]
    async fn zero_timeout_does_not_poll_future() {
        let polled = Arc::new(AtomicBool::new(false));
        let polled_for_call = Arc::clone(&polled);

        let result: Result<(), CallError<()>> = timeout(Duration::ZERO, async move {
            polled_for_call.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await;

        assert!(matches!(result, Err(CallError::Timeout(d)) if d.is_zero()));
        assert!(!polled.load(Ordering::SeqCst));
    }

    #[test]
    fn new_rejects_zero_timeout() {
        let err = TimeoutExecutor::new(Duration::ZERO).unwrap_err();
        assert_eq!(err.field, "timeout.duration");
    }

    #[tokio::test]
    async fn timeout_operation_error() {
        let result = timeout(Duration::from_millis(100), async {
            Err::<(), &str>("fail")
        })
        .await;

        assert!(matches!(result, Err(CallError::Operation("fail"))));
    }

    #[tokio::test]
    async fn emits_timeout_elapsed_event() {
        let sink = RecordingSink::new();
        let result = timeout_with_sink(
            Duration::from_millis(10),
            async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Err::<(), &str>("unreachable")
            },
            &sink,
        )
        .await;

        assert!(matches!(result, Err(CallError::Timeout(_))));
        assert_eq!(sink.count(ResilienceEventKind::TimeoutElapsed), 1);
    }

    #[tokio::test]
    async fn executor_emits_timeout_event() {
        let sink = RecordingSink::new();
        let executor = TimeoutExecutor::new(Duration::from_millis(10))
            .expect("non-zero duration")
            .with_sink(sink.clone());

        let _ = executor
            .call(async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Err::<(), &str>("unreachable")
            })
            .await;

        assert_eq!(sink.count(ResilienceEventKind::TimeoutElapsed), 1);
    }

    #[tokio::test]
    async fn call_context_cancellation_wins_without_polling_future() {
        let cancellation = CancellationContext::with_reason("shutdown");
        let context = CallContext::from_cancellation(cancellation.clone());
        cancellation.cancel();
        let polled = Arc::new(AtomicBool::new(false));
        let polled_for_call = Arc::clone(&polled);

        let result: Result<(), CallError<()>> =
            timeout_with_context(&context, Duration::from_secs(1), async move {
                polled_for_call.store(true, Ordering::SeqCst);
                Ok(())
            })
            .await;

        assert!(matches!(result, Err(CallError::Cancelled { .. })));
        assert!(!polled.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn executor_policy_context_deadline_bounds_call() {
        let sink = RecordingSink::new();
        let executor = TimeoutExecutor::new(Duration::from_mins(1))
            .expect("non-zero duration")
            .with_sink(sink.clone());
        let context = CallContext::with_timeout(Duration::from_millis(1));

        let result: Result<(), CallError<()>> = executor
            .call_with_context(&context, async {
                tokio::time::sleep(Duration::from_mins(1)).await;
                Ok(())
            })
            .await;

        assert!(matches!(result, Err(CallError::Timeout(_))));
        assert_eq!(sink.count(ResilienceEventKind::TimeoutElapsed), 0);
    }
}
