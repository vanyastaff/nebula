# Proposals

## P001: Bounded/Bucketed Histogram

**Type:** Non-breaking (additive)

**Motivation:** Current Histogram stores all observations in `Vec<f64>`, leading to unbounded memory growth. Production deployments need bounded memory and Prometheus-compatible bucketing.

**Proposal:** Introduce `BucketedHistogram` or replace `Histogram` with bucketed implementation (e.g. Prometheus-style buckets). Keep existing `Histogram` as deprecated or behind feature flag for compatibility.

**Expected benefits:** Bounded memory; Prometheus-native export; production-ready.

**Costs:** API change or new type; migration for consumers using Histogram.

**Risks:** Breaking change if we replace in place; need deprecation path.

**Compatibility impact:** Additive if new type; breaking if replacement.

**Status:** Draft

---

## P002: Prometheus Exporter

**Type:** Non-breaking

**Motivation:** Production deployments need to scrape metrics into Prometheus for dashboards and alerting.

**Proposal:** Add optional `prometheus` feature; implement `PrometheusTelemetry` or separate `MetricsExporter` that exposes `/metrics` endpoint. Use `prometheus` crate or manual formatting.

**Expected benefits:** Standard observability stack; Grafana integration.

**Costs:** New dependency (feature-gated); cardinality management.

**Risks:** High cardinality from workflow_id/node_id labels; need naming convention.

**Compatibility impact:** None; additive feature.

**Status:** Draft

---

## P003: Event Schema Versioning

**Type:** Non-breaking

**Motivation:** As ExecutionEvent evolves, subscribers may need to detect schema version for compatibility.

**Proposal:** Add optional `version` field to serialized ExecutionEvent or emit schema version in first event. Subscribers can check version before processing.

**Expected benefits:** Forward/backward compatibility; clearer upgrade path.

**Costs:** Slight payload overhead; version negotiation logic.

**Risks:** Low; additive.

**Compatibility impact:** Additive; old subscribers ignore version.

**Status:** Draft

---

## P004: Trace ID in ExecutionEvent

**Type:** Non-breaking

**Motivation:** Correlate telemetry events with OpenTelemetry traces from nebula-log.

**Proposal:** Add optional `trace_id: Option<String>` and `span_id: Option<String>` to ExecutionEvent variants. Engine/runtime set from `tracing::Span::current()` when available.

**Expected benefits:** Unified observability; trace-event correlation in Jaeger/Grafana.

**Costs:** Slightly larger event payload; dependency on tracing context.

**Risks:** Optional fields; no breaking change.

**Compatibility impact:** Additive.

**Status:** Draft
