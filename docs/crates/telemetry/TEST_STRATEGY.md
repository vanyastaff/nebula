# Test Strategy

## Test Pyramid

- **Unit:** EventBus emit/subscribe; Counter, Gauge, Histogram; MetricsRegistry; NoopTelemetry. Fast, no I/O.
- **Integration:** Engine + telemetry (event sequence); runtime + telemetry (action metrics). Use real EventBus and MetricsRegistry.
- **Contract:** Event schema serialization; metric names stability. Cross-crate compatibility.
- **End-to-end:** Not applicable for telemetry alone; covered by engine/runtime E2E.

## Critical Invariants

- Emit without subscribers does not panic.
- Subscriber receives events in order when not lagging.
- Same metric name returns same Counter/Gauge/Histogram instance.
- NoopTelemetry satisfies TelemetryService and never panics.
- ExecutionEvent serialization roundtrip preserves data.

## Scenario Matrix

| Scenario | Coverage |
|----------|----------|
| Happy path | emit → subscribe → recv; counter inc; gauge set; histogram observe |
| Retry path | N/A (no retries) |
| Cancellation path | Subscriber drop; EventBus continues |
| Timeout path | N/A (no I/O) |
| Upgrade/migration path | ExecutionEvent schema additive; MIGRATION.md |

## Tooling

- **Property testing:** Optional: proptest for ExecutionEvent serialization.
- **Fuzzing:** Optional: serde_json fuzz for event deserialization.
- **Benchmarks:** Emit throughput; metric record latency. `cargo bench` when added.
- **CI quality gates:** `cargo test -p nebula-telemetry`; `cargo test -p nebula-engine`; `cargo test -p nebula-runtime`.

## Exit Criteria

- **Coverage goals:** All public APIs exercised; event variants covered.
- **Flaky test budget:** Zero; tests are deterministic.
- **Performance regression thresholds:** Emit < 1µs; no allocation in hot path (when measured).
