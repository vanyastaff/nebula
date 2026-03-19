//! Load shedding — immediately reject requests when the system is overloaded.

use std::future::Future;

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
pub async fn load_shed<T, E, S, Fut, F>(should_shed: S, f: F) -> Result<T, CallError<E>>
where
    S: Fn() -> bool,
    Fut: Future<Output = Result<T, E>>,
    F: FnOnce() -> Fut,
{
    if should_shed() {
        return Err(CallError::LoadShed);
    }
    f().await.map_err(CallError::Operation)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
