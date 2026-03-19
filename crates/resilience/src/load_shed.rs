//! Load shedding — immediately reject requests when the system is overloaded.

use std::future::Future;
use std::pin::Pin;

use crate::CallError;

/// Shed load immediately when `should_shed()` returns `true`.
///
/// - If `should_shed()` returns `true` -> `Err(CallError::LoadShed)` without calling `f`.
/// - Otherwise -> executes `f()` and maps `Err(e)` to `Err(CallError::Operation(e))`.
///
/// # Errors
///
/// Returns `Err(CallError::LoadShed)` when the shed predicate fires,
/// or `Err(CallError::Operation)` if the operation itself fails.
pub async fn load_shed<T, E, S, F>(should_shed: S, f: F) -> Result<T, CallError<E>>
where
    S: Fn() -> bool,
    F: FnOnce() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
{
    if should_shed() {
        return Err(CallError::LoadShed);
    }
    f().await.map_err(CallError::Operation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn load_shed_rejects_when_signaled() {
        let shed = Arc::new(AtomicBool::new(true));
        let s = shed.clone();
        let result: Result<u32, CallError<()>> = load_shed(
            move || s.load(Ordering::SeqCst),
            || Box::pin(async { Ok(1u32) }),
        )
        .await;
        assert!(matches!(result, Err(CallError::LoadShed)));
    }

    #[tokio::test]
    async fn load_shed_passes_through_when_not_signaled() {
        let result: Result<u32, CallError<()>> =
            load_shed(|| false, || Box::pin(async { Ok(42u32) })).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn load_shed_propagates_operation_error() {
        let result: Result<u32, CallError<&str>> =
            load_shed(|| false, || Box::pin(async { Err("fail") })).await;
        assert!(matches!(result, Err(CallError::Operation("fail"))));
    }
}
