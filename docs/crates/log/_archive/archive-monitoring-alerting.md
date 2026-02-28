# Archived From "docs/archive/monitoring-alerting.md"

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

