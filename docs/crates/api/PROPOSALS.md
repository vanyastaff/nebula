# Proposals

## P001: Dynamic WorkerStatus from WorkerPool

**Type:** Non-breaking

**Motivation:** Current workers are static snapshot. Live worker state (queue depth, active count) would improve status accuracy.

**Proposal:** ApiState accepts `Arc<dyn WorkerPool>` or similar; status handler queries pool for current worker states. WorkerPool trait in ports or worker crate.

**Expected benefits:** Real-time status; no stale snapshot.

**Costs:** Api depends on worker/ports; trait design.

**Risks:** WorkerPool may not exist yet; coupling.

**Compatibility impact:** Additive; fallback to Vec<WorkerStatus> if pool not provided.

**Status:** Draft

---

## P002: GraphQL layer

**Type:** Non-breaking (additive)

**Motivation:** Archive deferred GraphQL. Some consumers prefer GraphQL for flexible queries.

**Proposal:** Add optional GraphQL layer (async-graphql, juniper) behind feature flag. REST remains primary. GraphQL for complex queries (e.g. workflow with executions, nodes).

**Expected benefits:** Flexible queries; single endpoint for varied clients.

**Costs:** Additional dependency; schema maintenance; resolver implementation.

**Risks:** Two APIs to maintain; possible divergence.

**Compatibility impact:** Additive.

**Status:** Defer

---

## P003: Prometheus metrics endpoint

**Type:** Non-breaking

**Motivation:** Production monitoring needs /metrics for Prometheus scrape.

**Proposal:** Add GET /metrics when metrics feature enabled. Expose request count, latency, error rate. Use metrics crate + metrics-exporter-prometheus.

**Expected benefits:** Standard observability; Grafana dashboards.

**Costs:** metrics dependency; instrumentation in handlers.

**Risks:** Cardinality if high-dimensional labels.

**Compatibility impact:** Additive.

**Status:** Draft
