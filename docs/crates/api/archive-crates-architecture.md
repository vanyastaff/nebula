# Archived From "docs/archive/crates-architecture.md"

## 10. nebula-api

**Purpose**: API server for external interactions.

```rust
// nebula-api/src/lib.rs
use axum::{
    routing::{get, post},
    Router,
    Json,
    extract::{Path, State},
};
use tower_http::trace::TraceLayer;

pub struct ApiServer {
    engine: Arc<WorkflowEngine>,
    storage: Arc<dyn StorageBackend>,
}

impl ApiServer {
    pub fn new(engine: Arc<WorkflowEngine>, storage: Arc<dyn StorageBackend>) -> Self {
        Self { engine, storage }
    }
    
    pub fn routes(&self) -> Router {
        Router::new()
            // Workflow management
            .route("/workflows", post(create_workflow))
            .route("/workflows", get(list_workflows))
            .route("/workflows/:id", get(get_workflow))
            .route("/workflows/:id", put(update_workflow))
            .route("/workflows/:id", delete(delete_workflow))
            
            // Execution
            .route("/workflows/:id/execute", post(execute_workflow))
            .route("/executions/:id", get(get_execution))
            .route("/executions/:id/status", get(get_execution_status))
            
            // Resources
            .route("/resources", get(list_resources))
            .route("/resources/:type/health", get(check_resource_health))
            
            // WebSocket for real-time updates
            .route("/ws", get(websocket_handler))
            
            .layer(TraceLayer::new_for_http())
            .with_state(ApiState {
                engine: self.engine.clone(),
                storage: self.storage.clone(),
            })
    }
}

// Handler implementations
async fn create_workflow(
    State(state): State<ApiState>,
    Json(workflow): Json<Workflow>,
) -> Result<Json<CreateWorkflowResponse>, ApiError> {
    state.storage.save_workflow(&workflow).await?;
    state.engine.deploy_workflow(workflow.clone()).await?;
    
    Ok(Json(CreateWorkflowResponse {
        id: workflow.id,
        version: workflow.version,
    }))
}

async fn execute_workflow(
    State(state): State<ApiState>,
    Path(workflow_id): Path<String>,
    Json(input): Json<serde_json::Value>,
) -> Result<Json<ExecuteWorkflowResponse>, ApiError> {
    let workflow_id = WorkflowId::from(workflow_id);
    let data_item = WorkflowDataItem {
        json: input,
        binary: None,
        metadata: Default::default(),
    };
    
    let handle = state.engine.execute_workflow(&workflow_id, data_item).await?;
    
    Ok(Json(ExecuteWorkflowResponse {
        execution_id: handle.execution_id,
        status: handle.status,
    }))
}
```

