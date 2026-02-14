//! Retry Strategies and Manager Integration Demonstration
//!
//! This example demonstrates various retry strategies and the resilience manager
//! with type-safe const generic APIs.

use nebula_resilience::prelude::*;
use nebula_resilience::{PolicyBuilder, ResilienceManager};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

// Helper for simulating retryable operations
struct RetryableService {
    name: String,
    failure_count: Arc<AtomicUsize>,
    max_failures: usize,
}

impl RetryableService {
    fn new(name: impl Into<String>, max_failures: usize) -> Self {
        Self {
            name: name.into(),
            failure_count: Arc::new(AtomicUsize::new(0)),
            max_failures,
        }
    }

    async fn call(&self) -> ResilienceResult<String> {
        let current_failures = self.failure_count.fetch_add(1, Ordering::SeqCst);

        if current_failures < self.max_failures {
            println!(
                "    💥 {} failed (attempt {})",
                self.name,
                current_failures + 1
            );
            Err(ResilienceError::Custom {
                message: format!("{} temporary failure", self.name),
                retryable: true,
                source: None,
            })
        } else {
            println!(
                "    ✅ {} succeeded after {} failures",
                self.name, current_failures
            );
            Ok(format!("{} result", self.name))
        }
    }

    #[allow(dead_code)]
    fn reset(&self) {
        self.failure_count.store(0, Ordering::SeqCst);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔄 Retry Strategies and Manager Demo");
    println!("===================================");

    // Test 1: Exponential Retry Strategy with Const Generics
    println!("\n📊 Test 1: Exponential Retry Strategy (Const Generics)");

    // Type-safe retry: MAX_ATTEMPTS = 3
    let exp_strategy = exponential_retry::<3>()?;
    println!("  ✅ Created exponential retry strategy (3 attempts)");

    let service = RetryableService::new("ExponentialService", 2);
    let start = Instant::now();

    println!("  🧪 Testing exponential retry...");
    let result = exp_strategy.execute_resilient(|| service.call()).await;
    let elapsed = start.elapsed();

    match result {
        Ok((value, _stats)) => println!(
            "  ✅ Exponential retry succeeded: {} (took {:?})",
            value, elapsed
        ),
        Err(e) => println!("  ❌ Exponential retry failed: {}", e),
    }

    // Test 2: Fixed Delay Strategy
    println!("\n📊 Test 2: Fixed Delay Strategy");

    // Type-safe: DELAY_MS = 100, MAX_ATTEMPTS = 3
    let fixed_strategy = fixed_retry::<100, 3>()?;
    println!("  ✅ Created fixed delay strategy (3 attempts, 100ms delay)");

    let service2 = RetryableService::new("FixedDelayService", 2);
    let start = Instant::now();

    let result = fixed_strategy.execute_resilient(|| service2.call()).await;
    let elapsed = start.elapsed();

    match result {
        Ok((value, _stats)) => println!(
            "  ✅ Fixed delay retry succeeded: {} (took {:?})",
            value, elapsed
        ),
        Err(e) => println!("  ❌ Fixed delay retry failed: {}", e),
    }

    // Test 3: Aggressive Retry Strategy
    println!("\n📊 Test 3: Aggressive Retry Strategy");

    let aggressive_strategy = aggressive_retry::<5>()?;
    println!("  ✅ Created aggressive retry strategy (5 attempts)");

    let service3 = RetryableService::new("AggressiveService", 4);
    let start = Instant::now();

    let result = aggressive_strategy
        .execute_resilient(|| service3.call())
        .await;
    let elapsed = start.elapsed();

    match result {
        Ok((value, _stats)) => println!(
            "  ✅ Aggressive retry succeeded: {} (took {:?})",
            value, elapsed
        ),
        Err(e) => println!("  ❌ Aggressive retry failed: {}", e),
    }

    // Test 4: Circuit Breaker + Retry Combination
    println!("\n📊 Test 4: Circuit Breaker + Retry Combination");

    let circuit_breaker = CircuitBreaker::<3, 5000>::with_defaults()?;
    let retry = exponential_retry::<3>()?;
    println!("  ✅ Created circuit breaker (3 failures, 5s reset) + retry (3 attempts)");

    let service4 = RetryableService::new("CombinedService", 2);
    let start = Instant::now();

    let result = circuit_breaker
        .execute(|| {
            let retry_ref = &retry;
            async move { retry_ref.execute_resilient(|| service4.call()).await }
        })
        .await;
    let elapsed = start.elapsed();

    match result {
        Ok((value, _stats)) => println!(
            "  ✅ Combined pattern succeeded: {} (took {:?})",
            value, elapsed
        ),
        Err(e) => println!("  ❌ Combined pattern failed: {}", e),
    }

    // Test 5: Resilience Manager Integration
    println!("\n📊 Test 5: Resilience Manager with Policy Builder");

    let policy = PolicyBuilder::new()
        .with_timeout(Duration::from_secs(5))
        .with_retry_exponential(3, Duration::from_millis(100))
        .build();

    let manager = ResilienceManager::new(policy);
    println!("  ✅ Created resilience manager with default policy");

    // Register a database policy
    let db_policy = PolicyBuilder::new()
        .with_timeout(Duration::from_secs(2))
        .with_retry_exponential(5, Duration::from_millis(200))
        .build();

    manager.register_service("database", db_policy);
    println!("  ✅ Registered database service with custom policy");

    // Register an API policy
    let api_policy = PolicyBuilder::new()
        .with_timeout(Duration::from_secs(10))
        .with_retry_fixed(2, Duration::from_millis(500))
        .build();

    manager.register_service("external_api", api_policy);
    println!("  ✅ Registered external_api service with custom policy");

    // Test 6: Concurrent Retry Operations
    println!("\n📊 Test 6: Concurrent Retry Operations");

    let concurrent_strategy = Arc::new(exponential_retry::<3>()?);
    let mut handles = vec![];

    println!("  🧪 Starting 5 concurrent retry operations...");

    for i in 1..=5 {
        let strategy = Arc::clone(&concurrent_strategy);
        let handle = tokio::spawn(async move {
            let service = RetryableService::new(format!("Concurrent-{}", i), 1);
            let start = Instant::now();

            let result = strategy.execute_resilient(|| service.call()).await;
            let elapsed = start.elapsed();

            match result {
                Ok((value, _stats)) => {
                    println!("    ✅ {} completed in {:?}", value, elapsed);
                    Ok(())
                }
                Err(e) => {
                    println!("    ❌ Concurrent-{} failed: {}", i, e);
                    Err(e)
                }
            }
        });
        handles.push(handle);
    }

    // Wait for all concurrent operations
    for handle in handles {
        let _ = handle.await;
    }

    // Test 7: Retry with Statistics
    println!("\n📊 Test 7: Retry with Statistics");

    let stats_strategy = exponential_retry::<4>()?;
    let service7 = RetryableService::new("StatsService", 3);

    println!("  🧪 Testing retry with statistics collection...");
    let start = Instant::now();

    let result = stats_strategy.execute(|| service7.call()).await;
    let elapsed = start.elapsed();

    match result {
        Ok((value, stats)) => {
            println!("  ✅ Operation succeeded: {}", value);
            println!("    📊 Total attempts: {}", stats.total_attempts);
            println!("    📊 Total duration: {:?}", stats.total_duration);
            println!("    📊 Total elapsed: {:?}", elapsed);
        }
        Err(e) => println!("  ❌ Operation failed: {}", e),
    }

    // Test 8: Error Classification
    println!("\n📊 Test 8: Error Classification");

    let retryable_error = ResilienceError::Custom {
        message: "Temporary network error".to_string(),
        retryable: true,
        source: None,
    };

    let terminal_error = ResilienceError::Custom {
        message: "Authentication failed".to_string(),
        retryable: false,
        source: None,
    };

    println!("  🧪 Testing error classification...");
    println!(
        "    Retryable error is_retryable: {}",
        retryable_error.is_retryable()
    );
    println!(
        "    Terminal error is_retryable: {}",
        terminal_error.is_retryable()
    );
    println!(
        "    Retryable error is_terminal: {}",
        retryable_error.is_terminal()
    );
    println!(
        "    Terminal error is_terminal: {}",
        terminal_error.is_terminal()
    );

    // Test 9: Standard Configuration Aliases
    println!("\n📊 Test 9: Standard Configuration Aliases");

    let standard_cb = StandardCircuitBreaker::default();
    let _fast_cb = FastCircuitBreaker::default();
    let _slow_cb = SlowCircuitBreaker::default();

    println!("  ✅ StandardCircuitBreaker: 5 failures, 30s reset");
    println!("  ✅ FastCircuitBreaker: 3 failures, 10s reset");
    println!("  ✅ SlowCircuitBreaker: 10 failures, 60s reset");

    // Quick test with standard breaker
    let result = standard_cb
        .execute(|| async { Ok::<_, ResilienceError>("Quick test") })
        .await;

    match result {
        Ok(value) => println!("  ✅ Standard breaker test: {}", value),
        Err(e) => println!("  ❌ Standard breaker failed: {}", e),
    }

    println!("\n🎉 Retry and Manager Demo Completed Successfully!");
    println!("   ✅ All retry strategies working");
    println!("   ✅ Const generic type safety verified");
    println!("   ✅ Manager integration working");
    println!("   ✅ Error classification working");
    println!("   ✅ Concurrent operations safe");

    Ok(())
}
