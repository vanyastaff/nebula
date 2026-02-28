# Interactions

## Ecosystem Map (Current + Planned)

### Existing Crates

| Crate | Relationship | Description |
|-------|-------------|-------------|
| `nebula-core` | Upstream | Identifiers (ExecutionId, NodeId, WorkflowId) used in ExecutionEvent |
| `nebula-engine` | Downstream | Emits ExecutionEvent, records workflow-level metrics |
| `nebula-runtime` | Downstream | Emits ExecutionEvent (NodeStarted/Completed/Failed), records action metrics |
| `nebula-log` | Sibling | Logging, OTel traces, Sentry; separate observability layer |
| `nebula-execution` | Indirect | ExecutionStatus, ExecutionBudget; engine uses for event context |

### Planned Crates

- **nebula-worker:** Will consume EventBus for execution status; record worker-level metrics
- **nebula-api:** May expose metrics endpoint (Prometheus scrape) or subscribe to events for real-time UI

## Downstream Consumers

### nebula-engine

- **Expectations:** `Arc<EventBus>`, `Arc<MetricsRegistry>`; emits Started, NodeStarted, NodeCompleted, NodeFailed, Completed, Failed, Cancelled
- **Contract:** Sync emit; no await; events are fire-and-forget

### nebula-runtime

- **Expectations:** Same as engine; records `actions_executed_total`, `actions_failed_total`, `action_duration_seconds`
- **Contract:** Sync emit and metric recording; never blocks execution

## Upstream Dependencies

| Crate | Why needed | Hard contract | Fallback |
|-------|------------|---------------|----------|
| `nebula-core` | Id types (indirect via engine) | — | — |
| `tokio` | broadcast channel | `broadcast::Sender` | — |
| `serde` | ExecutionEvent serialization | Serialize/Deserialize | — |
| `tracing` | Optional span correlation | — | — |

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|----------------------|-----------|----------|------------|------------------|-------|
| telemetry -> engine | out | EventBus, MetricsRegistry injection | sync | emit never fails | Engine owns emit calls |
| telemetry -> runtime | out | EventBus, MetricsRegistry injection | sync | emit never fails | Runtime owns emit calls |
| engine -> telemetry | in | emit(ExecutionEvent), metrics.inc() | sync | best-effort | Fire-and-forget |
| runtime -> telemetry | in | emit(ExecutionEvent), metrics.inc/observe | sync | best-effort | Fire-and-forget |
| telemetry -> log | — | None | — | — | Log has OTel/Sentry; telemetry has events/metrics |

## Runtime Sequence

1. Application constructs `NoopTelemetry::arc()` or custom `TelemetryService`.
2. Engine and runtime receive `event_bus` and `metrics` (or via trait).
3. On execution start: engine emits `Started`, increments counters.
4. On node execution: runtime emits `NodeStarted`; on completion/failure emits `NodeCompleted`/`NodeFailed`, records histogram.
5. On execution end: engine emits `Completed`/`Failed`/`Cancelled`.
6. Optional: subscriber task receives events for dashboard/audit.

## Cross-Crate Ownership

| Responsibility | Owner |
|----------------|-------|
| Event schema (ExecutionEvent) | `nebula-telemetry` |
| Metric names convention | `nebula-telemetry` (doc) + consumers (usage) |
| Emit timing and content | `nebula-engine`, `nebula-runtime` |
| Source of truth for execution state | `ports::ExecutionRepo` (not telemetry) |
| Logging, traces, error tracking | `nebula-log` |

## Failure Propagation

- **How failures bubble up:** Emit and metric recording do not return `Result`; they are infallible from caller perspective.
- **Where retries apply:** N/A; no I/O in hot path.
- **Where retries forbidden:** N/A.

## Versioning and Compatibility

- **Compatibility promise:** ExecutionEvent schema additive-only; new variants allowed; removal requires major bump.
- **Breaking-change protocol:** Major version bump; migration guide in MIGRATION.md.
- **Deprecation window:** Minimum 2 minor releases.

## Contract Tests Needed

- [ ] Engine emits expected event sequence for successful/failed/cancelled execution
- [ ] Runtime emits NodeStarted/NodeCompleted/NodeFailed with correct payloads
- [ ] Metric names `actions_executed_total`, `actions_failed_total`, `action_duration_seconds` stable
- [ ] EventBus subscriber receives events in order (when not lagging)
- [ ] NoopTelemetry satisfies TelemetryService and does not panic
