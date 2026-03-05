//! Retry storm-guard and jitter tuning integration tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nebula_resilience::{
    AggressiveCondition, FixedDelay, JitterPolicy, ResilienceError, RetryConfig, RetryStrategy,
};
use tokio::sync::Mutex;

#[tokio::test]
async fn test_retry_max_duration_caps_retry_storm() {
    let strategy = RetryStrategy::new(
        RetryConfig::new(FixedDelay::<20>::default(), AggressiveCondition::<100>::new())
            .with_jitter(JitterPolicy::None)
            .with_max_duration(Duration::from_millis(70)),
    )
    .expect("retry strategy config should be valid");

    let attempts = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();

    let result = strategy
        .execute({
            let attempts = Arc::clone(&attempts);
            move || {
                let attempts = Arc::clone(&attempts);
                async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>("transient")
                }
            }
        })
        .await;

    assert!(result.is_err(), "operation should fail after retry budget is exhausted");
    let elapsed = started.elapsed();
    let executed_attempts = attempts.load(Ordering::SeqCst);

    assert!(
        executed_attempts < 100,
        "max duration should cap attempts before aggressive max_attempts"
    );
    assert!(
        elapsed < Duration::from_millis(400),
        "retry loop should stop quickly when max_total_duration is configured"
    );
}

#[tokio::test]
async fn test_conservative_condition_blocks_terminal_error_storm() {
    let strategy = Arc::new(
        nebula_resilience::exponential_retry::<8>()
            .expect("conservative retry strategy should be constructible"),
    );

    let attempts = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for _ in 0..48 {
        let attempts = Arc::clone(&attempts);
        let strategy = Arc::clone(&strategy);
        handles.push(tokio::spawn(async move {
            strategy
                .execute_resilient(|| {
                    let attempts = Arc::clone(&attempts);
                    async move {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        Err::<(), _>(ResilienceError::InvalidConfig {
                            message: "terminal-config-error".to_string(),
                        })
                    }
                })
                .await
        }));
    }

    for handle in handles {
        let result = handle.await.expect("task join should succeed");
        assert!(result.is_err());
    }

    assert_eq!(
        attempts.load(Ordering::SeqCst),
        48,
        "terminal errors should not trigger retry amplification"
    );
}

#[tokio::test]
async fn test_full_jitter_desynchronizes_retry_wave() {
    async fn collect_second_attempt_offsets(jitter: JitterPolicy) -> Vec<u128> {
        let strategy = Arc::new(
            RetryStrategy::new(
            RetryConfig::new(FixedDelay::<20>::default(), AggressiveCondition::<3>::new())
                .with_jitter(jitter),
            )
            .expect("retry strategy config should be valid"),
        );

        let start = Instant::now();
        let second_attempt_offsets_ms: Arc<Mutex<Vec<u128>>> = Arc::new(Mutex::new(Vec::new()));
        let mut handles = Vec::new();

        for _ in 0..64 {
            let offsets = Arc::clone(&second_attempt_offsets_ms);
            let strategy = Arc::clone(&strategy);
            handles.push(tokio::spawn(async move {
                let local_attempt = Arc::new(AtomicUsize::new(0));
                let result = strategy
                    .execute({
                        let local_attempt = Arc::clone(&local_attempt);
                        let offsets = Arc::clone(&offsets);
                        move || {
                            let local_attempt = Arc::clone(&local_attempt);
                            let offsets = Arc::clone(&offsets);
                            async move {
                                let attempt_idx = local_attempt.fetch_add(1, Ordering::SeqCst);
                                if attempt_idx == 1 {
                                    let mut guard = offsets.lock().await;
                                    guard.push(start.elapsed().as_millis());
                                }

                                if attempt_idx == 0 {
                                    Err::<(), _>("retry-once")
                                } else {
                                    Ok::<(), _>(())
                                }
                            }
                        }
                    })
                    .await;

                assert!(result.is_ok());
            }));
        }

        for handle in handles {
            handle.await.expect("task join should succeed");
        }

        let guard = second_attempt_offsets_ms.lock().await;
        guard.clone()
    }

    let no_jitter_offsets = collect_second_attempt_offsets(JitterPolicy::None).await;
    let full_jitter_offsets = collect_second_attempt_offsets(JitterPolicy::Full).await;

    assert_eq!(no_jitter_offsets.len(), 64);
    assert_eq!(full_jitter_offsets.len(), 64);

    let min_no_jitter = *no_jitter_offsets.iter().min().expect("non-empty offsets");
    let min_full_jitter = *full_jitter_offsets.iter().min().expect("non-empty offsets");

    assert!(
        min_no_jitter >= 18,
        "without jitter second-attempt wave should not start before fixed base delay"
    );
    assert!(
        min_full_jitter < min_no_jitter,
        "full jitter should advance at least some retries earlier than synchronized fixed-delay wave"
    );
}
