//! Custom Observability Example
//!
//! This example demonstrates:
//! - Creating custom event types
//! - Implementing custom hooks
//! - Integration with external monitoring systems
//! - Advanced observability patterns

use nebula_log::observability::{emit_event, register_hook, ObservabilityEvent, ObservabilityHook};
use nebula_log::{info, warn};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "user_id": self.user_id,
            "action": self.action,
            "timestamp": self.timestamp,
            "metadata": self.metadata,
        }))
    }
}

#[derive(Debug)]
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

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "component": self.component,
            "status": format!("{:?}", self.status),
            "cpu_usage": self.metrics.cpu_usage,
            "memory_usage": self.metrics.memory_usage,
            "active_connections": self.metrics.active_connections,
        }))
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

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "metric": self.metric_name,
            "value": self.value,
            "currency": self.currency,
            "tags": self.tags.iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<_>>(),
        }))
    }
}

#[derive(Debug)]
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

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "error_type": self.error_type,
            "message": self.message,
            "stack_trace": self.stack_trace,
            "severity": format!("{:?}", self.severity),
        }))
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
        if event.name() == "error_event" {
            if let Some(data) = event.data() {
                if let Some(severity) = data.get("severity").and_then(|v| v.as_str()) {
                    if severity == "Critical" {
                        info!(hook = "slack", "Would send notification to Slack");
                        // In real implementation:
                        // slack_client.send_message("#alerts", format!("🚨 {}", data))
                    }
                }
            }
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
        // datadog_client.send_event(event.name(), event.data())
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
            // analytics_db.insert(event.name(), event.data(), timestamp)
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
