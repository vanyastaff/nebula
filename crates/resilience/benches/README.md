# Nebula Resilience Benchmarks

Comprehensive performance benchmarks for nebula-resilience patterns and optimizations.

## Running Benchmarks

### Run all benchmarks
```bash
cargo bench -p nebula-resilience
```

### Run specific benchmark suite
```bash
cargo bench -p nebula-resilience --bench circuit_breaker
cargo bench -p nebula-resilience --bench rate_limiter
cargo bench -p nebula-resilience --bench manager
cargo bench -p nebula-resilience --bench retry
```

### Run specific test within a suite
```bash
cargo bench -p nebula-resilience --bench manager -- concurrent_access
```

### Generate HTML reports
```bash
cargo bench -p nebula-resilience
# Reports are generated in target/criterion/
```

## Benchmark Suites

### 1. Circuit Breaker (`circuit_breaker.rs`)

Measures circuit breaker performance:
- **closed/execute_success**: Throughput when circuit is closed (happy path)
- **can_execute/closed**: Check overhead when circuit is closed
- **can_execute/open**: Check overhead when circuit is open
- **transitions/closed_to_open**: Time to trigger circuit open
- **transitions/halfopen_to_closed**: Time to recover circuit
- **stats/stats_collection**: Metrics collection overhead

**Key insights:**
- Circuit closed path should be < 1µs
- State checks are lock-free with parking_lot RwLock
- Stats collection overhead

### 2. Rate Limiter (`rate_limiter.rs`)

Compares different rate limiting algorithms:
- **acquire**: TokenBucket, LeakyBucket, SlidingWindow at various rates
- **execute**: Full operation execution with rate limiting
- **current_rate**: Metrics collection performance
- **contention**: Concurrent access from multiple tasks

**Algorithms compared:**
- TokenBucket: Burst-friendly, fast refill
- LeakyBucket: Constant rate, smooth traffic
- SlidingWindow: Precise time-window limiting
- Adaptive: Dynamic rate adjustment
- Governor: GCRA (production-grade)

**Key insights:**
- TokenBucket fastest for burst scenarios
- Governor (GCRA) best for strict rate limiting
- Contention handling varies by algorithm

### 3. Manager (`manager.rs`) ⭐

**CRITICAL: Measures Sprint 2 optimizations!**

This benchmark suite validates our performance improvements:
- **Arc<ResiliencePolicy>** optimization (commit 8ac17ce)
- **DashMap migration** (commit 1087d1e)

**Benchmarks:**
- **policy_lookup**: DashMap lock-free reads vs old RwLock
- **registration**: Service registration/unregistration speed
- **execute_overhead**: Manager overhead with different policies
- **concurrent_access**: Multi-task concurrent execution (DashMap shines!)
- **metrics**: get_metrics, get_all_metrics, list_services performance

**Expected improvements:**
- Policy lookup: 2-5x faster (lock-free reads)
- Concurrent access: 5-10x better scaling
- list_services: Now synchronous (no .await)!

### 4. Retry (`retry.rs`)

Measures retry strategies and jitter:
- **strategy_creation**: Overhead of creating retry strategies
- **jitter**: Jitter calculation overhead (none/full/equal/decorrelated)
- **successful_operation**: No retries needed (best case)
- **with_failures**: Operation fails N-1 times
- **backoff_comparison**: Exponential vs Fibonacci
- **should_retry**: Error classification check

**Key insights:**
- Jitter calculation overhead
- Impact of max_attempts on performance
- Backoff strategy comparisons

## Interpreting Results

### Look for:
1. **Throughput**: Operations/second
2. **Latency**: Time per operation (µs or ns)
3. **Scaling**: How performance changes with concurrency

### Criterion Output:
```
circuit_breaker/closed/execute_success/5
                        time:   [2.1234 µs 2.1567 µs 2.1923 µs]
                        thrpt:  [456.23 Kelem/s 463.45 Kelem/s 470.89 Kelem/s]
```

- **time**: Mean ± confidence interval
- **thrpt**: Throughput (elements/second)

### HTML Reports:
Open `target/criterion/report/index.html` for:
- Interactive charts
- Statistical analysis
- Historical comparison
- Violin plots showing distribution

## Comparing Before/After

To measure the impact of optimizations:

```bash
# Baseline (before optimization)
git checkout <before-commit>
cargo bench -p nebula-resilience --bench manager -- --save-baseline before

# After optimization
git checkout <after-commit>
cargo bench -p nebula-resilience --bench manager -- --baseline before
```

Criterion will show percentage improvements!

## Performance Targets

### Circuit Breaker
- Closed path: < 1µs per operation
- State check: < 100ns
- State transition: < 10µs

### Rate Limiter
- acquire(): < 5µs (TokenBucket)
- acquire(): < 10µs (Governor/GCRA)
- Throughput: > 100K ops/sec single-threaded

### Manager
- execute() overhead: < 2µs (no patterns)
- Policy lookup: < 500ns (DashMap)
- Concurrent access: Linear scaling up to 100 tasks

### Retry
- Strategy creation: < 100ns
- Jitter calculation: < 50ns
- Successful operation: < 1µs overhead

## CI Integration

Add to CI pipeline:
```yaml
- name: Run benchmarks
  run: cargo bench -p nebula-resilience --no-fail-fast

- name: Archive benchmark results
  uses: actions/upload-artifact@v3
  with:
    name: criterion-results
    path: target/criterion/
```

## Contributing

When adding new benchmarks:
1. Use descriptive names
2. Test realistic scenarios
3. Include both best-case and worst-case
4. Document what you're measuring
5. Set appropriate sample sizes
6. Add to this README

## Notes

- Benchmarks use `async_tokio` runtime
- Sample sizes are adjusted for expensive operations
- Results are in `target/criterion/`
- Criterion automatically detects regressions
