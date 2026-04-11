//! Custom Observability Example
//!
//! This example demonstrates:
//! - Creating custom event types
//! - Implementing custom hooks
//! - Integration with external monitoring systems
//! - Advanced observability patterns

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use nebula_log::{
    info,
    observability::{
        ObservabilityEvent, ObservabilityFieldValue, ObservabilityFieldVisitor, ObservabilityHook,
        emit_event, event_data_json, register_hook,
    },
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    nebula_log::init()?;

    info!("Starting custom observability example");

    println!("\n=== Custom Observability Example ===\n");

    // Register multiple custom hooks
    register_hook(Arc::new(SlackNotificationHook::new()));
    register_hook(Arc::new(DatadogHook::new()));
    register_hook(Arc::new(CustomAnalyticsHook::new()));

    // Emit various custom events
    println!("Emitting custom events...\n");

    // 1. User action event
    let user_event = UserActionEvent {
        user_id: "user_123".to_string(),
        action: "login".to_string(),
        timestamp: current_timestamp(),
        metadata: serde_json::json!({
            "ip": "192.168.1.1",
            "user_agent": "Mozilla/5.0",
        }),
    };
    emit_event(&user_event);

    // 2. System health event
    let health_event = SystemHealthEvent {
        component: "api_server".to_string(),
        status: HealthStatus::Healthy,
        metrics: HealthMetrics {
            cpu_usage: 45.2,
            memory_usage: 62.8,
            active_connections: 127,
        },
    };
    emit_event(&health_event);

    // 3. Business metric event
    let business_event = BusinessMetricEvent {
        metric_name: "revenue".to_string(),
        value: 1234.56,
        currency: "USD".to_string(),
        tags: vec![
            ("region".to_string(), "us-west".to_string()),
            ("product".to_string(), "premium".to_string()),
        ],
    };
    emit_event(&business_event);

    // 4. Error event
    let error_event = ErrorEvent {
        error_type: "DatabaseConnectionError".to_string(),
        message: "Connection timeout after 30s".to_string(),
        stack_trace: Some("at main.rs:42\nat connection.rs:108".to_string()),
        severity: ErrorSeverity::Critical,
    };
    emit_event(&error_event);

    println!("\n=== Hook Outputs ===");
    println!("✓ SlackNotificationHook: Would send critical error to #alerts channel");
    println!("✓ DatadogHook: Would send 4 events to Datadog API");
    println!("✓ CustomAnalyticsHook: Would store events in analytics database");

    println!("\n=== Integration Examples ===");
    println!("\n1. Slack Integration:");
    println!("   POST https://hooks.slack.com/services/YOUR/WEBHOOK");
    println!(r#"   {{"text": "🚨 Critical Error: DatabaseConnectionError"}}"#);

    println!("\n2. Datadog Integration:");
    println!("   POST https://api.datadoghq.com/api/v1/events");
    println!(r#"   {{"title": "System Health", "tags": ["component:api_server"]}}"#);

    println!("\n3. Custom Analytics:");
    println!("   INSERT INTO events (type, data, timestamp)");
    println!("   VALUES ('user_action', '{{...}}', '2024-01-01 12:00:00')");

    Ok(())
}

// ============================================================================
// Custom Event Types
// ============================================================================

struct UserActionEvent {
    user_id: String,
    action: String,
    timestamp: u64,
    metadata: serde_json::Value,
}

impl ObservabilityEvent for UserActionEvent {
    fn name(&self) -> &str {
        "user_action"
    }

    fn visit_fields(&self, visitor: &mut dyn ObservabilityFieldVisitor) {
        visitor.record("user_id", ObservabilityFieldValue::Str(&self.user_id));
        visitor.record("action", ObservabilityFieldValue::Str(&self.action));
        visitor.record("timestamp", ObservabilityFieldValue::U64(self.timestamp));
        let metadata = self.metadata.to_string();
        visitor.record("metadata_json", ObservabilityFieldValue::Str(&metadata));
    }
}

#[derive(Debug)]
#[allow(dead_code)]
enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

struct HealthMetrics {
    cpu_usage: f64,
    memory_usage: f64,
    active_connections: u32,
}

struct SystemHealthEvent {
    component: String,
    status: HealthStatus,
    metrics: HealthMetrics,
}

impl ObservabilityEvent for SystemHealthEvent {
    fn name(&self) -> &str {
        "system_health"
    }

    fn visit_fields(&self, visitor: &mut dyn ObservabilityFieldVisitor) {
        visitor.record("component", ObservabilityFieldValue::Str(&self.component));
        let status = format!("{:?}", self.status);
        visitor.record("status", ObservabilityFieldValue::Str(&status));
        visitor.record(
            "cpu_usage",
            ObservabilityFieldValue::F64(self.metrics.cpu_usage),
        );
        visitor.record(
            "memory_usage",
            ObservabilityFieldValue::F64(self.metrics.memory_usage),
        );
        visitor.record(
            "active_connections",
            ObservabilityFieldValue::U64(self.metrics.active_connections as u64),
        );
    }
}

struct BusinessMetricEvent {
    metric_name: String,
    value: f64,
    currency: String,
    tags: Vec<(String, String)>,
}

impl ObservabilityEvent for BusinessMetricEvent {
    fn name(&self) -> &str {
        "business_metric"
    }

    fn visit_fields(&self, visitor: &mut dyn ObservabilityFieldVisitor) {
        visitor.record("metric", ObservabilityFieldValue::Str(&self.metric_name));
        visitor.record("value", ObservabilityFieldValue::F64(self.value));
        visitor.record("currency", ObservabilityFieldValue::Str(&self.currency));
        let tags = self
            .tags
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",");
        visitor.record("tags", ObservabilityFieldValue::Str(&tags));
    }
}

#[derive(Debug)]
#[allow(dead_code)]
enum ErrorSeverity {
    Low,
    Medium,
    High,
    Critical,
}

struct ErrorEvent {
    error_type: String,
    message: String,
    stack_trace: Option<String>,
    severity: ErrorSeverity,
}

impl ObservabilityEvent for ErrorEvent {
    fn name(&self) -> &str {
        "error_event"
    }

    fn visit_fields(&self, visitor: &mut dyn ObservabilityFieldVisitor) {
        visitor.record("error_type", ObservabilityFieldValue::Str(&self.error_type));
        visitor.record("message", ObservabilityFieldValue::Str(&self.message));
        if let Some(stack_trace) = &self.stack_trace {
            visitor.record("stack_trace", ObservabilityFieldValue::Str(stack_trace));
        }
        let severity = format!("{:?}", self.severity);
        visitor.record("severity", ObservabilityFieldValue::Str(&severity));
    }
}

// ============================================================================
// Custom Hooks
// ============================================================================

struct SlackNotificationHook;

impl SlackNotificationHook {
    fn new() -> Self {
        Self
    }
}

impl ObservabilityHook for SlackNotificationHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        // Only send critical errors to Slack
        if event.name() == "error_event"
            && let Some(data) = event_data_json(event)
            && let Some(severity) = data.get("severity").and_then(|v| v.as_str())
            && severity == "Critical"
        {
            info!(hook = "slack", "Would send notification to Slack");
            // In real implementation:
            // slack_client.send_message("#alerts", format!("🚨 {}", data))
        }
    }
}

struct DatadogHook {
    event_count: std::sync::atomic::AtomicU64,
}

impl DatadogHook {
    fn new() -> Self {
        Self {
            event_count: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl ObservabilityHook for DatadogHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        self.event_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        info!(
            hook = "datadog",
            event = event.name(),
            "Would send event to Datadog"
        );
        // In real implementation:
        // datadog_client.send_event(event.name(), event_data_json(event))
    }
}

struct CustomAnalyticsHook;

impl CustomAnalyticsHook {
    fn new() -> Self {
        Self
    }
}

impl ObservabilityHook for CustomAnalyticsHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        // Store all events in custom analytics database
        if event.name() == "business_metric" {
            info!(hook = "analytics", "Would store business metric");
            // In real implementation:
            // analytics_db.insert(event.name(), event_data_json(event), timestamp)
        }
    }
}

// ============================================================================
// Utilities
// ============================================================================

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
