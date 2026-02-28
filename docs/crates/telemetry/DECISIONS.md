# Decisions

## D001: Fire-and-Forget Event Delivery

**Status:** Adopt

**Context:** Events are projections for dashboards/audit; execution must not block on telemetry.

**Decision:** Use `tokio::sync::broadcast`; emit returns immediately; if no subscribers, events are dropped. Subscribers that lag receive `RecvError::Lagged` and skip missed events.

**Alternatives considered:** Reliable queue (Kafka, etc.); blocking until delivered; persistent log.

**Trade-offs:** Events can be lost; acceptable for observability use case. ExecutionRepo remains source of truth.

**Consequences:** Subscribers must handle lag; no backpressure to emitter.

**Migration impact:** None.

**Validation plan:** Unit tests for emit-without-subscribers; integration tests for subscriber receive.

---

## D002: In-Memory Metrics First

**Status:** Adopt

**Context:** MVP and desktop use case need zero external dependencies.

**Decision:** Counter, Gauge, Histogram store values in process memory. No Prometheus/OTLP export in initial implementation.

**Alternatives considered:** Prometheus crate as direct dep; metrics-only crate.

**Trade-offs:** No production dashboard without exporter; acceptable for Phase 1.

**Consequences:** Phase 2 adds exporter module (optional feature).

**Migration impact:** None.

**Validation plan:** Unit tests for atomic semantics; benchmark for hot-path overhead.

---

## D003: TelemetryService Trait

**Status:** Adopt

**Context:** Engine and runtime need pluggable telemetry for testing and different deployments.

**Decision:** `TelemetryService` trait with `event_bus()` and `metrics()`; `NoopTelemetry` as default implementation. Consumers receive `Arc<dyn TelemetryService>`.

**Alternatives considered:** Concrete types only; separate event and metrics traits.

**Trade-offs:** Trait object indirection; clear abstraction for DI.

**Consequences:** Future implementations (e.g. PrometheusTelemetry) implement same trait.

**Migration impact:** None.

**Validation plan:** Integration tests with NoopTelemetry; engine tests with real EventBus/MetricsRegistry.

---

## D004: ExecutionEvent as Enum (No Generic Payload)

**Status:** Adopt

**Context:** Strongly typed events enable schema evolution and clear documentation.

**Decision:** `ExecutionEvent` is an enum with fixed variants (Started, NodeStarted, NodeCompleted, etc.). No generic `Custom` variant.

**Alternatives considered:** Generic event with type-erased payload; protobuf/JSON schema only.

**Trade-offs:** Adding new lifecycle events requires enum change; clearer than opaque payloads.

**Consequences:** Schema changes are additive (new variants); removal is breaking.

**Migration impact:** Additive changes are non-breaking.

**Validation plan:** Serialization roundtrip tests; consumer compatibility tests.

---

## D005: Histogram Stores All Observations

**Status:** Adopt (with known limitation)

**Context:** Simple implementation for MVP; no external histogram library.

**Decision:** `Histogram` stores each observation in `Vec<f64>`. Suitable for dev/test; not for high cardinality or long-running production.

**Alternatives considered:** Bucketed histogram (Prometheus-style); HDR histogram; external crate.

**Trade-offs:** Unbounded memory growth; simple API. Defer optimization to Phase 2.

**Consequences:** Production deployments should use bounded/bucketed implementation (see PROPOSALS.md).

**Migration impact:** Future replacement may require API change.

**Validation plan:** Unit tests for sum/count; document memory implications.
