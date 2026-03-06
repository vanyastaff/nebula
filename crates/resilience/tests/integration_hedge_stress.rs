//! Stress and correctness tests for hedge executors.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use nebula_resilience::patterns::hedge::{
    AdaptiveHedgeExecutor, BimodalHedgeExecutor, HedgeConfig, HedgeExecutor,
};

#[tokio::test]
async fn test_hedge_executor_primary_fast_no_extra_hedges() {
    let config = HedgeConfig {
        hedge_delay: Duration::from_millis(20),
        max_hedges: 2,
        exponential_backoff: false,
        backoff_multiplier: 1.0,
    };
    let executor = HedgeExecutor::new(config);
    let calls = Arc::new(AtomicUsize::new(0));

    let result = executor
        .execute({
            let calls = Arc::clone(&calls);
            move || {
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    Ok::<_, nebula_resilience::ResilienceError>("primary-fast")
                }
            }
        })
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "primary-fast");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_hedge_executor_returns_first_completed_result() {
    let config = HedgeConfig {
        hedge_delay: Duration::from_millis(3),
        max_hedges: 1,
        exponential_backoff: false,
        backoff_multiplier: 1.0,
    };
    let executor = HedgeExecutor::new(config);
    let calls = Arc::new(AtomicUsize::new(0));

    let result = executor
        .execute({
            let calls = Arc::clone(&calls);
            move || {
                let calls = Arc::clone(&calls);
                async move {
                    let call_index = calls.fetch_add(1, Ordering::SeqCst);
                    if call_index == 0 {
                        tokio::time::sleep(Duration::from_millis(40)).await;
                        Ok::<_, nebula_resilience::ResilienceError>("slow-primary")
                    } else {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        Ok::<_, nebula_resilience::ResilienceError>("fast-hedge")
                    }
                }
            }
        })
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "fast-hedge");
    assert!(calls.load(Ordering::SeqCst) >= 2);
}

#[tokio::test]
async fn test_hedge_executor_stress_concurrent_requests() {
    let executor = Arc::new(HedgeExecutor::new(HedgeConfig {
        hedge_delay: Duration::from_millis(2),
        max_hedges: 2,
        exponential_backoff: true,
        backoff_multiplier: 2.0,
    }));

    let mut handles = Vec::new();
    for _ in 0..64 {
        let executor = Arc::clone(&executor);
        let handle = tokio::spawn(async move {
            let calls = Arc::new(AtomicUsize::new(0));
            executor
                .execute({
                    let calls = Arc::clone(&calls);
                    move || {
                        let calls = Arc::clone(&calls);
                        async move {
                            let call_index = calls.fetch_add(1, Ordering::SeqCst);
                            if call_index == 0 {
                                tokio::time::sleep(Duration::from_millis(25)).await;
                            } else {
                                tokio::time::sleep(Duration::from_millis(4)).await;
                            }
                            Ok::<_, nebula_resilience::ResilienceError>(42usize)
                        }
                    }
                })
                .await
        });
        handles.push(handle);
    }

    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }
}

#[tokio::test]
async fn test_adaptive_hedge_executor_completes_under_concurrency() {
    let executor = Arc::new(
        AdaptiveHedgeExecutor::new(HedgeConfig {
            hedge_delay: Duration::from_millis(5),
            max_hedges: 2,
            exponential_backoff: true,
            backoff_multiplier: 2.0,
        })
        .with_target_percentile(0.9)
        .expect("valid percentile"),
    );

    let mut handles = Vec::new();
    for i in 0..48usize {
        let executor = Arc::clone(&executor);
        let handle = tokio::spawn(async move {
            executor
                .execute(move || async move {
                    if i % 3 == 0 {
                        tokio::time::sleep(Duration::from_millis(18)).await;
                    } else {
                        tokio::time::sleep(Duration::from_millis(3)).await;
                    }
                    Ok::<_, nebula_resilience::ResilienceError>(i)
                })
                .await
        });
        handles.push(handle);
    }

    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result.is_ok());
    }
}

#[tokio::test]
async fn test_adaptive_hedge_executor_reduces_tail_after_warmup() {
    let executor = AdaptiveHedgeExecutor::new(HedgeConfig {
        hedge_delay: Duration::from_millis(10),
        max_hedges: 1,
        exponential_backoff: false,
        backoff_multiplier: 1.0,
    })
    .with_target_percentile(0.8)
    .expect("valid percentile");

    for _ in 0..8 {
        let _ = executor
            .execute(|| async {
                tokio::time::sleep(Duration::from_millis(12)).await;
                Ok::<_, nebula_resilience::ResilienceError>("warmup")
            })
            .await;
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let result = executor
        .execute({
            let calls = Arc::clone(&calls);
            move || {
                let calls = Arc::clone(&calls);
                async move {
                    let call_index = calls.fetch_add(1, Ordering::SeqCst);
                    if call_index == 0 {
                        tokio::time::sleep(Duration::from_millis(45)).await;
                        Ok::<_, nebula_resilience::ResilienceError>("slow-primary")
                    } else {
                        tokio::time::sleep(Duration::from_millis(2)).await;
                        Ok::<_, nebula_resilience::ResilienceError>("fast-hedge")
                    }
                }
            }
        })
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "fast-hedge");
    assert!(calls.load(Ordering::SeqCst) >= 2);
}

#[tokio::test]
async fn test_bimodal_hedge_executor_fast_path_uses_fast_mode() {
    let fast_config = HedgeConfig {
        hedge_delay: Duration::from_millis(20),
        max_hedges: 0,
        exponential_backoff: false,
        backoff_multiplier: 1.0,
    };
    let slow_config = HedgeConfig {
        hedge_delay: Duration::from_millis(1),
        max_hedges: 1,
        exponential_backoff: false,
        backoff_multiplier: 1.0,
    };
    let executor = BimodalHedgeExecutor::new(Duration::from_millis(8), fast_config, slow_config);
    let calls = Arc::new(AtomicUsize::new(0));

    let result = executor
        .execute({
            let calls = Arc::clone(&calls);
            move || {
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    Ok::<_, nebula_resilience::ResilienceError>("fast-mode")
                }
            }
        })
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "fast-mode");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_bimodal_hedge_executor_slow_path_allows_hedge_winner() {
    let fast_config = HedgeConfig {
        hedge_delay: Duration::from_millis(20),
        max_hedges: 0,
        exponential_backoff: false,
        backoff_multiplier: 1.0,
    };
    let slow_config = HedgeConfig {
        hedge_delay: Duration::from_millis(5),
        max_hedges: 1,
        exponential_backoff: false,
        backoff_multiplier: 1.0,
    };
    let executor = BimodalHedgeExecutor::new(Duration::from_millis(20), fast_config, slow_config);
    let calls = Arc::new(AtomicUsize::new(0));

    let result = executor
        .execute({
            let calls = Arc::clone(&calls);
            move || {
                let calls = Arc::clone(&calls);
                async move {
                    let call_index = calls.fetch_add(1, Ordering::SeqCst);
                    if call_index == 2 {
                        tokio::time::sleep(Duration::from_millis(2)).await;
                        Ok::<_, nebula_resilience::ResilienceError>("slow-hedge")
                    } else {
                        tokio::time::sleep(Duration::from_millis(120)).await;
                        Ok::<_, nebula_resilience::ResilienceError>("slow-primary")
                    }
                }
            }
        })
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "slow-hedge");
    assert!(calls.load(Ordering::SeqCst) >= 3);
}

#[tokio::test]
async fn test_bimodal_hedge_executor_side_effect_guard_prevents_duplicates() {
    let fast_config = HedgeConfig {
        hedge_delay: Duration::from_millis(20),
        max_hedges: 0,
        exponential_backoff: false,
        backoff_multiplier: 1.0,
    };
    let slow_config = HedgeConfig {
        hedge_delay: Duration::from_millis(5),
        max_hedges: 2,
        exponential_backoff: false,
        backoff_multiplier: 1.0,
    };
    let executor = BimodalHedgeExecutor::new(Duration::from_millis(20), fast_config, slow_config);
    let committed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let commits = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));

    let result = executor
        .execute({
            let committed = Arc::clone(&committed);
            let commits = Arc::clone(&commits);
            let calls = Arc::clone(&calls);
            move || {
                let committed = Arc::clone(&committed);
                let commits = Arc::clone(&commits);
                let calls = Arc::clone(&calls);
                async move {
                    let call_index = calls.fetch_add(1, Ordering::SeqCst);

                    if call_index == 0 {
                        tokio::time::sleep(Duration::from_millis(120)).await;
                        return Ok::<_, nebula_resilience::ResilienceError>("sample");
                    }

                    if committed
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                    {
                        commits.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(2)).await;
                        Ok::<_, nebula_resilience::ResilienceError>("committed")
                    } else {
                        tokio::time::sleep(Duration::from_millis(12)).await;
                        Err::<&'static str, _>(nebula_resilience::ResilienceError::custom(
                            "duplicate side effect",
                        ))
                    }
                }
            }
        })
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "committed");
    assert_eq!(commits.load(Ordering::SeqCst), 1);
    assert!(calls.load(Ordering::SeqCst) >= 2);
}
