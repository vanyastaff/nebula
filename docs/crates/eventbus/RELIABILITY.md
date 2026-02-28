# Reliability

## SLO Targets

| Metric | Target | Notes |
|--------|--------|-------|
| **Availability** | 100% (in-process) | Eventbus has no external deps; process up = bus up |
| **Latency** | emit < 1µs (sync) | Fire-and-forget; no blocking |
| **Error budget** | N/A | No user-facing errors; events are best-effort |

## Failure Modes

### Dependency Outage

- **tokio runtime:** Eventbus requires tokio. If runtime is down, process is down. No partial degradation.
- **No external I/O** in Phase 1–3.

### Timeout/Backpressure

- **Buffer full:** Policy determines behavior. DropOldest: oldest event overwritten; DropNewest: new event dropped; Block: wait up to timeout then drop.
- **Slow subscriber:** Subscriber receives `Lagged`; skips to latest. No retry of missed events.
- **No deadlock:** Emit never blocks (sync path); emit_async has timeout.

### Partial Degradation

- **No subscribers:** Events dropped silently. Emitters unaffected.
- **One subscriber slow:** Others unaffected (broadcast fan-out).
- **High drop rate:** EventBusStats.dropped increases; observability may be incomplete. Operational alert.

### Data Corruption

- **Event corruption:** Generic type E; no deserialization in eventbus. Corruption would be in emitter or subscriber.
- **Buffer corruption:** tokio broadcast is robust; no known corruption modes.

## Resilience Strategies

### Retry Policy

- **Emit:** No retry; fire-and-forget. Events are projections.
- **Recv:** Subscriber may retry recv() on Lagged (skip to latest). EventSubscriber handles Lagged internally.

### Circuit Breaking

- N/A; no external calls. Internal broadcast does not support circuit breaking.

### Fallback Behavior

- **No subscribers:** Drop event; no error.
- **Buffer full:** Per BackPressurePolicy.
- **Subscriber dropped:** Sender continues; no impact on other subscribers.

### Graceful Degradation

- Under load: increase buffer_size or use DropNewest to protect emitters.
- Subscriber slow: accept Lagged; reduce handler work or add more subscriber instances.

## Operational Runbook

### Alert Conditions

- `EventBusStats.dropped` > threshold (e.g. 1% of emitted)
- `EventBusStats.subscribers` drops to 0 unexpectedly
- Subscriber recv latency > threshold (if measured)

### Dashboards

- emitted, dropped, subscribers per EventBus instance
- Emit rate (events/sec)
- Lagged count (if exposed)

### Incident Triage Steps

1. Check EventBusStats: emitted vs dropped
2. Check subscriber_count: are consumers running?
3. Check buffer_size: too small for load?
4. Check handler latency: slow subscriber?
5. Consider BackPressurePolicy change or buffer increase

## Capacity Planning

### Load Profile Assumptions

- Execution events: ~10–100 per second per workflow (depends on node count)
- Resource events: ~1–10 per second per resource type
- Peak: 10x baseline during bulk operations

### Scaling Constraints

- **Buffer size:** 1024 default; 4096–8192 for high throughput. Memory: ~buffer_size * size_of(E).
- **Subscribers:** Each subscriber holds a clone of the broadcast receiver; minimal overhead.
- **Single process:** Phase 1–3; horizontal scaling = more processes, each with own buses.
