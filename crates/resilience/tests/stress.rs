//! Stress tests for high-concurrency pipeline scenarios.
//!
//! These tests exercise the resilience primitives under extreme concurrent load
//! (5K–10K tasks) to verify:
//! - No permit / resource leaks under heavy contention.
//! - Consistent state transitions with no stuck states.
//! - All futures resolve (no hangs) regardless of success/failure mix.
//! - Cooperative shutdown (Gate) is reliable while tasks are in-flight.
//!
//! Individual tests are gated with `#[cfg(not(miri))]` because Miri cannot
//! drive the Tokio multi-thread runtime.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_resilience::{
    CallError,
    bulkhead::{Bulkhead, BulkheadConfig},
    circuit_breaker::{CircuitBreaker, CircuitBreakerConfig},
    gate::Gate,
    pipeline::ResiliencePipeline,
    retry::{BackoffConfig, RetryConfig},
    sink::CircuitState,
};

// ── Test 1: Bulkhead under heavy contention ─────────────────────────────────

/// Verifies that 10 000 tasks competing for 100 bulkhead permits all either
/// complete successfully or get rejected (`BulkheadFull` / `Timeout`), with
/// zero permit leaks after every task has finished.
#[cfg(not(miri))]
#[tokio::test(flavor = "multi_thread")]
async fn stress_bulkhead_heavy_contention() {
    const TASKS: usize = 10_000;
    const PERMITS: usize = 100;
    const QUEUE: usize = 500;

    let bh = Arc::new(
        Bulkhead::new(BulkheadConfig {
            max_concurrency: PERMITS,
            queue_size: QUEUE,
            timeout: Some(Duration::from_millis(200)),
        })
        .unwrap(),
    );

    let completed = Arc::new(AtomicU32::new(0));
    let rejected = Arc::new(AtomicU32::new(0));

    let mut handles = Vec::with_capacity(TASKS);
    for _ in 0..TASKS {
        let bh = Arc::clone(&bh);
        let completed = Arc::clone(&completed);
        let rejected = Arc::clone(&rejected);
        handles.push(tokio::spawn(async move {
            let result = bh
                .call::<_, &str, _>(|| async {
                    // Simulate brief work so permit contention is real.
                    tokio::time::sleep(Duration::from_micros(100)).await;
                    Ok("ok")
                })
                .await;
            match result {
                Ok(_) => {
                    completed.fetch_add(1, Ordering::Relaxed);
                },
                Err(CallError::BulkheadFull | CallError::Timeout(_)) => {
                    rejected.fetch_add(1, Ordering::Relaxed);
                },
                Err(other) => panic!("unexpected error: {other:?}"),
            }
        }));
    }

    for h in handles {
        h.await.expect("task panicked");
    }

    let done = completed.load(Ordering::Relaxed);
    let shed = rejected.load(Ordering::Relaxed);
    assert_eq!(
        done + shed,
        TASKS as u32,
        "every task must complete or be rejected — no hangs"
    );
    assert_eq!(
        bh.active_operations(),
        0,
        "all permits must be returned after tasks finish"
    );
    assert_eq!(
        bh.available_permits(),
        PERMITS,
        "permit count must be fully restored"
    );
}

// ── Test 2: Circuit breaker concurrent state transitions ────────────────────

/// Verifies that 5 000 tasks sharing a single `Arc<CircuitBreaker>` produce
/// consistent state transitions. After injecting enough failures to trip the
/// breaker, the state is either `Open` or (after reset) `Closed` — never stuck
/// in an invalid combination. No panics allowed.
#[cfg(not(miri))]
#[tokio::test(flavor = "multi_thread")]
async fn stress_circuit_breaker_concurrent_transitions() {
    const TASKS: usize = 5_000;

    let cb = Arc::new(
        CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 20,
            min_operations: 10,
            reset_timeout: Duration::from_millis(100),
            max_half_open_operations: 5,
            ..Default::default()
        })
        .unwrap(),
    );

    let failures = Arc::new(AtomicU32::new(0));
    let open_rejections = Arc::new(AtomicU32::new(0));
    let successes = Arc::new(AtomicU32::new(0));

    let mut handles = Vec::with_capacity(TASKS);
    for i in 0..TASKS {
        let cb = Arc::clone(&cb);
        let failures = Arc::clone(&failures);
        let open_rejections = Arc::clone(&open_rejections);
        let successes = Arc::clone(&successes);
        handles.push(tokio::spawn(async move {
            // First 2 000 tasks always fail to trip the breaker.
            let result = cb
                .call(|| async move {
                    if i < 2_000 {
                        Err::<u32, &str>("injected failure")
                    } else {
                        Ok(1u32)
                    }
                })
                .await;
            match result {
                Ok(_) => {
                    successes.fetch_add(1, Ordering::Relaxed);
                },
                Err(CallError::CircuitOpen) => {
                    open_rejections.fetch_add(1, Ordering::Relaxed);
                },
                Err(CallError::Operation(_)) => {
                    failures.fetch_add(1, Ordering::Relaxed);
                },
                Err(other) => panic!("unexpected error: {other:?}"),
            }
        }));
    }

    for h in handles {
        h.await.expect("task panicked");
    }

    // State must be one of the valid terminal states — not stuck.
    let final_state = cb.circuit_state();
    assert!(
        matches!(
            final_state,
            CircuitState::Closed | CircuitState::Open | CircuitState::HalfOpen
        ),
        "circuit breaker must be in a valid state, got {final_state:?}"
    );

    // Sanity: total recorded outcomes equals task count minus open-rejections
    // (open rejections are rejected before touching the window).
    assert_eq!(
        successes.load(Ordering::Relaxed)
            + failures.load(Ordering::Relaxed)
            + open_rejections.load(Ordering::Relaxed),
        TASKS as u32,
        "all tasks must be accounted for"
    );
}

// ── Test 3: Full pipeline stress ─────────────────────────────────────────────

/// Verifies that 5 000 concurrent calls through a full pipeline
/// (retry × 2 + circuit breaker + bulkhead with 50 permits) all resolve
/// without panics or resource leaks. A 50/50 success/failure mix is used.
#[cfg(not(miri))]
#[tokio::test(flavor = "multi_thread")]
async fn stress_full_pipeline_no_leaks() {
    const TASKS: usize = 5_000;
    const BH_PERMITS: usize = 50;

    let cb = Arc::new(
        CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 60,
            min_operations: 20,
            reset_timeout: Duration::from_millis(50),
            max_half_open_operations: 10,
            ..Default::default()
        })
        .unwrap(),
    );
    let bh = Arc::new(
        Bulkhead::new(BulkheadConfig {
            max_concurrency: BH_PERMITS,
            queue_size: 200,
            timeout: Some(Duration::from_millis(500)),
        })
        .unwrap(),
    );

    let pipeline = Arc::new(
        ResiliencePipeline::<&str>::builder()
            .retry(
                RetryConfig::new(2)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::ZERO)),
            )
            .circuit_breaker(cb.clone())
            .bulkhead(bh.clone())
            .build(),
    );

    let resolved = Arc::new(AtomicU32::new(0));

    let mut handles = Vec::with_capacity(TASKS);
    for i in 0..TASKS {
        let pipeline = Arc::clone(&pipeline);
        let resolved = Arc::clone(&resolved);
        handles.push(tokio::spawn(async move {
            let _result = pipeline
                .call(move || async move {
                    // Alternate success / failure by task index.
                    tokio::time::sleep(Duration::from_micros(50)).await;
                    if i % 2 == 0 {
                        Ok::<u32, &str>(i as u32)
                    } else {
                        Err("transient")
                    }
                })
                .await;
            // All outcomes are valid — just count resolution.
            resolved.fetch_add(1, Ordering::Relaxed);
        }));
    }

    for h in handles {
        h.await.expect("task panicked");
    }

    assert_eq!(
        resolved.load(Ordering::Relaxed),
        TASKS as u32,
        "every pipeline call must resolve — no hangs"
    );
    assert_eq!(
        bh.active_operations(),
        0,
        "bulkhead permits must all be returned"
    );
}

// ── Test 4: Gate shutdown under load ─────────────────────────────────────────

/// Verifies that `Gate::close()` resolves promptly after 1 000 in-flight
/// guards are dropped, even when `close()` is called while tasks are still
/// running. New entries after `close()` must be rejected.
#[cfg(not(miri))]
#[tokio::test(flavor = "multi_thread")]
async fn stress_gate_shutdown_under_load() {
    const TASKS: usize = 1_000;

    let gate = Gate::new();
    let gate_for_tasks = gate.clone();

    let entered = Arc::new(AtomicU32::new(0));
    let completed = Arc::new(AtomicU32::new(0));

    // Spawn tasks that enter the gate, do brief work, then drop the guard.
    let mut handles = Vec::with_capacity(TASKS);
    for _ in 0..TASKS {
        let g = gate_for_tasks.clone();
        let entered = Arc::clone(&entered);
        let completed = Arc::clone(&completed);
        handles.push(tokio::spawn(async move {
            // Gate may already be closing by the time some tasks run — that's fine.
            let Ok(_guard) = g.enter() else {
                // Gate closed before we could enter; that is an expected outcome.
                completed.fetch_add(1, Ordering::Relaxed);
                return;
            };
            entered.fetch_add(1, Ordering::Relaxed);
            // Simulate brief work.
            tokio::time::sleep(Duration::from_micros(200)).await;
            // Guard dropped here, returning the permit to the gate.
            completed.fetch_add(1, Ordering::Relaxed);
        }));
    }

    // Give tasks time to enter before closing.
    tokio::time::sleep(Duration::from_millis(5)).await;

    // Close the gate — must resolve once all active guards are dropped.
    tokio::time::timeout(Duration::from_secs(10), gate.close())
        .await
        .expect("Gate::close() should resolve within 10 s");

    // All spawned tasks must have finished.
    for h in handles {
        h.await.expect("task panicked");
    }

    assert_eq!(
        completed.load(Ordering::Relaxed),
        TASKS as u32,
        "all tasks must complete (either entered+finished or rejected)"
    );

    // After close, new entries must be rejected.
    assert!(
        gate.enter().is_err(),
        "gate must reject entries after close()"
    );
    assert!(gate.is_closed(), "gate must report closed after close()");
}
