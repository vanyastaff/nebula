# Archived From "docs/archive/crates-architecture.md"

## 12. nebula-worker

**Purpose**: Worker processes for distributed execution.

```rust
// nebula-worker/src/lib.rs
use tokio::sync::mpsc;

pub struct Worker {
    id: WorkerId,
    engine: Arc<WorkflowEngine>,
    task_receiver: mpsc::Receiver<WorkerTask>,
    health_reporter: HealthReporter,
}

impl Worker {
    pub async fn run(mut self) -> Result<(), Error> {
        info!("Worker {} starting", self.id);
        
        loop {
            tokio::select! {
                Some(task) = self.task_receiver.recv() => {
                    self.handle_task(task).await?;
                }
                _ = self.health_reporter.tick() => {
                    self.report_health().await?;
                }
                _ = tokio::signal::ctrl_c() => {
                    info!("Worker {} shutting down", self.id);
                    break;
                }
            }
        }
        
        Ok(())
    }
    
    async fn handle_task(&self, task: WorkerTask) -> Result<(), Error> {
        match task {
            WorkerTask::ExecuteNode { node, context } => {
                let span = tracing::span!(Level::INFO, "worker.execute", worker.id = %self.id);
                let _enter = span.enter();
                
                self.engine.executor.execute_node(&node, context).await?;
            }
            WorkerTask::ExecuteSubgraph { subgraph, context } => {
                self.engine.execute_subgraph(&subgraph, context).await?;
            }
        }
        
        Ok(())
    }
}
```

