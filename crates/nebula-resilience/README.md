# nebula-resilience

Production-ready resilience patterns for Rust with compile-time configuration validation via const generics and typestate patterns.

## Patterns

| Pattern | Description |
|---------|-------------|
| **Circuit Breaker** | Failure detection with automatic recovery (Closed / Open / HalfOpen states) |
| **Retry** | Configurable strategies with backoff, jitter, and retry conditions |
| **Timeout** | Operation timeouts backed by `tokio::time::timeout` |
| **Bulkhead** | Concurrency isolation via semaphore-based permits |
| **Rate Limiter** | Token bucket, leaky bucket, sliding window, adaptive, and Governor-backed |
| **Fallback** | Graceful degradation with fallback chains |
| **Hedge** | Tail-latency reduction via parallel speculative requests |

## Quick Start

```toml
[dependencies]
nebula-resilience = { path = "crates/nebula-resilience" }
tokio = { version = "1", features = ["full"] }
```

```rust
use nebula_resilience::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile-time validated circuit breaker: 5 failures, 30s reset
    let breaker = CircuitBreaker::<5, 30_000>::new(
        CircuitBreakerConfig::new().with_half_open_limit(3),
    )?;

    // Exponential retry: max 3 attempts
    let retry = exponential_retry::<3>()?;

    let result = breaker.execute(|| async {
        retry.execute_resilient(|| async {
            Ok::<_, ResilienceError>("success")
        }).await
    }).await;

    println!("{result:?}");
    Ok(())
}
```

## Circuit Breaker

Const-generic configuration validated at compile time. Three preset aliases are provided:

```rust
use nebula_resilience::prelude::*;

// Presets
let _fast   = FastCircuitBreaker::default();     // 3 failures, 10s reset
let _std    = StandardCircuitBreaker::default();  // 5 failures, 30s reset
let _slow   = SlowCircuitBreaker::default();     // 10 failures, 60s reset

// Custom
let breaker = CircuitBreaker::<7, 20_000>::new(
    CircuitBreakerConfig::new()
        .with_half_open_limit(2)
        .with_min_operations(10),
)?;

let result = breaker.execute(|| async {
    Ok::<_, ResilienceError>("ok")
}).await;
```

A fast-path atomic state check avoids acquiring the inner lock on the hot path when the breaker is closed.

## Retry

Backoff policies (`ExponentialBackoff`, `LinearBackoff`, `FixedDelay`, `CustomBackoff`) combine with retry conditions (`ConservativeCondition`, `AggressiveCondition`, `TimeBasedCondition`) through const generics:

```rust
use nebula_resilience::prelude::*;

// Convenience constructors
let _standard   = exponential_retry::<3>()?;   // StandardRetry
let _quick      = fixed_retry::<50, 2>()?;     // QuickRetry
let _aggressive = aggressive_retry::<5>()?;    // AggressiveRetry

// Custom configuration
let config = RetryConfig::new(
    ExponentialBackoff::<100, 20, 5000>::default(),
    ConservativeCondition::<3>::new(),
).with_jitter(JitterPolicy::Equal);

let strategy = RetryStrategy::new(config)?;

let (value, stats) = strategy.execute(|| async {
    Ok::<_, ResilienceError>("done")
}).await?;

println!("attempts: {}", stats.attempts);
```

## Bulkhead

Semaphore-based concurrency isolation. Permits are RAII — dropping a permit releases the semaphore slot synchronously.

```rust
use nebula_resilience::prelude::*;

let bulkhead = Bulkhead::new(BulkheadConfig {
    max_concurrent: 10,
    max_wait: Duration::from_secs(5),
});

let permit = bulkhead.acquire().await?;
// ... do work ...
drop(permit); // slot released
```

## Rate Limiter

Five implementations behind the `RateLimiter` trait:

```rust
use nebula_resilience::{TokenBucket, LeakyBucket, SlidingWindow, AdaptiveRateLimiter, RateLimiter};

let tb = TokenBucket::new(100.0, 100);       // 100 req/s, burst 100
let lb = LeakyBucket::new(50.0, 50);         // 50 req/s, capacity 50
let sw = SlidingWindow::new(1000, 60);        // 1000 req per 60s window
let adaptive = AdaptiveRateLimiter::new(100.0, 10.0, 500.0); // min/max bounds
```

## Fallback

```rust
use nebula_resilience::{FallbackStrategy, ValueFallback};

let fallback = ValueFallback::new("default response".to_string());
let value = fallback.fallback(&nebula_resilience::ResilienceError::Timeout {
    duration: std::time::Duration::from_secs(5),
}).await;
```

## Hedge

Speculative execution to cut tail latency:

```rust
use nebula_resilience::{HedgeConfig, HedgeExecutor};
use std::time::Duration;

let hedge = HedgeExecutor::new(HedgeConfig {
    delay: Duration::from_millis(100),
    max_extra_requests: 1,
});

let result = hedge.execute(|| async {
    Ok::<_, nebula_resilience::ResilienceError>("fast path")
}).await;
```

## Policy Composition

Compose patterns into a layered resilience chain:

```rust
use nebula_resilience::{ResilienceChain, ResilienceLayer, LayerBuilder};
use std::time::Duration;

let chain = LayerBuilder::new()
    .timeout(Duration::from_secs(10))
    .retry_exponential(3, Duration::from_millis(100))
    .build();
```

Or use `ResilienceManager` for per-service policy management with built-in metrics:

```rust
use nebula_resilience::prelude::*;

let manager = ResilienceManager::new();
// Register service-level policies, collect metrics, etc.
```

## Type Safety Features

- **Const generics** — configuration like failure thresholds and delays validated at compile time
- **Typestate pattern** — circuit breaker states (Closed, Open, HalfOpen) tracked in the type system
- **Sealed traits** — controlled extensibility for backoff and condition traits
- **Phantom types** — zero-cost state markers with no runtime overhead

## Lint Policy

The crate enforces strict linting:

```rust
#![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::perf)]
#![warn(missing_docs)]
#![deny(unsafe_code)]
```

## Examples

| Example | Description |
|---------|-------------|
| `circuit_breaker_demo` | Circuit breaker states and recovery |
| `retry_manager_demo` | Retry strategies with manager |
| `rate_limiter_demo` | Rate limiter algorithms |
| `bulkhead_timeout_demo` | Bulkhead with timeout composition |
| `pattern_composition` | Composing multiple patterns |
| `ecosystem_integration` | Full Nebula ecosystem integration |
| `dynamic_config_builder` | Runtime configuration with hot reload |
| `observability_demo` | Metrics and observability hooks |
| `simple_manager` | Minimal manager usage |
| `simple_macros_demo` | Helper macro usage |

```bash
cargo run -p nebula-resilience --example circuit_breaker_demo
```

## Benchmarks

```bash
cargo bench -p nebula-resilience
```

Four benchmark suites: `circuit_breaker`, `retry`, `rate_limiter`, `manager`.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
