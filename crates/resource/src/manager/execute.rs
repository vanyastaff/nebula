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

/// Validates pool config invariants at registration time.
///
/// Catches obviously broken configs (`max_size == 0`, `min_size > max_size`)
/// before they reach [`PoolRuntime`](crate::runtime::pool::PoolRuntime), so
/// warmup never inflates beyond `max_size` and callers cannot deadlock on an
/// empty semaphore (#390).
pub(super) fn validate_pool_config(
    cfg: &crate::topology::pooled::config::Config,
) -> Result<(), Error> {
    if cfg.max_size == 0 {
        return Err(Error::permanent("pool max_size must be > 0"));
    }
    if cfg.min_size > cfg.max_size {
        return Err(Error::permanent(format!(
            "pool min_size ({}) must be <= max_size ({})",
            cfg.min_size, cfg.max_size,
        )));
    }
    Ok(())
}
