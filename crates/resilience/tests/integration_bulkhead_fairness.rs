//! Fairness and starvation stress tests for bulkhead under sustained contention.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nebula_resilience::{Bulkhead, BulkheadConfig, ResilienceError};

#[tokio::test]
async fn test_bulkhead_sustained_contention_all_queued_requests_progress() {
    let bulkhead = Arc::new(Bulkhead::with_config(BulkheadConfig {
        max_concurrency: 4,
        queue_size: 256,
        timeout: Some(Duration::from_secs(2)),
    }));

    let completed = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for _ in 0..96 {
        let bulkhead = Arc::clone(&bulkhead);
        let completed = Arc::clone(&completed);
        handles.push(tokio::spawn(async move {
            let result = bulkhead
                .execute(|| async {
                    tokio::time::sleep(Duration::from_millis(4)).await;
                    Ok::<_, ResilienceError>(())
                })
                .await;

            if result.is_ok() {
                completed.fetch_add(1, Ordering::SeqCst);
            }

            result
        }));
    }

    for handle in handles {
        let result = handle.await.expect("task join should succeed");
        assert!(
            result.is_ok(),
            "all queued tasks should eventually complete"
        );
    }

    assert_eq!(completed.load(Ordering::SeqCst), 96);
}

#[tokio::test]
async fn test_bulkhead_late_arrivals_are_not_starved() {
    let bulkhead = Arc::new(Bulkhead::with_config(BulkheadConfig {
        max_concurrency: 2,
        queue_size: 128,
        timeout: Some(Duration::from_secs(2)),
    }));

    let mut early_handles = Vec::new();
    for _ in 0..12 {
        let bulkhead = Arc::clone(&bulkhead);
        early_handles.push(tokio::spawn(async move {
            bulkhead
                .execute(|| async {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    Ok::<_, ResilienceError>("early")
                })
                .await
        }));
    }

    tokio::time::sleep(Duration::from_millis(10)).await;

    let late_completed = Arc::new(AtomicUsize::new(0));
    let mut late_handles = Vec::new();
    for _ in 0..8 {
        let bulkhead = Arc::clone(&bulkhead);
        let late_completed = Arc::clone(&late_completed);
        late_handles.push(tokio::spawn(async move {
            let started = Instant::now();
            let result = bulkhead
                .execute(|| async {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    Ok::<_, ResilienceError>("late")
                })
                .await;

            if result.is_ok() {
                late_completed.fetch_add(1, Ordering::SeqCst);
            }

            (result, started.elapsed())
        }));
    }

    for handle in early_handles {
        let result = handle.await.expect("early task join should succeed");
        assert!(result.is_ok());
    }

    for handle in late_handles {
        let (result, elapsed) = handle.await.expect("late task join should succeed");
        assert!(result.is_ok(), "late arrivals should not starve under load");
        assert!(
            elapsed < Duration::from_secs(2),
            "late request waited too long under contention: {elapsed:?}"
        );
    }

    assert_eq!(late_completed.load(Ordering::SeqCst), 8);
}

#[tokio::test]
async fn test_bulkhead_queue_backpressure_remains_bounded() {
    let bulkhead = Arc::new(Bulkhead::with_config(BulkheadConfig {
        max_concurrency: 1,
        queue_size: 2,
        timeout: Some(Duration::from_millis(200)),
    }));

    let first_permit = bulkhead
        .acquire()
        .await
        .expect("first permit should succeed");

    let b1 = Arc::clone(&bulkhead);
    let queued_1 = tokio::spawn(async move { b1.acquire().await });
    let b2 = Arc::clone(&bulkhead);
    let queued_2 = tokio::spawn(async move { b2.acquire().await });

    tokio::time::sleep(Duration::from_millis(20)).await;

    let rejected = bulkhead.acquire().await;
    match rejected {
        Err(ResilienceError::BulkheadFull { queued, .. }) => {
            assert_eq!(queued, 2, "queue depth should stay bounded by queue_size");
        }
        _ => panic!("expected BulkheadFull for over-capacity request"),
    }

    drop(first_permit);

    let r1 = queued_1.await.expect("queued task 1 should join");
    let r2 = queued_2.await.expect("queued task 2 should join");

    let ok_count = usize::from(r1.is_ok()) + usize::from(r2.is_ok());
    assert!(ok_count >= 1, "at least one queued request should progress");

    for result in [r1, r2] {
        match result {
            Ok(_) | Err(ResilienceError::Timeout { .. }) => {}
            Err(other) => panic!("unexpected queued outcome: {other}"),
        }
    }
}
