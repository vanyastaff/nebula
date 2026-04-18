# Spec 18 — Observability stack (logs, metrics, traces, audit, real-time)

> **Status:** draft
> **Canon target:** §4.6 fulfillment, §3.10 update (nebula-log/nebula-metrics/nebula-telemetry scope), new §4.7 observability contract
> **Depends on:** 06 (IDs, including new `InstanceId`/`AttemptId`), 08 (cancel cascade for span lifecycle), 16 (storage — `execution_journal`, `audit_log`)
> **Depended on by:** 19 (error taxonomy — errors flow through logs/traces)

## Problem

Canon §4.6 requires:

> *«Durable is not enough — runs must be explainable. Execution state, append-only journal, structured errors, and metrics should let an operator answer what happened and why a run failed without reading Rust source.»*

We have six separate observability primitives (`nebula-log`, `nebula-metrics`, `nebula-telemetry`, `nebula-eventbus`, `execution_journal`, `audit_log`). Without a unified correlation model, operators must search six places for one answer. That is precisely how Airflow, n8n, and early Temporal all failed their operators.

## Decision

**OpenTelemetry-compliant stack with unified correlation via `trace_id`.** `ObservabilityContext` is a parallel surface to `ScopeLevel` — reuses the same underlying IDs but specialized for telemetry. Minimum viable path (self-host): stdout JSON logs + Prometheus `/metrics` + `execution_journal`. Full path (cloud or ops-serious self-host): OTLP export to Grafana / Loki / Tempo / Prometheus or managed observability backend.

## Architecture diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│ Nebula Process (one InstanceId nbl_01J9...)                          │
│                                                                        │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │ Application layer                                             │   │
│  │   handler() → tracing::info_span!("http", ...)                │   │
│  │     ↓                                                          │   │
│  │   service_layer() → tracing::info!(...)                       │   │
│  │     ↓                                                          │   │
│  │   engine.run_execution() → tracing::info_span!("exec", ...)   │   │
│  │     ↓                                                          │   │
│  │   runtime.run_node() → tracing::info_span!("node", ...)       │   │
│  │     ↓                                                          │   │
│  │   action.execute() — author's code                            │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                              ↓                                         │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │ tracing / tracing-subscriber                                   │   │
│  │   ├ JSON formatter → stdout (always on)                        │   │
│  │   ├ OpenTelemetry layer → OTLP exporter (optional)             │   │
│  │   └ Events attached to spans, inherit attributes               │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                              ↓                                         │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │ nebula-metrics (counter/gauge/histogram)                       │   │
│  │   ├ In-memory aggregation via nebula-telemetry primitives      │   │
│  │   ├ /metrics endpoint (Prometheus scrape) — always on          │   │
│  │   └ OTLP exporter (optional)                                   │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                              ↓                                         │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │ nebula-eventbus (in-process fan-out)                          │   │
│  │   Engine emits ExecutionEvent                                  │   │
│  │     ↓                                                           │   │
│  │     ├ Storage writer → execution_journal (append-only, durable)│   │
│  │     ├ Metrics collector → counters / histograms                │   │
│  │     ├ Websocket broadcaster → live UI clients                  │   │
│  │     └ Audit writer → audit_log for security-relevant events    │   │
│  └─────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
               │                    │                    │
               ↓                    ↓                    ↓
         ┌──────────┐        ┌──────────┐         ┌──────────┐
         │  stdout  │        │  /metrics│         │ OTLP out │
         │ (JSON)   │        │ (Prom)   │         │ (opt)    │
         └─────┬────┘        └─────┬────┘         └─────┬────┘
               ↓                    ↓                    ↓
       Loki / Vector            Prometheus /          Tempo / Jaeger /
       ClickHouse / stdout      VictoriaMetrics       Honeycomb / Datadog
       (operator's choice)      (operator's choice)   (operator's choice)

All three outputs carry the same trace_id → correlation works across signals.
```

## ObservabilityContext — parallel surface to ScopeLevel

```rust
// nebula-log/src/context.rs (or nebula-core if widely depended on)

use opentelemetry::trace::{TraceId, SpanId};
use nebula_core::{OrgId, WorkspaceId, WorkflowId, WorkflowVersionId, ExecutionId, NodeId, AttemptId, InstanceId, UserId, ServiceAccountId};

/// Parallel telemetry surface. Reuses underlying IDs from nebula-core but
/// is NOT derived from ScopeLevel — they serve different purposes.
/// ScopeLevel manages resource lifecycles; ObservabilityContext enriches
/// tracing/logging/metrics with consistent attributes.
#[derive(Debug, Clone, Default)]
pub struct ObservabilityContext {
    // OpenTelemetry core identifiers
    pub trace_id: Option<TraceId>,
    pub parent_span_id: Option<SpanId>,

    // Infrastructure identity (same for all spans from this process)
    pub instance_id: Option<InstanceId>,  // nbl_01J9...

    // Tenant identifiers (from request path or execution record)
    pub org_id: Option<OrgId>,
    pub workspace_id: Option<WorkspaceId>,

    // Execution identifiers
    pub workflow_id: Option<WorkflowId>,
    pub workflow_version_id: Option<WorkflowVersionId>,
    pub execution_id: Option<ExecutionId>,
    pub logical_node_id: Option<NodeId>,          // stable name in workflow graph
    pub attempt_id: Option<AttemptId>,             // att_01J9... row in execution_nodes
    pub attempt_number: Option<u32>,

    // Principal that triggered / owns work
    pub user_id: Option<UserId>,
    pub service_account_id: Option<ServiceAccountId>,

    // Request identifier (set at API ingress, persists through async work)
    pub request_id: Option<String>,
}

impl ObservabilityContext {
    /// Fresh root context — e.g., at API handler entry.
    pub fn root() -> Self {
        let trace_id = opentelemetry::trace::TraceId::from(rand::random::<u128>());
        Self {
            trace_id: Some(trace_id),
            instance_id: Some(nebula_core::instance_id()),
            ..Default::default()
        }
    }

    /// Derive child context inheriting parent fields.
    /// Typically used when crossing layer boundaries (handler → engine, engine → runtime).
    pub fn child(&self) -> Self {
        self.clone()
    }

    /// Install as tracing span. All tracing::* inside will inherit attributes.
    pub fn span(&self, name: &'static str) -> tracing::Span {
        tracing::info_span!(
            target: "nebula",
            "nebula.operation" = name,
            "nebula.instance_id" = field::display_option(&self.instance_id),
            "nebula.org_id" = field::display_option(&self.org_id),
            "nebula.workspace_id" = field::display_option(&self.workspace_id),
            "nebula.workflow_id" = field::display_option(&self.workflow_id),
            "nebula.workflow_version_id" = field::display_option(&self.workflow_version_id),
            "nebula.execution_id" = field::display_option(&self.execution_id),
            "nebula.logical_node_id" = field::display_option(&self.logical_node_id),
            "nebula.attempt_id" = field::display_option(&self.attempt_id),
            "nebula.attempt" = self.attempt_number,
            "nebula.user_id" = field::display_option(&self.user_id),
            "nebula.service_account_id" = field::display_option(&self.service_account_id),
            "nebula.request_id" = self.request_id.as_deref(),
        )
    }
}
```

### Relationship to `ScopeLevel`

**Not a replacement, not derived automatically.** At layer boundaries, both are constructed from the same request / execution data but they stay separate:

```rust
// At engine entry
let exec_row = storage.load_execution(exec_id).await?;

// Resource lifecycle
let scope = ScopeLevel::Execution(exec_row.id);
let db_pool = resource_manager.acquire_for(scope);

// Telemetry
let obs_ctx = ObservabilityContext {
    trace_id: exec_row.trace_id,      // persisted from ingress
    org_id: Some(exec_row.org_id),
    workspace_id: Some(exec_row.workspace_id),
    workflow_id: Some(extract_workflow_id(&exec_row)),
    execution_id: Some(exec_row.id),
    ..Default::default()
};
let _guard = obs_ctx.span("execute_workflow").entered();

// Both use same IDs, neither is the authority for the other
```

**Rule:** if you're managing a resource's lifetime, use `ScopeLevel`. If you're emitting telemetry, use `ObservabilityContext`. They don't convert into each other; they happen to share IDs.

## Trace propagation

### At ingress — generate or extract

```rust
// nebula-api middleware
pub async fn tracing_middleware(mut req: Request, next: Next) -> Result<Response> {
    // 1. Extract or generate trace_id
    let trace_id = req.headers()
        .get("traceparent")
        .and_then(|v| parse_traceparent(v.to_str().ok()?))
        .unwrap_or_else(|| TraceId::from(rand::random::<u128>()));

    // 2. Generate request_id (separate from trace — request_id is for log grep,
    //    trace_id is for distributed tracing)
    let request_id = Ulid::new().to_string();

    // 3. Build root context
    let obs_ctx = ObservabilityContext {
        trace_id: Some(trace_id),
        instance_id: Some(nebula_core::instance_id()),
        request_id: Some(request_id.clone()),
        ..Default::default()
    };

    // 4. Install as task-local, enter span
    let span = obs_ctx.span("http_request");
    req.extensions_mut().insert(obs_ctx);

    // 5. Run rest of middleware chain inside span
    let response = async move {
        let mut resp = next.run(req).await;
        resp.headers_mut().insert("X-Request-ID", request_id.parse().unwrap());
        resp
    }.instrument(span).await;

    Ok(response)
}
```

### Persistence — `executions.trace_id`

When execution is created (trigger claim, manual start, scheduled fire), the **current `trace_id`** is persisted on the execution row:

```sql
ALTER TABLE executions
    ADD COLUMN trace_id BYTEA;  -- 16 bytes OpenTelemetry trace id

CREATE INDEX idx_executions_trace ON executions(trace_id);
```

**Why persist:** async executions run long after the initial request that triggered them. A webhook arrives at `10:00`, trigger_events row inserted, worker picks up at `10:02`, execution runs for 3 hours. All those later spans must join the original `trace_id` so logs/metrics/traces from any stage correlate.

### Across `nebula-eventbus`

Events carry their observability context:

```rust
#[derive(Debug, Clone)]
pub struct TracedEvent<E> {
    pub ctx: ObservabilityContext,
    pub payload: E,
    pub emitted_at: DateTime<Utc>,
}

impl<E: Clone + Send + Sync> EventBus<TracedEvent<E>> {
    pub fn emit(&self, ctx: ObservabilityContext, payload: E) {
        self.publish(TracedEvent { ctx, payload, emitted_at: Utc::now() });
    }
}

// Subscribers install context when handling
async fn journal_writer(event: TracedEvent<ExecutionEvent>) {
    let span = event.ctx.span("journal_write");
    async move {
        storage.append_journal(event.payload).await.ok();
    }.instrument(span).await;
}
```

### Across HTTP — outbound requests

When action makes outbound HTTP call, inject `traceparent`:

```rust
// nebula-action provides helper for action authors
impl ActionContext {
    pub fn http_client(&self) -> reqwest::Client {
        // Pre-configured client that automatically injects traceparent
        self.http_client_factory.build_with_trace(&self.obs_ctx)
    }
}
```

The receiving service (if OTel-aware) continues the trace under the same `trace_id`. Distributed trace spans our process plus any downstream service that speaks OTel.

### Cross-process takeover

When worker B takes over an execution that worker A crashed with (spec 17 stale lease), B loads `executions.trace_id` and continues the trace under the same root. The whole run is one logical trace across workers.

## Log format

### Mandatory fields (structured JSON to stdout)

Every log line includes:

```json
{
  "timestamp": "2026-04-15T10:23:45.123Z",
  "level": "INFO",
  "target": "nebula_engine::executor",
  "message": "node completed",
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
  "span_id": "00f067aa0ba902b7",
  "service.name": "nebula",
  "service.version": "0.1.0",
  "nebula.instance_id": "nbl_01J9X...",
  "nebula.org_id": "org_01J9X...",
  "nebula.workspace_id": "ws_01J9X...",
  "nebula.execution_id": "exec_01J9X...",
  "nebula.logical_node_id": "charge_customer",
  "nebula.attempt_id": "att_01J9X...",
  "nebula.attempt": 2,
  "nebula.request_id": "01J9XYZ...",

  "node.status": "succeeded",
  "node.duration_ms": 234
}
```

### What NOT to log

- **Credential values** — ever. Not in plain, not in "redacted", not as length hint. `SecretString`'s `Debug` impl returns `***REDACTED***` (see canon §12.5).
- **User PII in bulk** — emails, names, addresses. If relevant, use `user_id` reference only.
- **Action inputs/outputs in full** — these may contain anything. Log a hash or size, not contents.
- **Stack traces with secrets** — if a secret ended up in an error message chain, redact before logging.

### Span hierarchy (reflects request/execution flow)

```
http_request (root)
  ├─ auth_middleware
  ├─ tenancy_middleware
  ├─ rbac_middleware
  └─ handler.start_execution
      └─ storage.insert_execution
(separate root when worker picks up — new span, same trace_id via executions.trace_id)
engine.run_execution
  ├─ engine.load_plan
  ├─ engine.execute_level (level 0)
  │   ├─ runtime.run_node (charge_customer)
  │   │   ├─ action.execute
  │   │   │   ├─ resilience.retry_attempt (1)
  │   │   │   │   └─ http.post (to stripe.com)
  │   │   │   └─ resilience.retry_attempt (2)
  │   │   └─ runtime.persist_output
  │   └─ runtime.run_node (send_email)
  └─ engine.execute_level (level 1)
```

Any log line inside a span gets all its attributes. Grep by `trace_id` in Loki/ClickHouse/whatever shows the whole story.

## Metrics

### Naming convention — `nebula_*` prefix

Metric names follow OpenTelemetry semantic conventions where possible, with `nebula_` prefix to avoid collisions. Listed in `nebula-metrics::naming`.

### Execution metrics

```
nebula_executions_started_total{trigger_kind, plan}
nebula_executions_succeeded_total{plan}
nebula_executions_failed_total{error_kind, plan}
nebula_executions_cancelled_total{plan}
nebula_executions_orphaned_total
nebula_executions_active{plan}               # gauge
nebula_execution_duration_seconds{status, plan}  # histogram
```

### Node / action metrics

```
nebula_action_started_total{action_kind}
nebula_action_succeeded_total{action_kind}
nebula_action_failed_total{action_kind, error_kind}
nebula_action_duration_seconds{action_kind, status}  # histogram
nebula_action_retry_total{action_kind}
nebula_action_timeout_total{action_kind}
```

### Trigger metrics

```
nebula_trigger_events_received_total{trigger_kind}
nebula_trigger_events_deduplicated_total{trigger_kind}
nebula_trigger_events_dispatched_total{trigger_kind}
nebula_trigger_events_rejected_total{trigger_kind, reason}
nebula_trigger_events_inbox_depth{workspace_id}   # gauge — careful with cardinality
nebula_trigger_webhook_auth_failures_total{trigger_kind, auth_kind}
```

### Retry metrics

```
nebula_retry_scheduled_total{action_kind}
nebula_retry_budget_exhausted_total{action_kind, reason="attempts"|"time"}
```

### Quota / rate limit metrics

```
nebula_quota_exceeded_total{quota_kind, plan}     # quota_kind: concurrent/monthly/storage/...
nebula_rate_limited_total{limit_kind}
nebula_quota_usage_ratio{quota_kind, plan}         # gauge — current / limit
nebula_quota_drift_corrected_total                  # reconciliation job
```

### Storage / coordination metrics

```
nebula_claim_query_duration_seconds             # histogram
nebula_claim_query_total{result}                 # result: claimed/empty/error
nebula_lease_renewal_total{result}
nebula_lease_lost_total
nebula_takeover_total{reason}
nebula_storage_query_duration_seconds{query}    # histogram, query label is low cardinality
```

### Auth / identity metrics

```
nebula_auth_login_attempts_total{result}        # result: success/failure_password/failure_locked/failure_mfa
nebula_auth_login_failed_total{reason}
nebula_auth_pat_used_total
nebula_auth_session_created_total
nebula_auth_mfa_used_total
```

### Checkpoint / stateful metrics

```
nebula_stateful_buffer_bytes                    # gauge
nebula_stateful_buffer_entries                  # gauge
nebula_stateful_flush_total{trigger}
nebula_stateful_flush_duration_seconds
nebula_stateful_force_flush_total{reason}       # memory/count pressure
```

### Cardinality protection

Labels on metrics are **strictly allowlisted** per `nebula-metrics::LabelAllowlist`:

```rust
pub static ALLOWED_LABELS: phf::Set<&'static str> = phf::phf_set! {
    "status", "error_kind", "action_kind", "trigger_kind", "plan",
    "quota_kind", "limit_kind", "auth_kind", "result", "reason",
    "query", "trigger", "workspace_id",  // workspace_id only for small deployments
};
```

**Forbidden as labels:**

- `execution_id`, `attempt_id`, `user_id`, `trace_id` — unbounded cardinality
- `error_message` — unbounded text
- `url`, `path` — unbounded
- Any ULID

**Per-workspace labels are dangerous at scale:**

- OK for self-host (tens of workspaces)
- Cloud free tier: aggregate only (don't add `workspace_id`)
- Cloud paid tier: add `workspace_id` because there are fewer paying customers

This is **config-driven**: env var `NEBULA_METRICS_PER_WORKSPACE=true|false` toggles. Default: `false` for cloud, `true` for self-host.

### Histogram buckets

Default duration histogram buckets (seconds):

```
0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10, 30, 60, 300
```

Covers microseconds (fast in-memory ops) to minutes (long actions). Override per-metric when spread is different.

## Event bus fan-out — four subscribers

Canon §12.2 already names the event bus as the hub. This spec specifies the **four standard subscribers** that every Nebula process runs:

### Subscriber 1 — Storage batch writer

Writes `execution_nodes` updates and `execution_journal` entries. Batched via write-behind (spec 14).

**Durability:** high — failure loses work since last flush. Uses CAS + lease check (spec 17) for split-brain safety.

### Subscriber 2 — Metrics collector

Increments counters, records histograms for every ExecutionEvent variant. In-process only. Fast path, no allocation.

**Durability:** ephemeral. Process crash loses local metric state; Prometheus scrape captures pre-crash values from last scrape. Not authoritative.

### Subscriber 3 — Websocket broadcaster

Fans out events to live UI clients. Each client subscribes to rooms like `execution:exec_01J9...` or `workspace:ws_01J9...`.

**Durability:** ephemeral. Dropped messages are acceptable for UI (client gets next state from next event or from explicit refresh).

**Backpressure:** slow client → drop events for that client only, not block others. Metric `nebula_websocket_dropped_total{reason="slow_client"}` surfaces this.

### Subscriber 4 — Audit writer

Writes security-relevant events to `audit_log` table. Not every event — only those that matter for compliance:

- User login / logout / failed login
- PAT creation / use / revocation
- Workflow published / reverted / deleted
- Credential created / rotated / deleted
- Org member added / removed / role changed
- Service account created / disabled
- Admin actions (terminate execution, force takeover)

**Durability:** high — compliance requirement. Audit writes use same atomic-transaction pattern as state transitions.

## Real-time UI protocol (websocket)

### Endpoint

```
GET wss://nebula.io/api/v1/orgs/{org}/workspaces/{ws}/live
Upgrade: websocket
Authorization: Bearer sess_... OR Cookie: nebula_session=...
```

Same auth as REST. Permission: `WorkspaceViewer` minimum (for reading events).

### Client subscription messages

After connect, client sends subscription intent:

```json
{
  "type": "subscribe",
  "rooms": [
    "execution:exec_01J9XYZ",
    "workspace:ws_01J9XYZ/active_executions",
    "workflow:wf_01J9XYZ/recent"
  ]
}
```

Server validates each room against RBAC. Rooms not permitted are silently ignored (logged as `nebula_websocket_room_denied_total`).

### Server push messages

```json
{
  "type": "event",
  "room": "execution:exec_01J9XYZ",
  "event": "node_started",
  "payload": {
    "trace_id": "4bf92f...",
    "execution_id": "exec_01J9XYZ",
    "logical_node_id": "charge_customer",
    "attempt_id": "att_01J9XYZ",
    "attempt": 1,
    "started_at": "2026-04-15T10:23:45.123Z"
  }
}
```

Event types (non-exhaustive):

- `execution_started`, `execution_completed`, `execution_failed`, `execution_cancelled`
- `node_started`, `node_succeeded`, `node_failed`, `node_retrying`, `node_waiting`
- `stateful_progress` (for StatefulAction `emit_progress` updates)
- `log` (selected log entries, for live log tailing view — subject to quota)

### Multi-process fan-out

**Single-process (v1):** in-process `nebula-eventbus` → websocket broadcaster. Trivial.

**Multi-process (v2):** worker A emits event, client connected to worker B. Two options:

**Option 1 — Postgres LISTEN/NOTIFY**
```rust
// Worker A publishes
storage.pg_notify("nebula_events", &event_payload).await?;

// Worker B subscribes, forwards to local websocket broadcaster
pg_listener.listen("nebula_events").await?;
while let Some(notification) = pg_listener.next().await {
    let event: TracedEvent<...> = serde_json::from_str(&notification.payload())?;
    local_websocket_broadcaster.publish(event);
}
```

Zero dependencies beyond Postgres. Good for modest scale.

**Option 2 — Redis Pub/Sub (v3+)**

Add Redis as optional backend for higher throughput. Opt-in via config.

**Recommended v1:** single-process. **Recommended v2:** Postgres LISTEN/NOTIFY. **Recommended v3 (cloud scale):** Redis.

### Sticky routing alternative

Load balancer pins client to one worker for session duration. Client only sees events from that worker. Fallback for when fan-out isn't implemented yet. Limitation: if worker A handles execution X, client connected to worker B won't see X events. Bad UX. **Don't use as primary.**

## Audit log vs execution journal — two tables, two purposes

### `execution_journal` — per-execution timeline

Append-only log of everything that happened during one execution:

- `ExecutionStarted`, `NodeStarted`, `NodeSucceeded`, `NodeFailed`
- `RetryScheduled`, `AttemptStarted`
- `CheckpointFlushed`, `StateRestored`
- `Cancelled`, `CancelledEscalated`

**Audience:** operator debugging one specific run. UI shows this as timeline in execution detail view (prototype in Q17).

**Retention:** cascades with execution (spec 16).

**Storage:** high-frequency writes, small per-event. Partitioning by month if scale requires.

### `audit_log` — tenant-wide security events

Append-only log of security and lifecycle events across tenants:

- Identity: `user.logged_in`, `user.mfa_enrolled`, `pat.created`, `pat.used_first_time`
- Authorization: `member.added`, `member.role_changed`, `member.removed`
- Resource lifecycle: `workflow.published`, `credential.rotated`, `credential.deleted`
- Administrative: `execution.terminated_by_admin`, `org.plan_upgraded`

**Audience:** compliance officer, security review, incident forensics.

**Retention:** 90 days default, plan-configurable. Enterprise plans may require 1 year or indefinite.

**Storage:** lower frequency, structured payload. Indexed by (org_id, emitted_at) for audit queries.

**Why separate:** different retention, different RBAC (audit log requires Admin to read, journal viewable by Runner), different data shape.

## Canon §4.6 fulfilment checklist

Canon §4.6 says run must be explainable without reading Rust source. This stack delivers that if **all** of these are true:

- ✅ Every log line has `trace_id`, `execution_id`, `org_id`, `workspace_id`
- ✅ Same `trace_id` flows through logs, metrics (via exemplars), traces
- ✅ `execution_journal` records every state transition with full context
- ✅ Metrics answer «how often does this fail» without looking at individual runs
- ✅ UI timeline view shows the full execution story from `execution_journal`
- ✅ `audit_log` answers «who did what when»
- ✅ Errors in logs reference `error_code` from `nebula-error` (RFC 9457 — see spec 19)
- ✅ OTel export works so ops team can plug into their existing stack

**Test for §4.6 compliance:** operator opens failed execution, can answer these four questions **in under 2 minutes** without opening a Rust file:

1. What node failed?
2. What error did it return?
3. Was it retried? How many times?
4. What was the preceding context (what nodes ran before)?

If any answer requires grep'ing source, §4.6 is not met.

## Deployment patterns

### Minimal (self-host indie)

```bash
$ nebula serve
```

- Logs: stdout JSON (pipe to file or journald)
- Metrics: `GET /metrics` Prometheus endpoint (scrape manually or via `curl`)
- Traces: none
- Journal: in Postgres (via execution_journal table)
- Audit: in Postgres

Zero external services. Operator reads logs with `jq`, sees metrics with `curl /metrics | grep`, debugs via SQL queries on `execution_journal`.

### Standard (self-host serious)

Docker-compose manifest we ship:

```yaml
services:
  nebula:
    image: nebula:v0.1
    environment:
      NEBULA_OTEL_EXPORTER: "http://otel-collector:4317"
    ports: ["8080:8080"]
  
  otel-collector:
    image: otel/opentelemetry-collector
    # receives OTLP, forwards to Loki/Prometheus/Tempo
  
  loki:
    image: grafana/loki
  
  prometheus:
    image: prom/prometheus
  
  tempo:
    image: grafana/tempo
  
  grafana:
    image: grafana/grafana
    volumes:
      - ./grafana-dashboards:/etc/grafana/provisioning/dashboards
```

We ship **Grafana dashboard JSONs** in `ops/grafana/` for the standard queries (execution rate, latency, retry rate, etc.). Operator runs `docker-compose up`, opens Grafana, sees everything.

### Cloud

Same Nebula binary. Environment configured to export OTLP to our managed observability stack (Grafana Cloud, Honeycomb, or self-hosted Mimir + Tempo + Loki). Per-tenant views in our operator dashboard.

## Configuration surface

```toml
[observability]
# Always-on
stdout_json_logs = true
prometheus_endpoint_enabled = true
prometheus_endpoint_path = "/metrics"

# Optional OTel export
otel_exporter_enabled = false
otel_exporter_endpoint = "http://localhost:4317"
otel_exporter_protocol = "grpc"  # or "http/protobuf"
otel_service_name = "nebula"

# Sampling
trace_sampling_ratio = 1.0  # 1.0 = always trace; lower for high-throughput cloud

# Log level
log_level = "info"  # error / warn / info / debug / trace
log_target_filters = []  # e.g., ["nebula_engine=debug", "sqlx=warn"]

# Metric cardinality
metrics_per_workspace_labels = true  # false for cloud free tier

# Real-time
websocket_enabled = true
websocket_max_clients_per_execution = 50
websocket_event_buffer_per_client = 100
multi_process_fanout = "in_process"  # "in_process" / "postgres_listen_notify" / "redis"

# Audit
audit_retention_days = 90
audit_write_all_events = false  # if true, audit every ExecutionEvent (very verbose)
```

## Testing criteria

**Unit tests:**
- `ObservabilityContext::child()` inherits correctly
- Span attribute injection picks all fields
- Trace ID serialization / parse round-trip
- Metric label allowlist rejects unauthorized labels at compile or runtime

**Integration tests:**
- Full request → log → metric → trace chain in test fixture
- `trace_id` from HTTP header persists into execution row
- `trace_id` from execution row propagates to worker after takeover
- Event bus fan-out delivers to all four subscribers
- Websocket subscription receives events for subscribed room, not for others
- Websocket unsubscribe stops delivery
- Permission-denied room silently dropped

**End-to-end tests:**
- Start execution, kill worker mid-execution, trace covers both workers (same trace_id)
- Audit log captures login / workflow publish / credential rotation events
- Stress test: 1000 events/sec via eventbus, all subscribers keep up
- Slow websocket client doesn't block fast ones

**Cardinality tests:**
- Attempt to emit metric with forbidden label → rejected at compile or runtime
- Per-workspace metrics disabled → `workspace_id` label stripped
- Prometheus scrape output count is bounded (< N unique series per scrape)

## Performance targets

- Log line emit (JSON stdout): **< 5 µs**
- Span create + attribute set: **< 10 µs**
- Metric counter increment: **< 1 µs**
- Histogram record: **< 5 µs**
- Event bus emit (in-process): **< 10 µs**
- Event bus fan-out (all 4 subscribers): **< 100 µs**
- Websocket push to client: **< 5 ms p99**
- Storage journal write (batched): **< 50 ms p99** (not per event — per batch)
- Prometheus scrape full payload: **< 100 ms** for typical process

## Module boundaries

| Component | Crate |
|---|---|
| `ObservabilityContext`, `span()` helper | `nebula-log` |
| `tracing-subscriber` setup, JSON formatter | `nebula-log::init` |
| OTLP exporter configuration | `nebula-log::otel` (feature-gated) |
| Metric types (Counter/Gauge/Histogram) | `nebula-telemetry` (in-memory primitives) |
| `nebula_*` naming, registry, Prometheus text export | `nebula-metrics` |
| Metric label allowlist | `nebula-metrics::filter` |
| Event bus (transport) | `nebula-eventbus` |
| `ExecutionEvent` domain type | `nebula-engine` |
| Storage batch writer subscriber | `nebula-engine` (or `nebula-runtime`) |
| Metrics collector subscriber | `nebula-engine` |
| Websocket broadcaster subscriber | `nebula-api::websocket` |
| Audit writer subscriber | `nebula-api::audit` |
| `execution_journal` repo | `nebula-storage` |
| `audit_log` repo | `nebula-storage` |
| Postgres LISTEN/NOTIFY client | `nebula-storage::fanout` |

## Dependencies

- `tracing`, `tracing-subscriber` — core logging
- `tracing-opentelemetry` — bridge to OTel
- `opentelemetry`, `opentelemetry-otlp`, `opentelemetry-semantic-conventions` — OTel SDK + exporter
- `prometheus` crate or custom — Prometheus text export
- `axum` built-in websocket support
- `sqlx` with Postgres LISTEN/NOTIFY (v2)

## Migration path

**Greenfield** for OTel integration — nothing to migrate.

**`nebula-log`, `nebula-metrics`, `nebula-telemetry`** all exist already; this spec adds `ObservabilityContext` wrapper + OTel feature flag.

**Existing span usage:** if any crate already has `tracing::info_span!` calls, they will automatically inherit attributes from the outer `ObservabilityContext::span()` — no source changes needed. Over time, update crates to set explicit `obs_ctx.span()` at boundary points.

**Existing metric usage:** `nebula-metrics` registry already exists. This spec codifies naming and allowlist. Audit existing metric emissions against the allowlist, tighten where needed.

## Open questions

- **Log ingestion backend for self-host** — recommend Loki, ClickHouse, or just rotating file? Default: stdout is enough, operators bring their own.
- **Trace sampling strategy** — 100% for self-host (volumes manageable); cloud needs lower sampling for high-throughput tenants. Default: config-driven.
- **Log level per-module** — use `RUST_LOG` env var (`tracing-subscriber` native) or custom config? Default: support both.
- **Exemplars in Prometheus** — link metric samples to trace IDs. Advanced feature, requires Prometheus exemplars support. Deferred.
- **Structured error logging** — when `NebulaError` is logged, auto-include the error code, category, details. See spec 19 (error taxonomy).
- **Distributed tracing across Nebula instances** — when one workflow action calls another Nebula (via HTTP), traceparent propagation should just work. Test explicitly.
- **Retention of audit_log beyond 90 days** — enterprise compliance needs longer. Move to cold storage (S3 archive) after 90 days? Deferred.
- **Sampling at event bus layer** — if eventbus is overwhelmed, can we sample events before fan-out? Tricky because each subscriber has different needs. Deferred.
- **Log PII scrubbing pipeline** — automatic redaction of email-like strings, phone numbers, etc. before emission. Nice defense-in-depth, deferred.
