# Example: Record EventBus Metrics

Use `nebula-metrics::TelemetryAdapter` to expose standardized `nebula_eventbus_*` metrics.

```rust
use std::sync::Arc;

use nebula_eventbus::EventBus;
use nebula_metrics::TelemetryAdapter;
use nebula_telemetry::metrics::MetricsRegistry;

#[derive(Clone)]
struct MyEvent;

fn main() {
    let bus = EventBus::<MyEvent>::new(128);
    let registry = Arc::new(MetricsRegistry::new());
    let metrics = TelemetryAdapter::new(registry);

    let _sub = bus.subscribe();
    let _ = bus.emit(MyEvent);
    let _ = bus.emit(MyEvent);

    metrics.record_eventbus_stats(&bus.stats());

    assert_eq!(metrics.eventbus_sent().get(), 2);
    assert_eq!(metrics.eventbus_dropped().get(), 0);
    assert_eq!(metrics.eventbus_subscribers().get(), 1);
}
```

## Suggested scrape/update cadence

- snapshot every 1-10 seconds in a background task,
- store gauge values only (no per-event metric mutation on hot path).
