//! Timeout pattern — wraps futures with a deadline, returning `CallError::Timeout`.

use futures::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout as tokio_timeout;

use crate::{
    CallError,
    sink::{MetricsSink, NoopSink, ResilienceEvent},
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
pub async fn timeout_with_sink<T, E, F>(
    duration: Duration,
    future: F,
    sink: &dyn MetricsSink,
) -> Result<T, CallError<E>>
where
    F: Future<Output = Result<T, E>>,
{
    match tokio_timeout(duration, future).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(CallError::Operation(e)),
        Err(_) => {
            sink.record(ResilienceEvent::TimeoutElapsed { duration });
            Err(CallError::Timeout(duration))
        }
    }
}

/// A timeout executor with an injectable [`MetricsSink`].
pub struct TimeoutExecutor {
    duration: Duration,
    sink: Arc<dyn MetricsSink>,
}

impl TimeoutExecutor {
    /// Create a new executor with the given duration and a noop sink.
    #[must_use]
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            sink: Arc::new(NoopSink),
        }
    }

    /// Inject a metrics sink.
    #[must_use]
    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    /// Execute `future` within the configured timeout.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Timeout)` on timeout or `Err(CallError::Operation)` on operation error.
    pub async fn call<T, E, F>(&self, future: F) -> Result<T, CallError<E>>
    where
        F: Future<Output = Result<T, E>>,
    {
        timeout_with_sink(self.duration, future, self.sink.as_ref()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallError, RecordingSink};

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
        assert_eq!(sink.count("timeout_elapsed"), 1);
    }

    #[tokio::test]
    async fn executor_emits_timeout_event() {
        let sink = RecordingSink::new();
        let executor = TimeoutExecutor::new(Duration::from_millis(10)).with_sink(sink.clone());

        let _ = executor
            .call(async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Err::<(), &str>("unreachable")
            })
            .await;

        assert_eq!(sink.count("timeout_elapsed"), 1);
    }
}
