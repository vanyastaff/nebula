# Nebula Telemetry & Metrics — Design Spec

## Goal

Define the observability stack across three deployment modes (local, self-hosted, SaaS). One instrumentation layer, three export profiles. Zero telemetry leaves the process unless explicitly enabled.

## Philosophy

- **Instrument once, export differently.** Core code records metrics/events into a unified registry. Export adapters decide what goes where.
- **Local = full visibility, zero export.** Desktop user sees everything in the UI. Nothing leaves the process. No phone-home. Ever.
- **Self-hosted = ops-grade.** Prometheus, Grafana, Jaeger — standard stack. Opt-in, not default.
- **SaaS = ops + billing.** Same ops metrics plus per-tenant usage tracking for billing and SLA.
- **Privacy by default.** No PII in metrics. No credential values. No node output content. Only shapes and counts.

---

## 1. Three Export Profiles

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TelemetryProfile {
    /// Desktop app. All metrics in-memory/SQLite. No network export.
    Local,
    /// Self-hosted server. Prometheus + optional OTEL + structured logs.
    SelfHosted {
        prometheus_enabled: bool,
        otel_endpoint: Option<String>,
        log_level: LogLevel,
    },
    /// Managed SaaS. Full ops + billing metrics + tenant attribution.
    Cloud {
        otel_endpoint: String,
        billing_endpoint: String,
        log_level: LogLevel,
    },
}
```

Configuration:
```toml
# nebula.toml
[telemetry]
profile = "local"  # or "self_hosted" or "cloud"

# Self-hosted options:
[telemetry.prometheus]
enabled = true
endpoint = "0.0.0.0:9090"

[telemetry.otel]
endpoint = "http://localhost:4317"

[telemetry.logs]
level = "info"
format = "json"  # or "pretty" for development
```

---

## 2. Metric Categories

### Layer 1 — Core (ALL modes, including local)

These metrics power the UI and basic health monitoring:

```rust
// Execution lifecycle
pub const EXECUTIONS_STARTED: &str = "nebula_executions_started_total";
pub const EXECUTIONS_COMPLETED: &str = "nebula_executions_completed_total";
pub const EXECUTIONS_FAILED: &str = "nebula_executions_failed_total";
pub const EXECUTIONS_CANCELLED: &str = "nebula_executions_cancelled_total";
pub const EXECUTION_DURATION: &str = "nebula_execution_duration_seconds";

// Per-node lifecycle
pub const NODE_STARTED: &str = "nebula_node_started_total";
pub const NODE_COMPLETED: &str = "nebula_node_completed_total";
pub const NODE_FAILED: &str = "nebula_node_failed_total";
pub const NODE_SKIPPED: &str = "nebula_node_skipped_total";
pub const NODE_DURATION: &str = "nebula_node_duration_seconds";

// Error tracking
pub const ERRORS_TOTAL: &str = "nebula_errors_total";  // labeled by category
```

### Layer 2 — Performance (ALL modes)

Deeper metrics for debugging and optimization:

```rust
// Per-action performance
pub const ACTION_DURATION: &str = "nebula_action_duration_seconds";  // labeled by action_key
pub const ACTION_INPUT_BYTES: &str = "nebula_action_input_bytes";
pub const ACTION_OUTPUT_BYTES: &str = "nebula_action_output_bytes";

// Expression evaluation
pub const EXPRESSION_EVAL_COUNT: &str = "nebula_expression_eval_total";
pub const EXPRESSION_EVAL_DURATION: &str = "nebula_expression_eval_duration_seconds";
pub const EXPRESSION_CACHE_HITS: &str = "nebula_expression_cache_hits_total";
pub const EXPRESSION_CACHE_MISSES: &str = "nebula_expression_cache_misses_total";

// Credential operations
pub const CREDENTIAL_RESOLVE_DURATION: &str = "nebula_credential_resolve_duration_seconds";
pub const CREDENTIAL_REFRESH_TOTAL: &str = "nebula_credential_refresh_total";
pub const CREDENTIAL_REFRESH_FAILED: &str = "nebula_credential_refresh_failed_total";

// Resource pool
pub const RESOURCE_ACQUIRE_DURATION: &str = "nebula_resource_acquire_duration_seconds";
pub const RESOURCE_POOL_ACTIVE: &str = "nebula_resource_pool_active";  // gauge
pub const RESOURCE_POOL_IDLE: &str = "nebula_resource_pool_idle";      // gauge

// Storage
pub const CHECKPOINT_WRITE_DURATION: &str = "nebula_checkpoint_write_duration_seconds";
pub const CHECKPOINT_WRITE_BYTES: &str = "nebula_checkpoint_write_bytes";

// Memory
pub const EXECUTION_MEMORY_BYTES: &str = "nebula_execution_memory_bytes";  // gauge per execution
```

### Layer 3 — Ops Export (self-hosted + SaaS only)

Exported to external systems:

```rust
// Queue depth (QueueBackend — RT11)
pub const QUEUE_DEPTH: &str = "nebula_queue_depth";  // gauge
pub const QUEUE_ENQUEUE_DURATION: &str = "nebula_queue_enqueue_duration_seconds";
pub const QUEUE_DEQUEUE_DURATION: &str = "nebula_queue_dequeue_duration_seconds";

// Rate limiting
pub const RATE_LIMIT_HITS: &str = "nebula_rate_limit_hits_total";  // labeled by tenant, provider

// Webhook ingest
pub const WEBHOOK_RECEIVED: &str = "nebula_webhook_received_total";
pub const WEBHOOK_PROCESSED: &str = "nebula_webhook_processed_total";
pub const WEBHOOK_FAILED: &str = "nebula_webhook_failed_total";
pub const WEBHOOK_QUEUE_LAG: &str = "nebula_webhook_queue_lag_seconds";

// API
pub const API_REQUEST_DURATION: &str = "nebula_api_request_duration_seconds";
pub const API_REQUEST_TOTAL: &str = "nebula_api_request_total";  // labeled by method, path, status
```

### Layer 4 — Billing (SaaS only)

Per-tenant usage for billing:

```rust
// Tenant-scoped counters
pub const TENANT_EXECUTIONS: &str = "nebula_tenant_executions_total";
pub const TENANT_NODE_EXECUTIONS: &str = "nebula_tenant_node_executions_total";
pub const TENANT_COMPUTE_SECONDS: &str = "nebula_tenant_compute_seconds_total";
pub const TENANT_STORAGE_BYTES: &str = "nebula_tenant_storage_bytes";  // gauge
pub const TENANT_API_CALLS: &str = "nebula_tenant_api_calls_total";

// Cost attribution (LLM actions)
pub const TENANT_LLM_INPUT_TOKENS: &str = "nebula_tenant_llm_input_tokens_total";
pub const TENANT_LLM_OUTPUT_TOKENS: &str = "nebula_tenant_llm_output_tokens_total";
pub const TENANT_LLM_COST_USD: &str = "nebula_tenant_llm_cost_usd_total";
```

---

## 3. Local Mode — In-Memory Metrics Store

Desktop app stores metrics in-memory with SQLite persistence for history:

```rust
/// In-memory metric store for desktop UI.
/// Not exported. Read directly by UI rendering code.
pub struct LocalMetricsStore {
    /// Live counters and gauges — reset on restart.
    counters: DashMap<MetricKey, AtomicU64>,
    gauges: DashMap<MetricKey, AtomicI64>,
    histograms: DashMap<MetricKey, Histogram>,

    /// Execution history — persisted in SQLite.
    history: SqliteExecutionHistory,
}

pub struct SqliteExecutionHistory {
    /// Last N executions with per-node timing.
    /// Retention: configurable, default 1000 executions or 30 days.
    db: libsql::Connection,
}

impl LocalMetricsStore {
    /// Called by UI to render dashboard.
    pub fn execution_summary(&self, execution_id: &ExecutionId) -> ExecutionSummary {
        ExecutionSummary {
            duration: self.get_duration(execution_id),
            node_timings: self.get_node_timings(execution_id),
            total_input_bytes: self.get_total_input_bytes(execution_id),
            total_output_bytes: self.get_total_output_bytes(execution_id),
            expression_eval_count: self.get_expression_count(execution_id),
            error_count: self.get_error_count(execution_id),
        }
    }

    /// Called by UI to render history sidebar.
    pub fn recent_executions(&self, limit: usize) -> Vec<ExecutionHistoryEntry> {
        self.history.list_recent(limit)
    }

    /// Per-action stats (success/fail rate over time).
    pub fn action_stats(&self, action_key: &ActionKey, days: u32) -> ActionStats {
        self.history.action_stats(action_key, days)
    }

    /// Per-workflow run frequency.
    pub fn workflow_frequency(&self, workflow_id: &WorkflowId, days: u32) -> Vec<(Date, u32)> {
        self.history.workflow_runs_per_day(workflow_id, days)
    }
}
```

### What the Desktop UI shows

```
┌─────────────────────────────────────────────────────┐
│ Workflow: "Sync Orders"                    [Run ▶]  │
│ Last run: 3.2s ago — ✅ Completed in 1.4s          │
│ Runs today: 12 | Success rate: 91.7%               │
├─────────────────────────────────────────────────────┤
│ Node Timeline:                                      │
│ ├── Webhook Trigger    0ms   ████                   │
│ ├── Fetch Orders      450ms  ████████████           │
│ ├── Transform         12ms   █                      │
│ ├── Update CRM        890ms  ██████████████████████ │
│ └── Send Slack        48ms   ██                     │
│                                                     │
│ Data: In 2.3KB → Out 1.1KB                         │
│ Expressions: 8 evals, avg 0.05ms                   │
│ Memory: 4.2MB peak                                 │
├─────────────────────────────────────────────────────┤
│ History                                             │
│ 12:04 ✅ 1.4s  │ 12:03 ✅ 1.2s  │ 12:02 ❌ 0.8s  │
│ 12:01 ✅ 1.5s  │ 12:00 ✅ 1.3s  │ ...             │
└─────────────────────────────────────────────────────┘
```

All powered by `LocalMetricsStore` — no external dependencies.

---

## 4. Self-Hosted Mode — Prometheus + OTEL

### Prometheus Export

Existing `GET /metrics` endpoint serves Prometheus text format:

```
# HELP nebula_executions_started_total Total workflow executions started
# TYPE nebula_executions_started_total counter
nebula_executions_started_total{workflow_id="wf_abc"} 142

# HELP nebula_node_duration_seconds Node execution duration
# TYPE nebula_node_duration_seconds histogram
nebula_node_duration_seconds_bucket{action_key="http.request",le="0.1"} 89
nebula_node_duration_seconds_bucket{action_key="http.request",le="0.5"} 120
nebula_node_duration_seconds_bucket{action_key="http.request",le="1.0"} 135
nebula_node_duration_seconds_sum{action_key="http.request"} 45.6
nebula_node_duration_seconds_count{action_key="http.request"} 142
```

### OpenTelemetry Traces

When OTEL enabled, engine emits spans:

```
Trace: execution_abc
├── Span: workflow.execute (workflow_id=wf_abc, owner_id=tenant_1)
│   ├── Span: node.execute (node_id=nd_1, action_key=http.request)
│   │   ├── Span: credential.resolve (credential_key=bearer_secret, duration_ms=12)
│   │   ├── Span: action.execute (duration_ms=450)
│   │   └── Span: checkpoint.write (bytes=1024, duration_ms=3)
│   ├── Span: node.execute (node_id=nd_2, action_key=transform)
│   │   └── Span: expression.eval (count=8, duration_ms=0.4)
│   └── Span: node.execute (node_id=nd_3, action_key=slack.send)
│       ├── Span: credential.resolve (credential_key=slack_oauth, duration_ms=5)
│       └── Span: action.execute (duration_ms=48)
```

Correlation: all spans carry `execution_id` as trace context (RT4 from Round 3, Datadog feedback).

### Structured Logging

```json
{
  "timestamp": "2026-04-06T12:04:01.234Z",
  "level": "info",
  "target": "nebula_engine::execute",
  "message": "Node completed",
  "execution_id": "ex_abc123",
  "node_id": "nd_456",
  "action_key": "http.request",
  "duration_ms": 450,
  "output_bytes": 1024,
  "status": "completed"
}
```

No PII, no credential values, no node output content in logs. Only metadata.

---

## 5. SaaS Mode — Billing Metrics

Everything from self-hosted PLUS per-tenant usage:

```rust
/// Billing metrics collector — SaaS only.
pub struct BillingCollector {
    /// Per-tenant counters.
    tenant_usage: DashMap<OwnerId, TenantUsage>,
}

pub struct TenantUsage {
    pub executions: AtomicU64,
    pub node_executions: AtomicU64,
    pub compute_seconds: AtomicU64,   // in milliseconds, divide by 1000
    pub storage_bytes: AtomicU64,
    pub api_calls: AtomicU64,
    pub llm_input_tokens: AtomicU64,
    pub llm_output_tokens: AtomicU64,
}

impl BillingCollector {
    /// Called by engine after each node execution.
    pub fn record_node_execution(
        &self,
        owner_id: &OwnerId,
        duration: Duration,
        output_bytes: u64,
    );

    /// Called by API on each request.
    pub fn record_api_call(&self, owner_id: &OwnerId);

    /// Called by AgentAction on LLM usage.
    pub fn record_llm_usage(
        &self,
        owner_id: &OwnerId,
        input_tokens: u64,
        output_tokens: u64,
    );

    /// Export snapshot for billing system.
    pub fn snapshot(&self, owner_id: &OwnerId) -> TenantUsageSnapshot;

    /// Reset after billing cycle.
    pub fn reset(&self, owner_id: &OwnerId);
}
```

---

## 6. USDT Probes (from breakthrough #7 + RT17)

Six stable probe points — zero overhead when disabled:

```rust
dtrace_provider!("nebula", {
    fn action__entry(execution_id: &str, node_id: &str, action_key: &str) {}
    fn action__return(execution_id: &str, node_id: &str, latency_us: u64, ok: u8) {}
    fn checkpoint__write(execution_id: &str, node_id: &str, bytes: u64) {}
    fn credential__resolve(credential_key: &str, scheme: &str, latency_us: u64) {}
    fn blob__write(execution_id: &str, node_id: &str, bytes: u64) {}
    fn resource__acquire(resource_key: &str, topology: &str, latency_us: u64) {}
});
```

Available in ALL modes. Enabled by attaching bpftrace/DTrace at runtime.

---

## 7. Cardinality Protection (RT5, Datadog feedback)

```rust
/// Label cardinality limits prevent metric explosion.
pub struct CardinalityConfig {
    /// Max unique label values per metric (default 10,000).
    pub max_label_cardinality: usize,
    /// Labels sourced from user input are hashed if they exceed cardinality.
    pub hash_overflow_labels: bool,
}
```

Labels bounded by registry size:
- `action_key` — bounded by ActionRegistry size (~100s)
- `workflow_id` — bounded by WorkflowRepo (~1000s)
- `owner_id` — bounded by tenant count (~10,000s for SaaS)
- User-supplied labels — rejected or hashed

---

## 8. Integration Points

| Component | What it records | How |
|-----------|----------------|-----|
| **Engine** | execution_*, node_* counters + durations | `MetricsRegistry` methods |
| **Runtime** | action_* performance, checkpoint writes | `MetricsRegistry` + USDT probes |
| **Credential** | resolve duration, refresh events | `MetricsRegistry` + USDT probes |
| **Resource** | pool gauges, acquire latency, rotation events | `MetricsRegistry` + USDT probes |
| **Expression** | eval count, cache hit/miss, duration | `MetricsRegistry` |
| **API** | request count, duration, status codes | Axum middleware |
| **Webhook** | received, processed, failed, queue lag | `MetricsRegistry` |
| **Billing** (SaaS) | per-tenant usage counters | `BillingCollector` |

---

## 9. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| Telemetry crate | Pure metrics primitives (Counter, Gauge, Histogram) | + TelemetryProfile, + LocalMetricsStore |
| Export | Prometheus text only | + OTEL traces, + billing export |
| Local mode | No metrics UI support | LocalMetricsStore + SQLite history |
| Traces | Removed (stripped 2026-04-04) | Reintroduce as opt-in OTEL spans |
| Correlation | execution_id only | execution_id in all spans + logs (RT4) |
| Cardinality | Unlimited | 10K limit per metric (RT5) |
| USDT probes | None | 6 stable probes (RT17) |
| Billing | None | BillingCollector (SaaS only) |
| Privacy | Not documented | Zero outbound in local mode. No PII in any mode. |

---

## 10. Storage Backend Recommendations

### Why NOT Elasticsearch
ELK is overkill for workflow execution logs: JVM Heap hunger (8-32GB RAM), complex scaling, expensive full-text indexing when we mostly query by structured fields (execution_id, node_id, status, timestamp).

### Recommended Stack

| Data type | Storage | Why | Deployment |
|-----------|---------|-----|------------|
| **Metrics** (counters, gauges, histograms) | **Prometheus** or **VictoriaMetrics** | Already supported via `PrometheusExporter`. VM is drop-in compatible, 10x less RAM | Self-hosted: VM single binary. SaaS: VM cluster |
| **Execution history** (node I/O, timings, large payloads) | **ClickHouse** | Columnar, 10-100x compression vs Elastic. Perfect for analytics queries. Low CPU/RAM | Self-hosted: single node. SaaS: distributed |
| **Logs** (structured text, errors, debug) | **Grafana Loki** | Only indexes labels (not full text), 10x lighter than Elastic. Pairs with Grafana | Self-hosted: single binary. SaaS: Loki cluster |
| **Traces** (OTEL spans) | **Jaeger** or **Grafana Tempo** | Already supported via OTEL exporter. Tempo = no external DB needed | Self-hosted: Jaeger all-in-one. SaaS: Tempo |
| **Local desktop** | **SQLite/libSQL** | Embedded, no external deps, offline-first | In-process |

### ClickHouse for Execution History

Node inputs/outputs are the heaviest data. ClickHouse handles this efficiently:

```sql
CREATE TABLE nebula_node_outputs (
    execution_id UUID,
    node_id UUID,
    action_key LowCardinality(String),
    workflow_id UUID,
    owner_id LowCardinality(String),
    status Enum8('completed' = 1, 'failed' = 2, 'skipped' = 3),
    started_at DateTime64(3),
    completed_at DateTime64(3),
    duration_ms UInt32,
    input_bytes UInt32,
    output_bytes UInt32,
    output_json String CODEC(ZSTD(3)),  -- compressed JSON
    error_message Nullable(String),
    INDEX idx_status status TYPE set(10) GRANULARITY 4,
    INDEX idx_action action_key TYPE set(100) GRANULARITY 4
) ENGINE = MergeTree()
PARTITION BY toYYYYMM(started_at)
ORDER BY (owner_id, workflow_id, execution_id, started_at)
TTL started_at + INTERVAL 90 DAY;
```

Queries that become fast:
```sql
-- How many times did this workflow fail last month?
SELECT count() FROM nebula_node_outputs
WHERE workflow_id = ? AND status = 'failed'
AND started_at > now() - INTERVAL 30 DAY;

-- Slowest nodes across all workflows
SELECT action_key, avg(duration_ms), p99(duration_ms)
FROM nebula_node_outputs
WHERE owner_id = ?
GROUP BY action_key
ORDER BY avg(duration_ms) DESC
LIMIT 20;

-- Full execution replay (all node I/O for one execution)
SELECT node_id, action_key, status, duration_ms, output_json
FROM nebula_node_outputs
WHERE execution_id = ?
ORDER BY started_at;
```

Compression: 10KB JSON per node → ~500 bytes in ClickHouse (ZSTD). 1M node outputs = ~500MB storage vs ~10GB in Elasticsearch.

### VictoriaMetrics vs Prometheus

Both use the same PromQL query language and remote_write protocol. Nebula's existing `PrometheusExporter` works with both.

| Feature | Prometheus | VictoriaMetrics |
|---------|-----------|-----------------|
| RAM per 1M series | ~3GB | ~1GB |
| Disk per 1M series/day | ~2GB | ~0.5GB |
| Long-term retention | Needs Thanos/Cortex | Built-in, unlimited |
| HA | External (Thanos) | Built-in clustering |
| PromQL compatible | Native | 100% compatible |
| Deployment | Single binary | Single binary |

Recommendation: **VictoriaMetrics for self-hosted** (simpler HA, less RAM). Prometheus for dev/small deployments.

### Grafana Loki for Logs

Nebula's structured logs (JSON format) are perfect for Loki:
- Labels: `{app="nebula", level="error", execution_id="ex_abc"}` — indexed
- Log content: full JSON line — NOT indexed, only stored
- Query: `{app="nebula"} |= "credential_resolve" | json | duration_ms > 100`

10x less storage than Elasticsearch because only labels are indexed.

### Integration Architecture

```
Self-hosted deployment:

  Nebula Engine
    │
    ├── /metrics (Prometheus scrape) ──→ VictoriaMetrics ──→ Grafana
    │
    ├── OTEL traces ──→ Jaeger/Tempo ──→ Grafana
    │
    ├── Structured logs (stdout JSON) ──→ Promtail ──→ Loki ──→ Grafana
    │
    └── Node I/O (async writer) ──→ ClickHouse ──→ Grafana (CH plugin)
                                                 ──→ API (execution replay)
```

All four data types visible in ONE Grafana dashboard.

### ExecutionHistoryWriter — Async ClickHouse Integration

```rust
/// Async writer that batches node outputs to ClickHouse.
/// Engine calls record_node_output() after each node.
/// Writer flushes batch to ClickHouse every N records or M seconds.
pub struct ExecutionHistoryWriter {
    buffer: Mutex<Vec<NodeOutputRecord>>,
    batch_size: usize,         // default 100
    flush_interval: Duration,  // default 5s
    client: clickhouse::Client,
}

impl ExecutionHistoryWriter {
    pub fn record_node_output(&self, record: NodeOutputRecord) {
        let mut buf = self.buffer.lock();
        buf.push(record);
        if buf.len() >= self.batch_size {
            self.flush_batch(buf);
        }
    }

    async fn flush_batch(&self, records: Vec<NodeOutputRecord>) {
        let mut insert = self.client.insert("nebula_node_outputs").unwrap();
        for record in records {
            insert.write(&record).await.unwrap();
        }
        insert.end().await.unwrap();
    }
}
```

Feature-gated: `clickhouse-history` feature. Not required for local or basic self-hosted.

---

## 11. Not In Scope

- Custom dashboards (Grafana dashboards are user-configured)
- Log aggregation (ELK/Loki — external tooling)
- APM integration (Datadog/New Relic agents — user installs)
- Alerting rules (Grafana Alerting or PagerDuty — external)
- eBPF metrics backend (v2 — breakthrough idea, Linux-only)
- Analytics/BI on execution data (data warehouse concern)
