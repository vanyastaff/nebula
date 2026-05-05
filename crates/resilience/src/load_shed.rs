//! Load shedding — immediately reject requests when the system is overloaded.

use std::future::Future;

use crate::{
    CallError, PolicyContext,
    sink::{MetricsSink, NoopSink, ResilienceEvent},
};

/// Shed load immediately when `should_shed()` returns `true`.
///
/// - If `should_shed()` returns `true` -> `Err(CallError::LoadShed)` without calling `f`.
/// - Otherwise -> executes `f()` and maps `Err(e)` to `Err(CallError::Operation(e))`.
///
/// # Errors
///
/// Returns `Err(CallError::LoadShed)` when the shed predicate fires,
/// or `Err(CallError::Operation)` if the operation itself fails.
///
/// # Examples
///
/// ```rust,no_run
/// use nebula_resilience::{CallError, load_shed};
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Shed when the system reports overload.
/// let overloaded = || true;
/// let result: Result<u32, CallError<()>> = load_shed(overloaded, || async { Ok(42) }).await;
/// assert!(matches!(result, Err(CallError::LoadShed)));
///
/// // Pass through when healthy.
/// let healthy = || false;
/// let result: Result<u32, CallError<()>> = load_shed(healthy, || async { Ok(42) }).await;
/// assert_eq!(result.unwrap(), 42);
/// # Ok(())
/// # }
/// ```
pub async fn load_shed<T, E, S, Fut, F>(should_shed: S, f: F) -> Result<T, CallError<E>>
where
    S: Fn() -> bool,
    Fut: Future<Output = Result<T, E>>,
    F: FnOnce() -> Fut,
{
    load_shed_with_sink(should_shed, f, &NoopSink).await
}

/// Like [`load_shed`] but emits [`ResilienceEvent::LoadShed`] via `sink`.
///
/// # Errors
///
/// Returns `Err(CallError::LoadShed)` when the shed predicate fires,
/// or `Err(CallError::Operation)` if the operation itself fails.
pub async fn load_shed_with_sink<T, E, S, Fut, F>(
    should_shed: S,
    f: F,
    sink: &dyn MetricsSink,
) -> Result<T, CallError<E>>
where
    S: Fn() -> bool,
    Fut: Future<Output = Result<T, E>>,
    F: FnOnce() -> Fut,
{
    if should_shed() {
        sink.record(ResilienceEvent::LoadShed);
        return Err(CallError::LoadShed);
    }
    f().await.map_err(CallError::Operation)
}

/// Like [`load_shed`] but runs under a shared [`PolicyContext`].
///
/// Context cancellation/deadline wins before load-shed evaluation and while the
/// operation is in flight. This prevents shutdown or action deadline expiry from
/// being reported as ordinary overload.
///
/// # Errors
///
/// Returns `Err(CallError::Cancelled)` when the context is cancelled,
/// `Err(CallError::Timeout)` when the context deadline expires,
/// `Err(CallError::LoadShed)` when the shed predicate fires, or
/// `Err(CallError::Operation)` if the operation itself fails.
pub async fn load_shed_with_policy_context<T, E, S, Fut, F>(
    context: &PolicyContext,
    should_shed: S,
    f: F,
) -> Result<T, CallError<E>>
where
    S: FnOnce() -> bool + Send,
    Fut: Future<Output = Result<T, E>> + Send,
    F: FnOnce() -> Fut + Send,
    E: Send,
{
    load_shed_with_policy_context_and_sink(context, should_shed, f, &NoopSink).await
}

/// Like [`load_shed_with_policy_context`] but emits [`ResilienceEvent::LoadShed`]
/// via `sink` when the predicate rejects the call.
///
/// # Errors
///
/// Returns `Err(CallError::Cancelled)` when the context is cancelled,
/// `Err(CallError::Timeout)` when the context deadline expires,
/// `Err(CallError::LoadShed)` when the shed predicate fires, or
/// `Err(CallError::Operation)` if the operation itself fails.
pub async fn load_shed_with_policy_context_and_sink<T, E, S, Fut, F>(
    context: &PolicyContext,
    should_shed: S,
    f: F,
    sink: &dyn MetricsSink,
) -> Result<T, CallError<E>>
where
    S: FnOnce() -> bool + Send,
    Fut: Future<Output = Result<T, E>> + Send,
    F: FnOnce() -> Fut + Send,
    E: Send,
{
    context
        .run_result(async move {
            if should_shed() {
                sink.record(ResilienceEvent::LoadShed);
                return Err(CallError::LoadShed);
            }

            f().await.map_err(CallError::Operation)
        })
        .await
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use super::*;
    use crate::{RecordingSink, ResilienceEventKind};

    #[tokio::test]
    async fn load_shed_rejects_when_signaled() {
        let result: Result<u32, CallError<()>> = load_shed(|| true, || async { Ok(1u32) }).await;
        assert!(matches!(result, Err(CallError::LoadShed)));
    }

    #[tokio::test]
    async fn load_shed_passes_through_when_not_signaled() {
        let result: Result<u32, CallError<()>> = load_shed(|| false, || async { Ok(42u32) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn load_shed_propagates_operation_error() {
        let result: Result<u32, CallError<&str>> =
            load_shed(|| false, || async { Err("fail") }).await;
        assert!(matches!(result, Err(CallError::Operation("fail"))));
    }

    #[tokio::test]
    async fn load_shed_with_sink_emits_event() {
        let sink = RecordingSink::new();

        let result: Result<u32, CallError<()>> =
            load_shed_with_sink(|| true, || async { Ok(1u32) }, &sink).await;

        assert!(matches!(result, Err(CallError::LoadShed)));
        assert_eq!(sink.count(ResilienceEventKind::LoadShed), 1);
    }

    #[tokio::test]
    async fn policy_context_cancellation_skips_predicate_and_operation() {
        let cancellation = crate::CancellationContext::with_reason("shutdown");
        let context = PolicyContext::from_cancellation(cancellation.clone());
        cancellation.cancel();
        let predicate_called = Arc::new(AtomicBool::new(false));
        let operation_polled = Arc::new(AtomicBool::new(false));

        let predicate_called_for_call = Arc::clone(&predicate_called);
        let operation_polled_for_call = Arc::clone(&operation_polled);
        let result: Result<u32, CallError<()>> = load_shed_with_policy_context(
            &context,
            move || {
                predicate_called_for_call.store(true, Ordering::SeqCst);
                false
            },
            move || async move {
                operation_polled_for_call.store(true, Ordering::SeqCst);
                Ok(42)
            },
        )
        .await;

        assert!(matches!(result, Err(CallError::Cancelled { .. })));
        assert!(!predicate_called.load(Ordering::SeqCst));
        assert!(!operation_polled.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn policy_context_deadline_bounds_operation() {
        let context = PolicyContext::with_timeout(Duration::from_millis(1));

        let result: Result<u32, CallError<()>> = load_shed_with_policy_context(
            &context,
            || false,
            || async {
                tokio::time::sleep(Duration::from_mins(1)).await;
                Ok(42)
            },
        )
        .await;

        assert!(matches!(result, Err(CallError::Timeout(_))));
    }

    #[tokio::test]
    async fn policy_context_load_shed_emits_event() {
        let sink = RecordingSink::new();

        let result: Result<u32, CallError<()>> = load_shed_with_policy_context_and_sink(
            &PolicyContext::empty(),
            || true,
            || async { Ok(42) },
            &sink,
        )
        .await;

        assert!(matches!(result, Err(CallError::LoadShed)));
        assert_eq!(sink.count(ResilienceEventKind::LoadShed), 1);
    }
}
