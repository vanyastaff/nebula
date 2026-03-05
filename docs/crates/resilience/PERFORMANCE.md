# Performance

## Goal

Keep `nebula-resilience` fast under high concurrency and burst traffic, with repeatable benchmark comparisons for every optimization wave.

## Benchmark Harness

- Windows (PowerShell): `scripts/bench-resilience.ps1`
- Linux/macOS (Bash): `scripts/bench-resilience.sh`

Supported modes:

- `single` — run benchmarks without baseline comparison
- `baseline` — save a Criterion baseline
- `compare` — compare current code against saved baseline

## Recommended Workflow (A/B)

1. Save baseline on reference revision:
   - PowerShell: `./scripts/bench-resilience.ps1 -Mode baseline -Baseline main`
   - Bash: `./scripts/bench-resilience.sh baseline main`
2. Checkout optimization revision and run compare:
   - PowerShell: `./scripts/bench-resilience.ps1 -Mode compare -Baseline main`
   - Bash: `./scripts/bench-resilience.sh compare main`
3. Review Criterion reports in `target/criterion/`.

## Current Snapshot (2026-03-05)

Post-optimization measurements (release/criterion) on local development machine.

### Manager

- `manager/policy_lookup/get_policy_registered`: ~251–258 ns
- `manager/policy_lookup/get_policy_default`: ~247–252 ns
- `manager/execute_overhead/no_patterns`: ~254–264 ns
- `manager/execute_overhead/with_timeout`: ~255–268 ns
- `manager/execute_overhead/full_policy`: ~251–257 ns
- `manager/concurrent_access/concurrent_execute/10`: ~15.8–16.9 µs
- `manager/concurrent_access/concurrent_execute/50`: ~36.4–38.3 µs
- `manager/concurrent_access/concurrent_execute/100`: ~60.0–61.6 µs
- `manager/metrics/get_metrics_single`: ~102–104 ns
- `manager/metrics/get_all_metrics/100`: ~28.0–28.4 µs

### Rate Limiter

- `rate_limiter/execute/token_bucket_1000rps`: ~179–183 ns
- `rate_limiter/execute/adaptive_1000rps`: ~267–271 ns
- `rate_limiter/execute/governor_1000rps`: ~82.1–83.6 ns
- `rate_limiter/contention/concurrent_acquire/10`: ~12.7–14.0 µs
- `rate_limiter/contention/concurrent_acquire/50`: ~47.8–49.7 µs
- `rate_limiter/contention/concurrent_acquire/100`: ~97.8–100.0 µs
- `rate_limiter/contention/governor_concurrent_acquire/10`: ~10.0–10.9 µs
- `rate_limiter/contention/governor_concurrent_acquire/50`: ~26.7–28.0 µs
- `rate_limiter/contention/governor_concurrent_acquire/100`: ~45.1–46.4 µs

### Circuit Breaker

- `circuit_breaker/closed/execute_success/5`: ~216–220 ns
- `circuit_breaker/closed/execute_success/10`: ~215–220 ns
- `circuit_breaker/can_execute/closed`: ~38.3–38.8 ns
- `circuit_breaker/can_execute/open`: ~138–140 ns
- `circuit_breaker/transitions/closed_to_open`: ~2.22–2.26 µs
- `circuit_breaker/transitions/halfopen_to_closed`: ~13.1–15.3 µs
- `circuit_breaker/stats/stats_collection`: ~77.8–79.0 ns

### Timeout

- `timeout/overhead/direct_yield_once`: ~11.3–11.4 ns
- `timeout/overhead/wrapped_yield_once`: ~108.1–109.5 ns
- `timeout/cancellation/pending_future_1ms`: ~12.4–14.5 ms
- `timeout/cancellation/pending_future_5ms`: ~15.2–15.4 ms

### Bulkhead

- `bulkhead/acquire/fast_path/4`: ~42.5–43.8 ns
- `bulkhead/acquire/fast_path/16`: ~41.1–41.6 ns
- `bulkhead/acquire/fast_path/64`: ~41.3–41.7 ns
- `bulkhead/execute/no_timeout`: ~42.5–43.8 ns
- `bulkhead/contention/concurrent_execute/10`: ~14.3–15.1 µs
- `bulkhead/contention/concurrent_execute/50`: ~34.6–35.7 µs
- `bulkhead/contention/concurrent_execute/100`: ~78.5–80.0 µs
- `bulkhead/queue_timeout/acquire_timeout_1ms`: ~11.3–12.8 ms

## Notes

- Absolute numbers vary by CPU, governor, background load, and thermal state.
- Use Criterion baseline comparisons (`compare` mode) for regression decisions, not single-run absolute values.
- For release gating, compare against the last accepted baseline and attach summary deltas in PR description.
- Budget thresholds and gate policy are defined in [PERFORMANCE_BUDGET.md](PERFORMANCE_BUDGET.md).

## First A/B Findings (2026-03-05)

Using `scripts/bench-resilience.ps1 -Mode single -Benches manager` with existing Criterion baseline data:

- Improved under contention on execute path:
   - `concurrent_execute/50`: about **6.4% faster**
   - `concurrent_execute/100`: about **16.2% faster**
- Policy lookup also improved slightly (`get_policy_registered`: about **3.2% faster**).
- Metrics read path regressed after adding runtime counters:
   - `get_metrics_single`: about **15.4% slower**
   - `get_all_metrics/10`: about **7.2% slower**

Interpretation: write-path and high-concurrency execution gains are real; metrics retrieval cost increased due to additional atomic snapshots. Keep this trade-off explicit in performance reviews.

## Second A/B Findings (2026-03-05)

Using `scripts/bench-resilience.ps1 -Mode single -Benches rate_limiter` after single-lock refactor in `TokenBucket` and `LeakyBucket`:

- Strong improvements in acquire paths:
   - `token_bucket/*`: about **38–40% faster**
   - `leaky_bucket/*`: about **37–43% faster**
- Execute path improved:
   - `execute/token_bucket_1000rps`: about **31% faster**
   - `execute/adaptive_1000rps`: about **10% faster**
- Contention improved at higher concurrency:
   - `concurrent_acquire/50`: about **25% faster**
   - `concurrent_acquire/100`: about **30% faster**

Interpretation: consolidating mutable algorithm state under one lock significantly reduced lock handoff overhead and contention in rate limiter hot paths.

## Third A/B Findings (2026-03-05)

Using `scripts/bench-resilience.ps1 -Mode single -Benches rate_limiter` after single-lock cleanup/check refactor in `SlidingWindow`:

- Strong improvements in `SlidingWindow` hot paths:
   - `acquire/sliding_window/*`: about **36–37% faster**
   - `current_rate/sliding_window`: about **20–25% faster**
- Adaptive execute path also improved in this run:
   - `execute/adaptive_1000rps`: about **8–14% faster**
- Token/leaky acquire paths showed small-to-moderate regressions versus the currently stored Criterion baseline in this run.

Interpretation: removing the extra lock cycle in `SlidingWindow` produced clear wins; token/leaky fluctuations should be validated with a fresh locked baseline/compare cycle before treating them as true regressions.

## Fourth A/B Findings (2026-03-05)

Using `scripts/bench-resilience.ps1 -Mode single -Benches circuit_breaker` after lock-contention refactor in `CircuitBreaker::can_execute()` and `state()`:

- Major improvements in state-check path:
   - `can_execute/closed`: about **81–82% faster**
   - `can_execute/open`: about **18–21% faster**
- Transition/stat paths remained within noise threshold in this run.
- Closed execute path showed a small regression versus stored baseline:
   - `closed/execute_success/*`: about **1–7% slower**

Interpretation: contention-focused optimization succeeded for the target high-frequency gate check path (`can_execute`), while minor execute-path drift should be re-validated with fresh baseline/compare runs before hard conclusions.

## Fifth A/B Findings (2026-03-05)

Using `scripts/bench-resilience.ps1 -Mode single -Benches rate_limiter` after migrating rate-limiter internal locks from `tokio::sync` to `parking_lot` in hot paths:

- Strong contention-path improvements:
   - `contention/concurrent_acquire/10`: about **49–53% faster**
   - `contention/concurrent_acquire/50`: about **35–38% faster**
   - `contention/concurrent_acquire/100`: about **35–37% faster**
- Acquire and execute paths improved broadly:
   - `acquire/*`: about **21–32% faster**
   - `execute/token_bucket_1000rps`: about **17–18% faster**
   - `execute/adaptive_1000rps`: about **18–21% faster**
- Read-side rate queries also improved:
   - `current_rate/token_bucket`: about **42–47% faster**
   - `current_rate/sliding_window`: about **37–41% faster**

Interpretation: for these short critical sections (no `await` under lock), moving to `parking_lot` materially reduced synchronization overhead and closed the remaining `RSL-T008` contention gap.

## Sixth Findings: Compose Deep-Chain Profile (2026-03-05)

Using `cargo bench -p nebula-resilience --bench compose -- --noplot` with synthetic no-op layers across depths `1/3/5/8/12/16`:

- Build overhead scales near-linearly by depth:
   - `build/depth/1`: about **120 ns**
   - `build/depth/8`: about **0.79 µs**
   - `build/depth/16`: about **1.36 µs**
- Execute overhead also scales near-linearly by depth:
   - `execute/depth/1`: about **0.21 µs**
   - `execute/depth/8`: about **0.75 µs**
   - `execute/depth/16`: about **1.77 µs**
- Retryable-op clone path remains in the same order of magnitude:
   - `execute_retryable_clone/depth/16`: about **1.68 µs**

Interpretation: layer composition overhead is predictable and mostly linear with chain depth; at depth 16 the framework-only overhead stays sub-2µs for no-op layers, which is acceptable for typical policy stacks.

## Seventh Findings: Governor Rate Limiter Coverage (2026-03-05)

Using `cargo bench -p nebula-resilience --bench rate_limiter -- --noplot` after adding dedicated governor benchmark cases and tuning `retry_after` clock sourcing in `GovernorRateLimiter`:

- Governor acquire path is consistently low-latency across configured rates:
   - `acquire/governor/100`: about **55.3–59.0 ns**
   - `acquire/governor/1000`: about **54.9–55.6 ns**
   - `acquire/governor/10000`: about **54.8–56.4 ns**
- Governor execute path is the fastest measured among current limiter variants in this run:
   - `execute/governor_1000rps`: about **82.1–83.6 ns**
   - `execute/token_bucket_1000rps`: about **102.4–105.8 ns**
   - `execute/adaptive_1000rps`: about **169.6–193.3 ns**
- Governor contention scales predictably and remains below token-bucket contention in the same run:
   - `governor_concurrent_acquire/10`: about **10.0–10.9 µs**
   - `governor_concurrent_acquire/50`: about **26.7–28.0 µs**
   - `governor_concurrent_acquire/100`: about **45.1–46.4 µs**

Interpretation: dedicated governor coverage confirms stable sub-100ns hot-path overhead and competitive contention behavior under concurrent pressure; this closes `RSL-T027` and provides a baseline for subsequent timeout/fallback/hedge expansion work.

## Eighth Findings: Timeout Overhead and Cancellation Latency (2026-03-05)

Using `cargo bench -p nebula-resilience --bench timeout -- --noplot` after adding a dedicated timeout benchmark target:

- Wrapper overhead on success path remains low and predictable:
   - `overhead/direct_yield_once`: about **11.3–11.4 ns**
   - `overhead/wrapped_yield_once`: about **108.1–109.5 ns**
   - Estimated timeout-wrapper overhead over direct path: about **97 ns** in this run.
- Cancellation-path latency is stable but platform-sensitive on Windows:
   - `cancellation/pending_future_1ms`: about **12.4–14.5 ms**
   - `cancellation/pending_future_5ms`: about **15.2–15.4 ms**
   - Effective overshoot over requested timeout is in the single-digit to low-double-digit millisecond range.

Interpretation: timeout wrapper cost is negligible for typical I/O workloads, while cancellation latency is dominated by runtime/OS timer granularity under short deadlines; this closes `RSL-T028` and establishes a baseline for timeout-specific operational guidance in `RSL-T031`.

## Ninth Findings: Bulkhead Baseline Kickoff (2026-03-05)

Using `cargo bench -p nebula-resilience --bench bulkhead -- --noplot` after adding dedicated `bulkhead` benchmark scenarios:

- Acquire and execute fast paths remain very low overhead:
   - `acquire/fast_path/*`: about **41–44 ns**
   - `execute/no_timeout`: about **42–44 ns**
- Contention scales predictably under concurrent execute load:
   - `concurrent_execute/10`: about **14.3–15.1 µs**
   - `concurrent_execute/50`: about **34.6–35.7 µs**
   - `concurrent_execute/100`: about **78.5–80.0 µs**
- Queue-timeout scenario confirms bounded wait behavior under saturation:
   - `queue_timeout/acquire_timeout_1ms`: about **11.3–12.8 ms** on current Windows runtime.

Interpretation: bulkhead fast path is inexpensive, and the new benchmark baseline establishes a concrete target for Phase 7 fairness/starvation hardening work (`RSL-T033`).

## Tenth Findings: Retry Hardening Baseline (2026-03-05)

Using `cargo bench -p nebula-resilience --bench retry -- --noplot` during Phase 7 consolidation:

- Success-path retry overhead remains low:
   - `retry/successful_operation/fixed_delay/5`: about **162.6–163.7 ns**
   - `retry/successful_operation/fixed_delay/1`: about **163.0–164.0 ns**
- Jitter computation is inexpensive and bounded:
   - `retry/jitter_calculation/full_jitter`: about **8.25–8.34 ns**
   - `retry/jitter_calculation/decorrelated_jitter`: about **8.44–8.54 ns**
- Failure-heavy retry paths are naturally higher-latency and timer-sensitive:
   - `retry/with_failures/fail_until_last/3`: about **20.2–25.3 ms**
   - `retry/with_failures/fail_until_last/5`: about **44.6–50.6 ms**

Interpretation: retry hot-path and jitter overhead are stable and suitable for hard-gate budgeting, while failure-heavy paths should be interpreted with scheduler/timer context as stress indicators rather than strict hard gates.
