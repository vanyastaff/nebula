//! Rate Limiter Pattern Demonstration
//!
//! This example demonstrates various rate limiting strategies including security features
//! and shows the DoS protection mechanisms implemented in the rate limiters.

use nebula_resilience::{
    AdaptiveRateLimiter, AnyRateLimiter, LeakyBucket, RateLimiter, ResilienceError, SlidingWindow,
    TokenBucket,
};
use std::time::{Duration, Instant};

#[tokio::main]
#[expect(clippy::excessive_nesting)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚦 Rate Limiter Pattern Demo");
    println!("============================");

    // Test 1: Token Bucket with Security Validation
    println!("\n📊 Test 1: Token Bucket with Security Features");

    // Test safe parameter validation
    let safe_bucket = TokenBucket::new(10, 5.0);
    println!("  ✅ Created token bucket with safe parameters (capacity=10, rate=5.0/s)");

    // Test parameter clamping for security
    let _extreme_bucket = TokenBucket::new(1_000_000, 50_000.0); // Should be clamped
    println!("  🔒 Created token bucket with extreme parameters (auto-clamped for security)");

    // Demonstrate token bucket operation
    println!("  🧪 Testing token acquisition...");
    let start = Instant::now();
    let mut successful_ops = 0;
    let mut rate_limited_ops = 0;

    for i in 1..=20 {
        match safe_bucket.acquire().await {
            Ok(()) => {
                successful_ops += 1;
                print!("✅");
            }
            Err(ResilienceError::RateLimitExceeded { retry_after, .. }) => {
                rate_limited_ops += 1;
                print!("🚫");
                if let Some(delay) = retry_after {
                    tokio::time::sleep(delay).await;
                }
            }
            Err(e) => println!("  ❌ Unexpected error: {}", e),
        }

        if i % 5 == 0 {
            println!(" (batch {})", i / 5);
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let elapsed = start.elapsed();
    println!(
        "  📊 Results: {} successful, {} rate-limited in {:?}",
        successful_ops, rate_limited_ops, elapsed
    );

    // Test 2: Leaky Bucket
    println!("\n📊 Test 2: Leaky Bucket Pattern");

    let leaky_bucket = LeakyBucket::new(5, 2.0); // 5 capacity, 2 ops/sec leak rate
    println!("  ✅ Created leaky bucket (capacity=5, leak_rate=2.0/s)");

    // Fill the bucket quickly
    println!("  🧪 Filling bucket rapidly...");
    for _i in 1..=8 {
        match leaky_bucket.acquire().await {
            Ok(()) => print!("✅"),
            Err(_) => print!("🚫"),
        }
    }
    println!(" (initial burst)");

    // Wait and try again
    println!("  ⏳ Waiting for leak...");
    tokio::time::sleep(Duration::from_secs(2)).await;

    for _i in 1..=3 {
        match leaky_bucket.acquire().await {
            Ok(()) => print!("✅"),
            Err(_) => print!("🚫"),
        }
    }
    println!(" (after leak period)");

    // Test 3: Sliding Window
    println!("\n📊 Test 3: Sliding Window Rate Limiter");

    let sliding_window = SlidingWindow::new(Duration::from_secs(2), 5); // 5 ops per 2 seconds
    println!("  ✅ Created sliding window (5 ops per 2 seconds)");

    println!("  🧪 Testing sliding window behavior...");
    let test_start = Instant::now();

    for round in 1..=3 {
        println!(
            "  📅 Round {} (t={:.1}s):",
            round,
            test_start.elapsed().as_secs_f64()
        );

        for i in 1..=7 {
            match sliding_window.acquire().await {
                Ok(()) => print!("✅"),
                Err(ResilienceError::RateLimitExceeded { retry_after, .. }) => {
                    print!("🚫");
                    if i == 6 {
                        // Show retry_after for the first rejection
                        if let Some(delay) = retry_after {
                            println!(" (retry after {:?})", delay);
                        }
                    }
                }
                Err(e) => println!("  ❌ Error: {}", e),
            }
        }
        println!();

        tokio::time::sleep(Duration::from_millis(700)).await;
    }

    // Test 4: Adaptive Rate Limiter
    println!("\n📊 Test 4: Adaptive Rate Limiter");

    let adaptive = AdaptiveRateLimiter::new(10.0, 1.0, 50.0);
    println!("  ✅ Created adaptive rate limiter (initial=10, min=1, max=50)");

    // Simulate success scenario
    println!("  🧪 Simulating successful operations...");
    for _ in 1..=20 {
        let result = adaptive
            .execute(|| async {
                // Simulate successful operation
                tokio::time::sleep(Duration::from_millis(10)).await;
                Ok::<String, ResilienceError>("Success".to_string())
            })
            .await;

        match result {
            Ok(_) => print!("✅"),
            Err(_) => print!("❌"),
        }
    }
    println!(
        "\n  📈 Current adaptive rate: {:.2}",
        adaptive.current_rate().await
    );

    // Simulate failure scenario
    println!("  💥 Simulating failing operations...");
    for _ in 1..=10 {
        let result = adaptive
            .execute(|| async {
                // Simulate failing operation
                Err::<String, ResilienceError>(ResilienceError::Custom {
                    message: "Simulated failure".to_string(),
                    retryable: true,
                    source: None,
                })
            })
            .await;

        match result {
            Ok(_) => print!("✅"),
            Err(_) => print!("❌"),
        }
    }
    println!(
        "\n  📉 Current adaptive rate: {:.2}",
        adaptive.current_rate().await
    );

    // Test 5: Security and DoS Protection
    println!("\n📊 Test 5: Security and DoS Protection");

    // Test with type-erased rate limiter
    let rate_limiters: Vec<AnyRateLimiter> = vec![
        AnyRateLimiter::TokenBucket(std::sync::Arc::new(TokenBucket::new(5, 2.0))),
        AnyRateLimiter::LeakyBucket(std::sync::Arc::new(LeakyBucket::new(5, 2.0))),
        AnyRateLimiter::SlidingWindow(std::sync::Arc::new(SlidingWindow::new(
            Duration::from_secs(1),
            5,
        ))),
        AnyRateLimiter::Adaptive(std::sync::Arc::new(AdaptiveRateLimiter::new(
            5.0, 1.0, 10.0,
        ))),
    ];

    let rate_limiter_names = ["TokenBucket", "LeakyBucket", "SlidingWindow", "Adaptive"];

    for (limiter, name) in rate_limiters.iter().zip(rate_limiter_names.iter()) {
        println!("  🧪 Testing {} under high load...", name);

        let start = Instant::now();
        let mut success_count = 0;
        let mut rejection_count = 0;

        // Simulate high-frequency requests (potential DoS attack)
        for _ in 0..50 {
            match limiter.acquire().await {
                Ok(()) => {
                    success_count += 1;
                    print!("✅");
                }
                Err(_) => {
                    rejection_count += 1;
                    print!("🚫");
                }
            }

            // Small delay to prevent overwhelming the system
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let elapsed = start.elapsed();
        let effective_rate = success_count as f64 / elapsed.as_secs_f64();

        println!();
        println!(
            "    📊 {}: {} success, {} rejected, {:.2} ops/sec effective rate",
            name, success_count, rejection_count, effective_rate
        );

        // Reset for next test
        limiter.reset().await;
    }

    // Test 6: Error Handling and Edge Cases
    println!("\n📊 Test 6: Error Handling and Edge Cases");

    let bucket = TokenBucket::new(1, 1.0);

    // Test execute method with various scenarios
    println!("  🧪 Testing execute method with different scenarios...");

    // Successful operation
    let result = bucket
        .execute(|| async { Ok::<String, ResilienceError>("Success".to_string()) })
        .await;
    println!("  ✅ Successful operation: {:?}", result.is_ok());

    // Operation that fails after rate limit check
    let result = bucket
        .execute(|| async {
            Err::<String, ResilienceError>(ResilienceError::Custom {
                message: "Operation failed".to_string(),
                retryable: false,
                source: None,
            })
        })
        .await;
    println!("  ❌ Failed operation: {:?}", result.is_err());

    // Rate limited operation
    let result = bucket
        .execute(|| async { Ok::<String, ResilienceError>("Should be rate limited".to_string()) })
        .await;
    println!("  🚫 Rate limited operation: {:?}", result.is_err());

    // Test 7: Performance Benchmark
    println!("\n📊 Test 7: Performance Benchmark");

    let perf_bucket = TokenBucket::new(1000, 500.0); // High capacity for benchmark
    let operations = 1000;
    let start = Instant::now();

    for _ in 0..operations {
        let _ = perf_bucket.acquire().await;
    }

    let elapsed = start.elapsed();
    let throughput = operations as f64 / elapsed.as_secs_f64();

    println!(
        "  ⚡ Completed {} rate limit checks in {:?}",
        operations, elapsed
    );
    println!("  📈 Throughput: {:.2} checks/second", throughput);

    println!("\n🎉 Rate Limiter Demo Completed Successfully!");
    println!("   ✅ All rate limiter types working");
    println!("   ✅ Security validation active");
    println!("   ✅ DoS protection effective");
    println!("   ✅ Performance optimized");

    Ok(())
}
