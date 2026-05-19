//! Resilience pipeline + register-time validation helpers used by the
//! [`Manager`](super::Manager) acquire and registration paths.

use std::future::Future;

use crate::{error::Error, integration::AcquireResilience};

/// Executes an async operation with optional timeout and retry from
/// [`AcquireResilience`] configuration.
///
/// Delegates to [`nebula_resilience::retry_with`] which handles exponential
/// backoff, wall-clock budget, retry-after hints, and `Classify`-based
/// error filtering automatically.
pub(super) async fn execute_with_resilience<F, Fut, T>(
    resilience: &Option<AcquireResilience>,
    mut operation: F,
) -> Result<T, Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, Error>> + Send,
{
    let Some(config) = resilience else {
        return operation().await;
    };

    // #383: `to_retry_config` returns `None` only if the underlying
    // `RetryConfig::new` rejects the clamped attempt count. That path is
    // unreachable today; if it ever fires we prefer to fall through to a
    // single un-retried attempt rather than panic the manager.
    let Some(retry_cfg) = config.to_retry_config() else {
        return operation().await;
    };
    nebula_resilience::retry_with(retry_cfg, operation)
        .await
        .map_err(|call_err| match call_err {
            nebula_resilience::CallError::Operation(e)
            | nebula_resilience::CallError::RetriesExhausted { last: e, .. } => e,
            nebula_resilience::CallError::Timeout(d) => {
                Error::transient(format!("acquire timed out after {d:?}"))
            },
            other => Error::transient(other.to_string()),
        })
}

// Pool `(min_size, max_size)` sanity (#390) is enforced at
// `PoolRuntime` construction, not as a separate register-time re-check:
// a `TopologyRuntime::Pool` can only be built by going through a
// `PoolRuntime` constructor. The registration path (operator-/JSON-
// derived config) must use the fallible `PoolRuntime::try_new`, which
// returns a typed `Error::permanent` so an invalid topology fails the
// registration safely instead of aborting the process;
// `PoolRuntime::new` keeps an equivalent assert for compile-time-known
// callers only (doctests, const fixtures). The former register-time
// soft-`Err` re-check on the deleted `register_pooled[_with]`
// shorthands is gone — the same check now lives in the constructor
// seam every caller already funnels through.
