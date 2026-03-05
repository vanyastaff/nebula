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
- `rate_limiter/contention/concurrent_acquire/10`: ~12.7–14.0 µs
- `rate_limiter/contention/concurrent_acquire/50`: ~47.8–49.7 µs
- `rate_limiter/contention/concurrent_acquire/100`: ~97.8–100.0 µs

### Circuit Breaker

- `circuit_breaker/closed/execute_success/5`: ~216–220 ns
- `circuit_breaker/closed/execute_success/10`: ~215–220 ns
- `circuit_breaker/can_execute/closed`: ~38.3–38.8 ns
- `circuit_breaker/can_execute/open`: ~138–140 ns
- `circuit_breaker/transitions/closed_to_open`: ~2.22–2.26 µs
- `circuit_breaker/transitions/halfopen_to_closed`: ~13.1–15.3 µs
- `circuit_breaker/stats/stats_collection`: ~77.8–79.0 ns

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
