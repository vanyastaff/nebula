# Archived From "docs/archive/layers-interaction.md"

### 5. nebula-eventbus ↔ nebula-execution ↔ nebula-log

**Event flow:** Execution генерирует события, Log их записывает

```rust
// nebula-execution генерирует события
impl ExecutionContext {
    pub async fn start_node(&self, node_id: NodeId) -> Result<()> {
        // Emit event через eventbus
        self.event_bus.publish(NodeEvent::Started {
            execution_id: self.execution_id.clone(),
            workflow_id: self.workflow_id.clone(),
            node_id: node_id.clone(),
            timestamp: SystemTime::now(),
        }).await?;
        
        // Также логируем
        self.logger.info(&format!("Starting node {}", node_id));
        
        Ok(())
    }
}

// nebula-log подписывается на события
pub struct EventLogger {
    logger: Logger,
}

impl EventLogger {
    pub fn subscribe_to_events(event_bus: &EventBus) {
        // Подписываемся на все execution события
        event_bus.subscribe(|event: ExecutionEvent| async move {
            match event {
                ExecutionEvent::Started { execution_id, workflow_id, .. } => {
                    log::info!(
                        target: "execution",
                        execution_id = %execution_id,
                        workflow_id = %workflow_id,
                        "Execution started"
                    );
                }
                ExecutionEvent::Failed { execution_id, error, .. } => {
                    log::error!(
                        target: "execution", 
                        execution_id = %execution_id,
                        error = %error,
                        "Execution failed"
                    );
                }
                // ...
            }
        });
        
        // Подписываемся на node события
        event_bus.subscribe(|event: NodeEvent| async move {
            match event {
                NodeEvent::Started { node_id, .. } => {
                    log::debug!("Node {} started", node_id);
                }
                NodeEvent::Completed { node_id, duration, .. } => {
                    log::info!("Node {} completed in {:?}", node_id, duration);
                }
                // ...
            }
        });
    }
}

// nebula-metrics тоже слушает события
pub struct MetricsCollector;

impl MetricsCollector {
    pub fn subscribe_to_events(event_bus: &EventBus) {
        event_bus.subscribe(|event: NodeEvent| async move {
            match event {
                NodeEvent::Completed { duration, .. } => {
                    metrics::histogram!("node_duration_seconds", duration.as_secs_f64());
                    metrics::increment_counter!("nodes_completed_total");
                }
                NodeEvent::Failed { .. } => {
                    metrics::increment_counter!("nodes_failed_total");
                }
                // ...
            }
        });
    }
}
```
