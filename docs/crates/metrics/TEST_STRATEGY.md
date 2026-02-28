# Test Strategy

## Test Pyramid

- **Unit:** Metric recording (telemetry, log); naming helpers; export format generation.
- **Integration:** Prometheus scrape; OTLP push; adapter reads from telemetry.
- **Contract:** Prometheus format valid; metric names stable.
- **End-to-end:** Scrape → Grafana; alert evaluation.

## Critical Invariants

- Recording never blocks execution.
- Export failures do not affect recording.
- Prometheus output is valid (parseable).
- No secrets in metric names or labels.

## Scenario Matrix

| Scenario | Coverage |
|----------|----------|
| Happy path | Record → scrape → valid output |
| Export failure | Recording continues; export retries or fails |
| High cardinality | Bounded; no OOM |
| Upgrade/migration | Metric name changes documented |

## Tooling

- **Property testing:** Optional; export format roundtrip.
- **Fuzzing:** Optional; metric name/label fuzz.
- **Benchmarks:** Recording latency; scrape throughput.
- **CI quality gates:** `cargo test`; Prometheus format validation.

## Exit Criteria

- **Coverage goals:** Export path tested; recording path tested.
- **Flaky test budget:** Zero.
- **Performance regression:** Recording < 1µs; scrape < 100ms.
