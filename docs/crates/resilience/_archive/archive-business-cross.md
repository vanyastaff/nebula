# Archived From "docs/archive/business-cross.md"

### nebula-resilience
**Назначение:** Паттерны устойчивости для надежной работы.

**Паттерны:**
- Circuit Breaker
- Retry with backoff
- Bulkhead isolation
- Rate limiting
- Timeout

```rust
// Circuit Breaker
let breaker = CircuitBreaker::new()
    .failure_threshold(5)
    .reset_timeout(Duration::from_secs(60));

let result = breaker.call(async {
    external_api.call().await
}).await?;

// Retry policy
let policy = RetryPolicy::exponential()
    .initial_delay(Duration::from_millis(100))
    .max_attempts(3)
    .max_delay(Duration::from_secs(10));

let result = with_retry!(policy, async {
    unreliable_operation().await
});

// Bulkhead для изоляции
let bulkhead = Bulkhead::new()
    .max_concurrent(10)
    .queue_size(50);

let result = bulkhead.execute(async {
    heavy_operation().await
}).await?;

// Композиция паттернов
let executor = ResilientExecutor::new()
    .with_circuit_breaker(breaker)
    .with_retry(policy)
    .with_bulkhead(bulkhead)
    .with_timeout(Duration::from_secs(30));

let result = executor.execute(async {
    complex_operation().await
}).await?;
```

---

