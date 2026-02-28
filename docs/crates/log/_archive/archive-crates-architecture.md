# Archived From "docs/archive/crates-architecture.md"

## 9. nebula-log

**Purpose**: Structured logging and tracing.

```rust
// nebula-log/src/lib.rs
use tracing::{info, warn, error, span, Level};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub struct LogManager {
    config: LogConfig,
}

impl LogManager {
    pub fn init(config: LogConfig) -> Result<(), Error> {
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_thread_ids(true)
            .json();
            
        let filter_layer = tracing_subscriber::EnvFilter::try_from_default_env()
            .or_else(|_| tracing_subscriber::EnvFilter::try_new("info"))
            .unwrap();
            
        tracing_subscriber::registry()
            .with(filter_layer)
            .with(fmt_layer)
            .init();
            
        Ok(())
    }
    
    pub fn execution_span(execution_id: &ExecutionId) -> tracing::Span {
        span!(
            Level::INFO,
            "execution",
            execution.id = %execution_id,
        )
    }
    
    pub fn node_span(node_id: &NodeId, node_type: &str) -> tracing::Span {
        span!(
            Level::INFO,
            "node",
            node.id = %node_id,
            node.type = %node_type,
        )
    }
}

// Macros for structured logging
#[macro_export]
macro_rules! log_execution_started {
    ($execution_id:expr, $workflow_id:expr) => {
        info!(
            execution_id = %$execution_id,
            workflow_id = %$workflow_id,
            "Execution started"
        );
    };
}

#[macro_export]
macro_rules! log_node_error {
    ($node_id:expr, $error:expr) => {
        error!(
            node_id = %$node_id,
            error = %$error,
            "Node execution failed"
        );
    };
}
```

