# Target State

This document defines the **target** for Nebula metrics: unified export and a standard naming convention. It is the reference for Phase 2 (naming and adapters) and Phase 3 (export implementation).

---

## 1. Unified Export

### Goals

- **Single scrape endpoint:** One `/metrics` (Prometheus) per process; optional OTLP push.
- **Stability:** Export failures do not affect execution; recording is best-effort and non-blocking.
- **Format:** Prometheus text format; OTLP when feature-enabled.

### Components

| Component | Responsibility |
|-----------|----------------|
| **Registry (or adapter)** | Single place to read all metric values — either telemetry `MetricsRegistry` or a unified registry/adapter that can pull from telemetry + adapters for log/domain crates. |
| **Prometheus exporter** | Renders registry (or adapter) to Prometheus text format; serves via HTTP GET `/metrics`. |
| **OTLP exporter (optional)** | Pushes metrics to an OTLP collector; feature-gated. |

### Non-goals

- In-memory primitives remain in `nebula-telemetry` (see [DECISIONS.md](./DECISIONS.md)).
- Domain crates may keep internal metric types; export is via adapters or standard `metrics` crate recorder, not by moving all logic into one crate.

---

## 2. Naming Convention

### Rule: `nebula_<domain>_<metric>_<unit>`

- **Prefix:** `nebula_` — avoids collisions and identifies Nebula metrics.
- **Domain:** workflow, action, node, credential, resource, etc.
- **Metric:** short, lowercase, snake_case (e.g. `executions_total`, `duration_seconds`).
- **Unit suffix:** `_total` for counters, `_seconds` for duration, `_bytes` for size when applicable.

### Target names (to be adopted in Phase 2)

| Current (engine/runtime) | Target |
|-------------------------|--------|
| `executions_started_total` | `nebula_workflow_executions_started_total` |
| `executions_completed_total` | `nebula_workflow_executions_completed_total` |
| `executions_failed_total` | `nebula_workflow_executions_failed_total` |
| `execution_duration_seconds` | `nebula_workflow_execution_duration_seconds` |
| `actions_executed_total` | `nebula_action_executions_total` |
| `actions_failed_total` | `nebula_action_failures_total` |
| `action_duration_seconds` | `nebula_action_duration_seconds` |

### Domain-specific (documented for Phase 2)

- **Workflow:** `nebula_workflow_*` — executions, duration, node counts.
- **Action/Node:** `nebula_action_*`, `nebula_node_*` — executions, failures, duration.
- **Credential:** `nebula_credential_*` — operations, rotation, errors.
- **Resource:** `nebula_resource_*` — create, acquire, release, cleanup, pool, health (align with current `resource.*` names and add prefix).
- **Eventbus (when integrated):** `nebula_eventbus_*` — sent, dropped, subscribers (see eventbus ROADMAP Phase 3).

### Labels

- Keep label sets **bounded** (e.g. resource_id, workflow_id, status).
- Avoid high-cardinality labels (e.g. full request IDs) in default export.

---

## 3. Readiness Criteria (from ROADMAP)

| Metric | Target |
|--------|--------|
| **Correctness** | Prometheus format valid; metric values accurate |
| **Latency** | Scrape < 100ms; no impact on recording |
| **Throughput** | 10k metrics/sec recording; scrape handles all |
| **Stability** | Export failures do not affect execution |
| **Operability** | Dashboards; alert rules; runbook |

---

## 4. Relationship to Telemetry and Eventbus

- **Telemetry ROADMAP** Phase 2: Bounded Histogram + document `nebula_*` naming. Phase 3: Prometheus/OTLP export. Metrics target aligns with that; either telemetry gains export or a dedicated metrics crate provides it with an adapter to telemetry.
- **Eventbus ROADMAP** Phase 3: EventBusStats integration with metrics. Target: eventbus stats exposed under `nebula_eventbus_*` (or equivalent) when unified export exists.

See [ROADMAP.md](./ROADMAP.md) for phase ordering and exit criteria.
