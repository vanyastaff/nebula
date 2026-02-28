# Archived From "docs/archive/monitoring-alerting.md"

# Monitoring and Alerting

## Overview

Nebula implements comprehensive monitoring and alerting to ensure system reliability, performance optimization, and rapid incident response. This document covers metrics collection, alerting strategies, and operational dashboards.

## Architecture

### Monitoring Stack

```
┌─────────────────────────────────────────────────────────────┐
│                        Visualization Layer                    │
├──────────────────────┬────────────────────┬─────────────────┤
│      Grafana         │     Kibana         │    Jaeger UI    │
└──────────────────────┴────────────────────┴─────────────────┘
                │                  │                  │
┌───────────────┴──────────────────┴──────────────────┴────────┐
│                        Storage Layer                          │
├──────────────────────┬────────────────────┬─────────────────┤
│     Prometheus       │   Elasticsearch    │    Jaeger       │
└──────────────────────┴────────────────────┴─────────────────┘
                │                  │                  │
┌───────────────┴──────────────────┴──────────────────┴────────┐
│                      Collection Layer                         │
├──────────────────────┬────────────────────┬─────────────────┤
│  Metrics Exporters   │    Log Shippers    │  Trace Agents   │
└──────────────────────┴────────────────────┴─────────────────┘
                │                  │                  │
┌───────────────┴──────────────────┴──────────────────┴────────┐
│                      Application Layer                        │
├──────────────────────┬────────────────────┬─────────────────┤
│    nebula-engine     │   nebula-worker    │   nebula-api    │
└──────────────────────┴────────────────────┴─────────────────┘
```

### Components

#### Metrics Collection (Prometheus)
- Time-series metrics storage
- Pull-based collection model
- Service discovery
- Alert rule evaluation
- Long-term storage with Thanos

#### Logging (ELK Stack)
- Centralized log aggregation
- Full-text search capabilities
- Log parsing and enrichment
- Retention policies
- Correlation with traces

#### Distributed Tracing (Jaeger)
- Request flow visualization
- Latency analysis
- Dependency mapping
- Error tracking
- Performance bottleneck identification

## Metrics

### System Metrics

#### Host Metrics
```yaml
# CPU Metrics
- node_cpu_seconds_total
- node_load1, node_load5, node_load15
- container_cpu_usage_seconds_total

# Memory Metrics
- node_memory_MemTotal_bytes
- node_memory_MemAvailable_bytes
- container_memory_usage_bytes
- container_memory_cache

# Disk Metrics
- node_filesystem_size_bytes
- node_filesystem_avail_bytes
- node_disk_io_time_seconds_total
- container_fs_usage_bytes

# Network Metrics
- node_network_receive_bytes_total
- node_network_transmit_bytes_total
- node_network_receive_errs_total
- container_network_receive_bytes_total
```

#### Application Metrics

```rust
// Workflow Metrics
static WORKFLOW_EXECUTIONS: Lazy<CounterVec> = Lazy::new(|| {
    register_counter_vec!(
        "nebula_workflow_executions_total",
        "Total number of workflow executions",
        &["workflow_id", "status", "trigger_type"]
    ).unwrap()
});

static WORKFLOW_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "nebula_workflow_duration_seconds",
        "Workflow execution duration",
        &["workflow_id"],
        exponential_buckets(0.1, 2.0, 10).unwrap()
    ).unwrap()
});

// Node Metrics
static NODE_EXECUTIONS: Lazy<CounterVec> = Lazy::new(|| {
    register_counter_vec!(
        "nebula_node_executions_total",
        "Total number of node executions",
        &["node_type", "status"]
    ).unwrap()
});

static NODE_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "nebula_node_duration_seconds",
        "Node execution duration",
        &["node_type"],
        exponential_buckets(0.01, 2.0, 10).unwrap()
    ).unwrap()
});

// Worker Metrics
static ACTIVE_WORKERS: Lazy<Gauge> = Lazy::new(|| {
    register_gauge!(
        "nebula_active_workers",
        "Number of active workers"
    ).unwrap()
});

static WORKER_TASKS_COMPLETED: Lazy<CounterVec> = Lazy::new(|| {
    register_counter_vec!(
        "nebula_worker_tasks_completed_total",
        "Total tasks completed by workers",
        &["worker_id", "result"]
    ).unwrap()
});
```

### Custom Metrics Implementation

```rust
pub trait MetricsRecorder {
    fn record_workflow_start(&self, workflow_id: &WorkflowId, trigger: &TriggerType);
    fn record_workflow_complete(&self, workflow_id: &WorkflowId, duration: Duration, status: ExecutionStatus);
    fn record_node_execution(&self, node_type: &str, duration: Duration, success: bool);
    fn record_worker_task(&self, worker_id: &WorkerId, success: bool);
}

pub struct PrometheusRecorder;

impl MetricsRecorder for PrometheusRecorder {
    fn record_workflow_start(&self, workflow_id: &WorkflowId, trigger: &TriggerType) {
        WORKFLOW_EXECUTIONS
            .with_label_values(&[
                &workflow_id.to_string(),
                "started",
                &trigger.to_string()
            ])
            .inc();
    }
    
    fn record_workflow_complete(&self, workflow_id: &WorkflowId, duration: Duration, status: ExecutionStatus) {
        WORKFLOW_EXECUTIONS
            .with_label_values(&[
                &workflow_id.to_string(),
                &status.to_string(),
                ""
            ])
            .inc();
            
        WORKFLOW_DURATION
            .with_label_values(&[&workflow_id.to_string()])
            .observe(duration.as_secs_f64());
    }
}
```

## Logging

### Log Structure

```rust
#[derive(Serialize)]
pub struct LogEntry {
    // Standard fields
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub message: String,
    
    // Context fields
    pub service: String,
    pub version: String,
    pub environment: String,
    
    // Trace context
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    
    // Business context
    pub workflow_id: Option<WorkflowId>,
    pub execution_id: Option<ExecutionId>,
    pub node_id: Option<NodeId>,
    pub user_id: Option<UserId>,
    
    // Additional fields
    pub fields: HashMap<String, Value>,
}

// Structured logging macros
macro_rules! log_with_context {
    ($level:expr, $msg:expr, $($key:expr => $value:expr),*) => {
        let mut fields = HashMap::new();
        $(fields.insert($key.to_string(), json!($value));)*
        
        let entry = LogEntry {
            timestamp: Utc::now(),
            level: $level,
            message: $msg.to_string(),
            service: env!("CARGO_PKG_NAME").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            environment: std::env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string()),
            trace_id: current_trace_id(),
            span_id: current_span_id(),
            workflow_id: context::get::<WorkflowId>(),
            execution_id: context::get::<ExecutionId>(),
            node_id: context::get::<NodeId>(),
            user_id: context::get::<UserId>(),
            fields,
        };
        
        emit_log(entry);
    };
}
```

### Log Levels and Usage

```rust
// Error - System errors requiring immediate attention
error_with_context!(
    "Failed to execute workflow",
    "workflow_id" => workflow_id,
    "error" => error.to_string(),
    "error_type" => std::any::type_name_of_val(&error)
);

// Warn - Potential issues that don't prevent operation
warn_with_context!(
    "Retry attempt for node execution",
    "node_id" => node_id,
    "attempt" => retry_count,
    "reason" => last_error
);

// Info - Significant business events
info_with_context!(
    "Workflow execution completed",
    "workflow_id" => workflow_id,
    "duration_ms" => duration.as_millis(),
    "node_count" => executed_nodes
);

// Debug - Detailed technical information
debug_with_context!(
    "Node input validation",
    "node_type" => node_type,
    "input_size" => input_size,
    "validation_result" => result
);
```

## Distributed Tracing

### Trace Implementation

```rust
use opentelemetry::{
    trace::{Tracer, TracerProvider, Span, StatusCode, SpanKind},
    KeyValue,
};

pub struct TracingMiddleware {
    tracer: Box<dyn Tracer>,
}

impl TracingMiddleware {
    pub async fn trace_workflow_execution<F, Fut>(
        &self,
        workflow_id: &WorkflowId,
        execution_id: &ExecutionId,
        f: F,
    ) -> Result<WorkflowResult, Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<WorkflowResult, Error>>,
    {
        let mut span = self.tracer
            .span_builder("workflow.execute")
            .with_kind(SpanKind::Internal)
            .with_attributes(vec![
                KeyValue::new("workflow.id", workflow_id.to_string()),
                KeyValue::new("execution.id", execution_id.to_string()),
                KeyValue::new("service.name", "nebula-engine"),
            ])
            .start(&self.tracer);
            
        let _guard = span.clone().entered();
        
        match f().await {
            Ok(result) => {
                span.set_status(StatusCode::Ok, "Workflow completed successfully");
                span.set_attribute(KeyValue::new("workflow.status", "success"));
                Ok(result)
            }
            Err(error) => {
                span.record_error(&error);
                span.set_status(StatusCode::Error, error.to_string());
                span.set_attribute(KeyValue::new("workflow.status", "failed"));
                Err(error)
            }
        }
    }
    
    pub async fn trace_node_execution<F, Fut>(
        &self,
        node: &Node,
        parent_span: &Span,
        f: F,
    ) -> Result<Value, Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Value, Error>>,
    {
        let mut span = self.tracer
            .span_builder(&format!("node.{}", node.node_type))
            .with_parent_context(parent_span.span_context())
            .with_attributes(vec![
                KeyValue::new("node.id", node.id.to_string()),
                KeyValue::new("node.type", node.node_type.clone()),
                KeyValue::new("node.name", node.name.clone()),
            ])
            .start(&self.tracer);
            
        span.add_event(
            "node.execution.started",
            vec![KeyValue::new("timestamp", Utc::now().to_rfc3339())],
        );
        
        let result = f().await;
        
        span.add_event(
            "node.execution.completed",
            vec![
                KeyValue::new("timestamp", Utc::now().to_rfc3339()),
                KeyValue::new("success", result.is_ok()),
            ],
        );
        
        result
    }
}

// Trace context propagation
pub struct TraceContextPropagator;

impl TraceContextPropagator {
    pub fn inject_context(headers: &mut HeaderMap) {
        if let Some(span) = Span::current().span_context() {
            headers.insert(
                "traceparent",
                format!(
                    "00-{}-{}-01",
                    span.trace_id(),
                    span.span_id()
                ).parse().unwrap()
            );
        }
    }
    
    pub fn extract_context(headers: &HeaderMap) -> Option<SpanContext> {
        headers.get("traceparent")
            .and_then(|value| value.to_str().ok())
            .and_then(|traceparent| parse_traceparent(traceparent))
    }
}

