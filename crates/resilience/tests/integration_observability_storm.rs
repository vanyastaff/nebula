//! Integration tests validating observability behavior under failure storms.

use nebula_resilience::observability::{MetricsHook, ObservabilityHook, ObservabilityHooks, PatternEvent};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Default)]
struct CountingHook {
    failed_events: AtomicU64,
    retry_events: AtomicU64,
    timeout_events: AtomicU64,
    breaker_state_events: AtomicU64,
}

#[derive(Default)]
struct SlowHook {
    observed_events: AtomicU64,
    delay: Duration,
}

impl SlowHook {
    fn new(delay: Duration) -> Self {
        Self {
            observed_events: AtomicU64::new(0),
            delay,
        }
    }
}

impl ObservabilityHook for SlowHook {
    fn on_event(&self, _event: &PatternEvent) {
        std::thread::sleep(self.delay);
        self.observed_events.fetch_add(1, Ordering::Relaxed);
    }
}

impl ObservabilityHook for CountingHook {
    fn on_event(&self, event: &PatternEvent) {
        match event {
            PatternEvent::Failed { .. } => {
                self.failed_events.fetch_add(1, Ordering::Relaxed);
            }
            PatternEvent::RetryAttempt { .. } => {
                self.retry_events.fetch_add(1, Ordering::Relaxed);
            }
            PatternEvent::TimeoutOccurred { .. } => {
                self.timeout_events.fetch_add(1, Ordering::Relaxed);
            }
            PatternEvent::CircuitBreakerStateChanged { .. } => {
                self.breaker_state_events.fetch_add(1, Ordering::Relaxed);
            }
            PatternEvent::Started { .. }
            | PatternEvent::Succeeded { .. }
            | PatternEvent::RateLimitExceeded { .. }
            | PatternEvent::BulkheadCapacityReached { .. } => {}
        }
    }
}

#[tokio::test]
async fn test_observability_failure_storm_metrics_and_event_delivery() {
    let metrics_hook = Arc::new(MetricsHook::new());
    let counting_hook = Arc::new(CountingHook::default());

    let hooks = ObservabilityHooks::new()
        .with_hook(metrics_hook.clone())
        .with_hook(counting_hook.clone());

    const WORKERS: usize = 16;
    const EVENTS_PER_WORKER: usize = 200;
    let expected = (WORKERS * EVENTS_PER_WORKER) as u64;

    let mut tasks = Vec::with_capacity(WORKERS);

    for _ in 0..WORKERS {
        let hooks = hooks.clone();
        tasks.push(tokio::spawn(async move {
            for _ in 0..EVENTS_PER_WORKER {
                hooks.emit(PatternEvent::Failed {
                    pattern: "retry".to_string(),
                    operation: "storm-op".to_string(),
                    error: "downstream unavailable".to_string(),
                    duration: Duration::from_millis(3),
                });

                hooks.emit(PatternEvent::RetryAttempt {
                    operation: "storm-op".to_string(),
                    attempt: 2,
                    max_attempts: 3,
                });

                hooks.emit(PatternEvent::TimeoutOccurred {
                    operation: "storm-op".to_string(),
                    timeout: Duration::from_millis(10),
                });

                hooks.emit(PatternEvent::CircuitBreakerStateChanged {
                    service: "storm-svc".to_string(),
                    from_state: "closed".to_string(),
                    to_state: "open".to_string(),
                });
            }
        }));
    }

    for task in tasks {
        task.await.unwrap();
    }

    assert_eq!(
        counting_hook.failed_events.load(Ordering::Relaxed),
        expected
    );
    assert_eq!(
        counting_hook.retry_events.load(Ordering::Relaxed),
        expected
    );
    assert_eq!(
        counting_hook.timeout_events.load(Ordering::Relaxed),
        expected
    );
    assert_eq!(
        counting_hook.breaker_state_events.load(Ordering::Relaxed),
        expected
    );

    let metrics = metrics_hook.metrics();

    let retry_failure = metrics.get("retry.failure").unwrap();
    assert_eq!(retry_failure.sum, expected as f64);

    let retry_attempts = metrics.get("retry.attempts").unwrap();
    assert_eq!(retry_attempts.sum, expected as f64);

    let timeout = metrics.get("timeout.storm-op").unwrap();
    assert_eq!(timeout.sum, expected as f64);

    let breaker_open = metrics.get("circuit_breaker.storm-svc.state.open").unwrap();
    assert_eq!(breaker_open.sum, expected as f64);
}

#[tokio::test]
async fn test_observability_backpressure_no_drop_with_slow_hook() {
    let slow_hook = Arc::new(SlowHook::new(Duration::from_millis(1)));
    let fast_hook = Arc::new(CountingHook::default());

    let hooks = Arc::new(
        ObservabilityHooks::new()
            .with_hook(slow_hook.clone())
            .with_hook(fast_hook.clone()),
    );

    const WORKERS: usize = 8;
    const EVENTS_PER_WORKER: usize = 40;
    let expected = (WORKERS * EVENTS_PER_WORKER) as u64;

    let mut tasks = Vec::with_capacity(WORKERS);
    for _ in 0..WORKERS {
        let hooks = Arc::clone(&hooks);
        tasks.push(tokio::spawn(async move {
            for _ in 0..EVENTS_PER_WORKER {
                hooks.emit(PatternEvent::TimeoutOccurred {
                    operation: "slow-path".to_string(),
                    timeout: Duration::from_millis(5),
                });
            }
        }));
    }

    for task in tasks {
        task.await.unwrap();
    }

    assert_eq!(slow_hook.observed_events.load(Ordering::Relaxed), expected);
    assert_eq!(fast_hook.timeout_events.load(Ordering::Relaxed), expected);
}
