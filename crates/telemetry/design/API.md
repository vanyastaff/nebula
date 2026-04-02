# API

## Public Surface

### Stable APIs

- `EventBus` — broadcast-based event distribution
- `EventSubscriber` — subscription handle for receiving events
- `ExecutionEvent` — execution lifecycle event enum
- `Counter`, `Gauge`, `Histogram` — metric primitives
- `MetricsRegistry` — named metric creation and retrieval
- `NoopMetricsRegistry` — no-op registry for testing
- `TelemetryService` — trait for telemetry facade
- `NoopTelemetry` — no-op implementation

### Experimental APIs

- None; all public APIs are considered stable for current scope.

### Hidden/Internal APIs

- All types in `event`, `metrics`, `service` are public; no internal-only modules.

## Usage Patterns

### Dependency Injection

Consumers receive `Arc<EventBus>` and `Arc<MetricsRegistry>` (or `Arc<dyn TelemetryService>`) at construction. Engine and runtime are built with these injected.

### Event Emission

Emit from engine/runtime on lifecycle transitions; never await. Subscribers run in separate tasks.

### Metrics Recording

Call `metrics.counter("name").inc()` etc. from hot path; operations are atomic and non-blocking.

## Minimal Example

```rust
use nebula_telemetry::event::{EventBus, ExecutionEvent};
use nebula_telemetry::metrics::MetricsRegistry;
use std::sync::Arc;

let bus = Arc::new(EventBus::new(64));
let metrics = Arc::new(MetricsRegistry::new());

// Emit event
bus.emit(ExecutionEvent::Started {
    execution_id: "exec-1".into(),
    workflow_id: "wf-1".into(),
});

// Record metric
metrics.counter("executions_total").inc();
```

## Advanced Example

```rust
use nebula_telemetry::service::{NoopTelemetry, TelemetryService};
use std::sync::Arc;

// Use facade for DI
let telemetry: Arc<dyn TelemetryService> = NoopTelemetry::arc();

let mut sub = telemetry.event_bus().subscribe();
telemetry.event_bus().emit(ExecutionEvent::NodeCompleted {
    execution_id: "e1".into(),
    node_id: "n1".into(),
    duration: std::time::Duration::from_millis(100),
});

// In async context: let event = sub.recv().await;
let counter = telemetry.metrics().counter("actions_executed_total");
counter.inc();
```

## Error Semantics

- **Retryable errors:** None; emit and record are infallible (fire-and-forget).
- **Fatal errors:** None in public API; Histogram `observe` uses `expect` on lock (panic if poisoned).
- **Validation errors:** N/A; no validation at API boundary.

## Compatibility Rules

- **Major version bump:** Breaking changes to `ExecutionEvent` schema (removing variants), `TelemetryService` trait, or metric type signatures.
- **Deprecation policy:** Deprecate for at least one minor release before removal; document migration path.
