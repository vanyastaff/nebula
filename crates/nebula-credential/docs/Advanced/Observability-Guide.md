---
title: Observability Guide
tags: [observability, monitoring, metrics, prometheus, opentelemetry, tracing, advanced]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [advanced, platform-engineer, sre]
estimated_reading: 25
priority: P3
---

# Observability Guide

> [!NOTE] Production Observability
> nebula-credential provides comprehensive observability through Prometheus metrics, OpenTelemetry tracing, and structured logging for production monitoring and debugging.

## TL;DR

Observability stack for nebula-credential:
- **Metrics**: Prometheus with Grafana dashboards
- **Tracing**: OpenTelemetry with Jaeger/Tempo
- **Logging**: Structured JSON logs with correlation IDs
- **Health Checks**: `/health` endpoint with component status
- **Alerting**: Pre-configured alerts for critical issues

---

## Metrics with Prometheus

### Metrics Overview

```rust
use prometheus::{
    register_histogram_vec, register_counter_vec, register_gauge,
    HistogramVec, CounterVec, Gauge,
};

/// Metrics for credential operations
pub struct CredentialMetrics {
    /// Latency histogram (p50, p95, p99)
    pub operation_duration: HistogramVec,
    
    /// Total operations counter
    pub operations_total: CounterVec,
    
    /// Cache hit ratio
    pub cache_hit_ratio: Gauge,
    
    /// Active credentials
    pub active_credentials: Gauge,
    
    /// Error rate
    pub error_rate: Gauge,
}

impl CredentialMetrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        Ok(Self {
            operation_duration: register_histogram_vec!(
                "credential_operation_duration_seconds",
                "Duration of credential operations",
                &["operation", "result"],
                vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]
            )?,
            
            operations_total: register_counter_vec!(
                "credential_operations_total",
                "Total credential operations",
                &["operation", "result"]
            )?,
            
            cache_hit_ratio: register_gauge!(
                "credential_cache_hit_ratio",
                "Cache hit ratio (0.0 to 1.0)"
            )?,
            
            active_credentials: register_gauge!(
                "credential_active_total",
                "Number of active credentials"
            )?,
            
            error_rate: register_gauge!(
                "credential_error_rate",
                "Error rate (0.0 to 1.0)"
            )?,
        })
    }
}
```

### Instrumenting Operations

```rust
use std::time::Instant;

impl CredentialService {
    pub async fn get_credential(
        &self,
        id: &CredentialId,
    ) -> Result<Credential, CredentialError> {
        let start = Instant::now();
        
        // Attempt operation
        let result = self.get_credential_internal(id).await;
        
        // Record duration
        let duration = start.elapsed().as_secs_f64();
        let result_label = if result.is_ok() { "success" } else { "error" };
        
        self.metrics
            .operation_duration
            .with_label_values(&["get", result_label])
            .observe(duration);
        
        self.metrics
            .operations_total
            .with_label_values(&["get", result_label])
            .inc();
        
        result
    }
}
```

### Metrics Endpoint

```rust
use axum::{Router, routing::get};
use prometheus::{Encoder, TextEncoder};

pub fn metrics_router() -> Router {
    Router::new()
        .route("/metrics", get(metrics_handler))
}

async fn metrics_handler() -> Result<String, StatusCode> {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    String::from_utf8(buffer)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
```

**Metrics Output**:
```prometheus
# HELP credential_operation_duration_seconds Duration of credential operations
# TYPE credential_operation_duration_seconds histogram
credential_operation_duration_seconds_bucket{operation="get",result="success",le="0.001"} 1245
credential_operation_duration_seconds_bucket{operation="get",result="success",le="0.005"} 2890
credential_operation_duration_seconds_bucket{operation="get",result="success",le="0.01"} 3120
credential_operation_duration_seconds_sum{operation="get",result="success"} 15.23
credential_operation_duration_seconds_count{operation="get",result="success"} 3200

# HELP credential_operations_total Total credential operations
# TYPE credential_operations_total counter
credential_operations_total{operation="get",result="success"} 3200
credential_operations_total{operation="get",result="error"} 45

# HELP credential_cache_hit_ratio Cache hit ratio
# TYPE credential_cache_hit_ratio gauge
credential_cache_hit_ratio 0.87

# HELP credential_active_total Number of active credentials
# TYPE credential_active_total gauge
credential_active_total 1543
```

---

## Distributed Tracing with OpenTelemetry

### Setup OpenTelemetry

```toml
[dependencies]
opentelemetry = "0.21"
opentelemetry-otlp = "0.14"
tracing = "0.1"
tracing-subscriber = "0.3"
tracing-opentelemetry = "0.22"
```

### Initialize Tracing

```rust
use opentelemetry::{global, sdk::trace as sdktrace, trace::TraceError};
use opentelemetry_otlp::WithExportConfig;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_tracing() -> Result<(), TraceError> {
    // Create OTLP exporter
    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint("http://localhost:4317")
        )
        .with_trace_config(
            sdktrace::config()
                .with_resource(opentelemetry::sdk::Resource::new(vec![
                    opentelemetry::KeyValue::new("service.name", "nebula-credential"),
                    opentelemetry::KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
                ]))
        )
        .install_batch(opentelemetry::runtime::Tokio)?;
    
    // Create tracing layer
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
    
    // Initialize subscriber
    tracing_subscriber::registry()
        .with(telemetry)
        .with(tracing_subscriber::fmt::layer())
        .init();
    
    Ok(())
}
```

### Instrument Functions

```rust
use tracing::{instrument, info, error};

#[instrument(
    name = "credential.get",
    skip(self),
    fields(
        credential_id = %id,
        otel.kind = "server",
        otel.status_code = tracing::field::Empty,
    )
)]
pub async fn get_credential(
    &self,
    id: &CredentialId,
) -> Result<Credential, CredentialError> {
    info!("Fetching credential");
    
    match self.storage.get(id).await {
        Ok(credential) => {
            tracing::Span::current().record("otel.status_code", "OK");
            Ok(credential)
        }
        Err(e) => {
            tracing::Span::current().record("otel.status_code", "ERROR");
            error!(error = %e, "Failed to fetch credential");
            Err(e)
        }
    }
}
```

### Trace Context Propagation

```rust
use opentelemetry::global;
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub async fn propagate_context(
    &self,
    request: &mut Request,
) -> Result<(), Error> {
    // Get current span context
    let context = tracing::Span::current().context();
    
    // Inject into HTTP headers
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&context, &mut HeaderInjector(request.headers_mut()));
    });
    
    Ok(())
}
```

**Trace Visualization** (Jaeger):
```
Trace ID: 5f3a2b1c8e9d...
  ├─ credential.get (150ms)
  │  ├─ storage.query (80ms)
  │  ├─ decrypt (40ms)
  │  └─ audit.log (30ms)
  └─ cache.set (20ms)
```

---

## Structured Logging

### JSON Logging Format

```rust
use serde_json::json;
use tracing::{info, warn, error};

#[instrument(skip(self))]
pub async fn rotate_credential(
    &self,
    id: &CredentialId,
) -> Result<Credential, CredentialError> {
    info!(
        event = "rotation_started",
        credential_id = %id,
        "Starting credential rotation"
    );
    
    match self.rotate_internal(id).await {
        Ok(credential) => {
            info!(
                event = "rotation_completed",
                credential_id = %id,
                new_version = %credential.version,
                "Credential rotation completed"
            );
            Ok(credential)
        }
        Err(e) => {
            error!(
                event = "rotation_failed",
                credential_id = %id,
                error = %e,
                "Credential rotation failed"
            );
            Err(e)
        }
    }
}
```

**Log Output** (JSON):
```json
{
  "timestamp": "2026-02-03T10:15:30.123Z",
  "level": "INFO",
  "target": "nebula_credential::service",
  "fields": {
    "event": "rotation_started",
    "credential_id": "cred_abc123",
    "message": "Starting credential rotation"
  },
  "span": {
    "name": "rotate_credential",
    "credential_id": "cred_abc123"
  }
}
```

---

## Health Checks

### Health Check Endpoint

```rust
use axum::{Json, http::StatusCode};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: HealthStatus,
    pub components: HashMap<String, ComponentHealth>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

#[derive(Serialize, Deserialize)]
pub struct ComponentHealth {
    pub status: HealthStatus,
    pub message: Option<String>,
    pub latency_ms: Option<u64>,
}

pub async fn health_check(
    service: Arc<CredentialService>,
) -> (StatusCode, Json<HealthResponse>) {
    let mut components = HashMap::new();
    
    // Check storage
    let storage_health = check_storage_health(&service).await;
    components.insert("storage".to_string(), storage_health);
    
    // Check encryption
    let encryption_health = check_encryption_health(&service).await;
    components.insert("encryption".to_string(), encryption_health);
    
    // Check audit logging
    let audit_health = check_audit_health(&service).await;
    components.insert("audit".to_string(), audit_health);
    
    // Overall status
    let overall_status = if components.values().all(|c| matches!(c.status, HealthStatus::Healthy)) {
        HealthStatus::Healthy
    } else if components.values().any(|c| matches!(c.status, HealthStatus::Unhealthy)) {
        HealthStatus::Unhealthy
    } else {
        HealthStatus::Degraded
    };
    
    let status_code = match overall_status {
        HealthStatus::Healthy => StatusCode::OK,
        HealthStatus::Degraded => StatusCode::OK,
        HealthStatus::Unhealthy => StatusCode::SERVICE_UNAVAILABLE,
    };
    
    (
        status_code,
        Json(HealthResponse {
            status: overall_status,
            components,
            timestamp: Utc::now(),
        })
    )
}

async fn check_storage_health(service: &CredentialService) -> ComponentHealth {
    let start = Instant::now();
    
    match service.storage.health_check().await {
        Ok(_) => ComponentHealth {
            status: HealthStatus::Healthy,
            message: Some("Storage accessible".to_string()),
            latency_ms: Some(start.elapsed().as_millis() as u64),
        },
        Err(e) => ComponentHealth {
            status: HealthStatus::Unhealthy,
            message: Some(format!("Storage error: {}", e)),
            latency_ms: None,
        },
    }
}
```

**Health Response**:
```json
{
  "status": "healthy",
  "components": {
    "storage": {
      "status": "healthy",
      "message": "Storage accessible",
      "latency_ms": 15
    },
    "encryption": {
      "status": "healthy",
      "message": "Encryption operational",
      "latency_ms": 2
    },
    "audit": {
      "status": "healthy",
      "message": "Audit logging active",
      "latency_ms": 8
    }
  },
  "timestamp": "2026-02-03T10:15:30.456Z"
}
```

---

## Grafana Dashboards

### Credential Operations Dashboard

**Panels**:
1. **Request Rate** (Graph)
   ```promql
   rate(credential_operations_total[5m])
   ```

2. **Error Rate** (Graph)
   ```promql
   rate(credential_operations_total{result="error"}[5m]) /
   rate(credential_operations_total[5m])
   ```

3. **Latency Percentiles** (Graph)
   ```promql
   histogram_quantile(0.50, rate(credential_operation_duration_seconds_bucket[5m])) # p50
   histogram_quantile(0.95, rate(credential_operation_duration_seconds_bucket[5m])) # p95
   histogram_quantile(0.99, rate(credential_operation_duration_seconds_bucket[5m])) # p99
   ```

4. **Cache Hit Ratio** (Gauge)
   ```promql
   credential_cache_hit_ratio
   ```

5. **Active Credentials** (Stat)
   ```promql
   credential_active_total
   ```

### Security Events Dashboard

**Panels**:
1. **Failed Authentications** (Graph)
   ```promql
   rate(credential_operations_total{operation="authenticate",result="error"}[5m])
   ```

2. **Permission Denials** (Graph)
   ```promql
   rate(audit_events_total{event_type="PermissionDenied"}[5m])
   ```

3. **Scope Violations** (Graph)
   ```promql
   rate(audit_events_total{event_type="ScopeViolation"}[5m])
   ```

---

## Alerting Rules

### Prometheus Alerts

```yaml
groups:
  - name: credential_alerts
    interval: 30s
    rules:
      # High error rate
      - alert: HighErrorRate
        expr: |
          rate(credential_operations_total{result="error"}[5m]) /
          rate(credential_operations_total[5m]) > 0.05
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High error rate ({{ $value | humanizePercentage }})"
          description: "Error rate above 5% for 5 minutes"
      
      # High latency
      - alert: HighLatency
        expr: |
          histogram_quantile(0.95,
            rate(credential_operation_duration_seconds_bucket[5m])
          ) > 1.0
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High latency (p95: {{ $value }}s)"
          description: "p95 latency above 1 second for 5 minutes"
      
      # Storage unavailable
      - alert: StorageUnavailable
        expr: up{job="credential-storage"} == 0
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "Storage is down"
          description: "Credential storage has been down for 1 minute"
      
      # Unusual access patterns
      - alert: UnusualAccessPattern
        expr: |
          rate(credential_operations_total[5m]) >
          2 * avg_over_time(rate(credential_operations_total[5m])[1h:5m])
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "Unusual access pattern detected"
          description: "Request rate 2x higher than average"
```

---

## See Also

- [[How-To/Enable-Audit-Logging|Audit Logging Setup]]
- [[Advanced/Performance-Tuning|Performance Tuning]]
- [[Advanced/Security-Architecture|Security Architecture]]
- [[Reference/API-Reference|API Reference]]
