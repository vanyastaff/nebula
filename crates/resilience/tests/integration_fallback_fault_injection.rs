//! Fault-injection integration tests for fallback strategies.

use std::sync::Arc;
use std::time::Duration;

use nebula_resilience::patterns::fallback::{
    CacheFallback, ChainFallback, FallbackOperation, FallbackStrategy, FunctionFallback,
    ValueFallback,
};
use nebula_resilience::ResilienceError;

#[tokio::test]
async fn test_fault_injection_value_fallback_on_timeout() {
    let fallback = Arc::new(ValueFallback::new("value-fallback".to_string()));
    let operation = FallbackOperation::new(fallback);

    let result = operation
        .execute(|| async { Err::<String, _>(ResilienceError::timeout(Duration::from_millis(50))) })
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "value-fallback");
}

#[tokio::test]
async fn test_fault_injection_function_fallback_receives_original_error() {
    let fallback = Arc::new(FunctionFallback::new(|error: ResilienceError| async move {
        match error {
            ResilienceError::CircuitBreakerOpen { state, .. } => Ok(format!("fallback:{state}")),
            other => Err(ResilienceError::FallbackFailed {
                reason: format!("unexpected error type: {other}"),
                original_error: None,
            }),
        }
    }));

    let operation = FallbackOperation::new(fallback);
    let result = operation
        .execute(|| async {
            Err::<String, _>(ResilienceError::circuit_breaker_open("open"))
        })
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "fallback:open");
}

#[tokio::test]
async fn test_fault_injection_cache_fallback_uses_cached_value() {
    let fallback = Arc::new(CacheFallback::new());
    fallback.update("cached-value".to_string()).await;

    let operation = FallbackOperation::new(fallback);
    let result = operation
        .execute(|| async { Err::<String, _>(ResilienceError::custom("primary failed")) })
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "cached-value");
}

#[tokio::test]
async fn test_fault_injection_cache_fallback_expired_value_fails() {
    let fallback = Arc::new(CacheFallback::new().with_ttl(Duration::from_millis(5)));
    fallback.update("stale-value".to_string()).await;
    tokio::time::sleep(Duration::from_millis(10)).await;

    let operation = FallbackOperation::new(fallback);
    let result = operation
        .execute(|| async { Err::<String, _>(ResilienceError::custom("primary failed")) })
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        ResilienceError::FallbackFailed { reason, .. } => {
            assert!(reason.contains("Cache expired"));
        }
        other => panic!("Expected FallbackFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_fault_injection_cache_fallback_stale_if_error_returns_expired_value() {
    let fallback = Arc::new(
        CacheFallback::new()
            .with_ttl(Duration::from_millis(5))
            .with_stale_if_error(true),
    );
    fallback.update("stale-value".to_string()).await;
    tokio::time::sleep(Duration::from_millis(10)).await;

    let operation = FallbackOperation::new(fallback);
    let result = operation
        .execute(|| async { Err::<String, _>(ResilienceError::custom("primary failed")) })
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "stale-value");
}

#[tokio::test]
async fn test_fault_injection_chain_fallback_cascades_to_next_strategy() {
    let first = Arc::new(FunctionFallback::new(|_error: ResilienceError| async move {
        Err::<String, _>(ResilienceError::FallbackFailed {
            reason: "first fallback failed".to_string(),
            original_error: None,
        })
    }));
    let second = Arc::new(ValueFallback::new("chain-success".to_string()));

    let chain = Arc::new(
        ChainFallback::new()
            .add(first as Arc<dyn FallbackStrategy<String>>)
            .add(second as Arc<dyn FallbackStrategy<String>>),
    );

    let operation = FallbackOperation::new(chain);
    let result = operation
        .execute(|| async { Err::<String, _>(ResilienceError::custom("primary failed")) })
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "chain-success");
}

#[tokio::test]
async fn test_fault_injection_invalid_config_skips_fallback() {
    let fallback = Arc::new(ValueFallback::new("should-not-be-used".to_string()));
    let operation = FallbackOperation::new(fallback);

    let result = operation
        .execute(|| async {
            Err::<String, _>(ResilienceError::InvalidConfig {
                message: "bad policy".to_string(),
            })
        })
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        ResilienceError::InvalidConfig { message } => {
            assert_eq!(message, "bad policy");
        }
        other => panic!("Expected InvalidConfig, got: {other:?}"),
    }
}
