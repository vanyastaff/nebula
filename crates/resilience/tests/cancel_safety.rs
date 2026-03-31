//! Cancel safety integration tests — verify RAII guards release resources
//! when pipeline futures are dropped mid-flight (e.g., via tokio::select!).

use std::sync::Arc;
use std::time::Duration;

use nebula_resilience::bulkhead::{Bulkhead, BulkheadConfig};
use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, Outcome};
use nebula_resilience::pipeline::ResiliencePipeline;
use nebula_resilience::sink::CircuitState;

// ── CB probe slot released on cancel ────────────────────────────────────────

#[tokio::test]
async fn cb_probe_slot_released_when_pipeline_cancelled() {
    let cb = Arc::new(
        CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            reset_timeout: Duration::from_millis(30),
            max_half_open_operations: 1,
            min_operations: 1,
            ..Default::default()
        })
        .unwrap(),
    );

    // Trip the breaker
    cb.record_outcome(Outcome::Failure);
    assert_eq!(cb.circuit_state(), CircuitState::Open);

    // Wait for HalfOpen
    tokio::time::sleep(Duration::from_millis(40)).await;

    let pipeline = ResiliencePipeline::<&str>::builder()
        .circuit_breaker(cb.clone())
        .build();

    // Start a pipeline call that will be dropped mid-flight
    tokio::select! {
        _ = pipeline.call(|| Box::pin(async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok::<u32, &str>(42)
        })) => unreachable!("operation should not complete"),
        _ = tokio::time::sleep(Duration::from_millis(5)) => {
            // Future dropped — ProbeGuard should release the probe slot
        }
    }

    // Wait for another reset_timeout cycle
    tokio::time::sleep(Duration::from_millis(40)).await;

    // The probe slot must be free — a new probe should succeed
    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(99) }))
        .await;
    assert_eq!(result.unwrap(), 99);
    assert_eq!(cb.circuit_state(), CircuitState::Closed);
}

// ── Bulkhead permit released on cancel ──────────────────────────────────────

#[tokio::test]
async fn bulkhead_permit_released_when_pipeline_cancelled() {
    let bh = Arc::new(
        Bulkhead::new(BulkheadConfig {
            max_concurrency: 1,
            queue_size: 1,
            timeout: None,
        })
        .unwrap(),
    );

    let pipeline = ResiliencePipeline::<&str>::builder()
        .bulkhead(bh.clone())
        .build();

    // Start a long operation then cancel it
    tokio::select! {
        _ = pipeline.call(|| Box::pin(async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok::<u32, &str>(42)
        })) => unreachable!("operation should not complete"),
        _ = tokio::time::sleep(Duration::from_millis(5)) => {
            // Future dropped — permit should be released
        }
    }

    // Give a moment for the drop to propagate
    tokio::task::yield_now().await;

    // The permit must be free — max_concurrency is 1
    assert_eq!(
        bh.available_permits(),
        1,
        "permit should be returned after cancel"
    );

    // A new call should succeed
    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(99) }))
        .await;
    assert_eq!(result.unwrap(), 99);
}

// ── Both CB + Bulkhead released on cancel ───────────────────────────────────

#[tokio::test]
async fn combined_cb_and_bulkhead_released_on_cancel() {
    let cb = Arc::new(
        CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            reset_timeout: Duration::from_millis(30),
            max_half_open_operations: 1,
            min_operations: 1,
            ..Default::default()
        })
        .unwrap(),
    );
    let bh = Arc::new(
        Bulkhead::new(BulkheadConfig {
            max_concurrency: 1,
            queue_size: 1,
            timeout: None,
        })
        .unwrap(),
    );

    // Trip CB, wait for HalfOpen
    cb.record_outcome(Outcome::Failure);
    tokio::time::sleep(Duration::from_millis(40)).await;

    let pipeline = ResiliencePipeline::<&str>::builder()
        .circuit_breaker(cb.clone())
        .bulkhead(bh.clone())
        .build();

    // Cancel mid-flight
    tokio::select! {
        _ = pipeline.call(|| Box::pin(async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok::<u32, &str>(42)
        })) => unreachable!(),
        _ = tokio::time::sleep(Duration::from_millis(5)) => {}
    }

    tokio::task::yield_now().await;

    // Both resources must be released
    assert_eq!(bh.available_permits(), 1, "bulkhead permit leaked");

    // Wait for another reset cycle and verify CB is usable
    tokio::time::sleep(Duration::from_millis(40)).await;
    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(99) }))
        .await;
    assert_eq!(result.unwrap(), 99);
}
