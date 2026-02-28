# Nebula Crates Architecture & Implementation Guide

## Crate Structure Overview

```
crates/
├── nebula-api/          # REST + WebSocket API server
├── nebula-core/         # Core types, traits, and abstractions
├── nebula-derive/       # Procedural macros for nodes and parameters
├── nebula-log/          # Structured logging and tracing
├── nebula-memory/       # In-memory state management and caching
├── nebula-registry/     # Node registry and plugin management
├── nebula-runtime/      # Workflow execution engine
├── nebula-storage/      # Storage abstraction layer
├── nebula-storage-postgres/ # PostgreSQL implementation
├── nebula-template/     # Template engine for expressions
├── nebula-ui/           # egui-based UI application
└── nebula-worker/       # Worker processes for distributed execution
```

## 1. nebula-core

**Purpose**: Core types, traits, and abstractions used throughout the system.

```rust
// nebula-core/src/lib.rs
pub mod action;
pub mod connection;
pub mod error;
pub mod graph;
pub mod parameter;
pub mod resource;
pub mod workflow;

// nebula-core/src/action.rs
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait Action: Send + Sync + 'static {
    type Input: DeserializeOwned + Send + Sync;
    type Output: Serialize + Send + Sync;
    type Error: std::error::Error + Send + Sync;
    
    fn metadata(&self) -> ActionMetadata;
    
    async fn execute(
        &self,
        input: Self::Input,
        context: &mut ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, Self::Error>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub version: Version,
    pub inputs: Vec<Connection>,
    pub outputs: Vec<Connection>,
}

// nebula-core/src/workflow.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: WorkflowId,
    pub name: String,
    pub description: Option<String>,
    pub version: Version,
    pub graph: WorkflowGraph,
    pub metadata: WorkflowMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowGraph {
    pub nodes: HashMap<NodeId, Node>,
    pub edges: Vec<Edge>,
    pub subgraphs: HashMap<SubgraphId, WorkflowGraph>,
}

// nebula-core/src/connection.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Connection {
    Flow { key: String, name: String },
    Support {
        key: String,
        name: String,
        description: String,
        required: bool,
        filter: ConnectionFilter,
    },
    Dynamic {
        key: String,
        name: MaybeExpression<String>,
        description: MaybeExpression<String>,
    },
}

// nebula-core/src/resource.rs
#[async_trait]
pub trait Resource: Send + Sync + 'static {
    async fn initialize(&mut self, config: ResourceConfig) -> Result<(), Error>;
    async fn health_check(&self) -> Result<HealthStatus, Error>;
    async fn shutdown(&mut self) -> Result<(), Error>;
}

// nebula-core/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Action execution failed: {0}")]
    ActionExecutionFailed(String),
    
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),
    
    #[error("Resource not available: {0}")]
    ResourceNotAvailable(String),
    
    #[error("Workflow validation failed: {0}")]
    WorkflowValidationFailed(String),
}
```

## 2. nebula-derive

**Purpose**: Procedural macros for generating boilerplate code.

```rust
// nebula-derive/src/lib.rs
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(Action, attributes(action))]
pub fn derive_action(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    // Implementation for Action derive
}

#[proc_macro_derive(Parameters, attributes(param, validate, display))]
pub fn derive_parameters(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    
    // Generate parameter collection
    let gen = quote! {
        impl Parameters for #name {
            fn parameter_collection() -> ParameterCollection {
                // Generated code
            }
            
            fn from_values(values: HashMap<Key, ParameterValue>) -> Result<Self, Error> {
                // Generated code
            }
        }
    };
    
    gen.into()
}

// Example attributes handling
#[proc_macro_attribute]
pub fn node(args: TokenStream, input: TokenStream) -> TokenStream {
    // Parse node metadata from attributes
}
```

## 3. Value layer (serde / serde_json::Value)

Отдельный crate nebula-value не используется. Значения и сериализация — через **serde** и **serde_json::Value**.

```rust
// Типы данных workflow — serde_json::Value
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDataItem {
    pub json: JsonValue,
    pub binary: Option<BinaryData>,
    pub metadata: DataMetadata,
}

// Expression/transform — в nebula-expression, nebula-template; работают с JsonValue
```

## 4. nebula-runtime

**Purpose**: Workflow execution engine and scheduling.

```rust
// nebula-runtime/src/lib.rs
pub mod engine;
pub mod executor;
pub mod scheduler;
pub mod context;

// nebula-runtime/src/engine.rs
pub struct WorkflowEngine {
    scheduler: Arc<Scheduler>,
    executor: Arc<Executor>,
    state_manager: Arc<StateManager>,
    resource_pool: Arc<ResourcePool>,
}

impl WorkflowEngine {
    pub async fn new(config: EngineConfig) -> Result<Self, Error> {
        // Initialize components
    }
    
    pub async fn deploy_workflow(&self, workflow: Workflow) -> Result<WorkflowId, Error> {
        // Validate and deploy workflow
    }
    
    pub async fn execute_workflow(
        &self,
        workflow_id: &WorkflowId,
        input: WorkflowDataItem,
    ) -> Result<ExecutionHandle, Error> {
        // Create execution and submit to scheduler
    }
}

// nebula-runtime/src/executor.rs
pub struct Executor {
    registry: Arc<Registry>,
    worker_pool: Arc<WorkerPool>,
}

impl Executor {
    pub async fn execute_node(
        &self,
        node: &Node,
        context: &mut ExecutionContext,
    ) -> Result<ActionResult, Error> {
        let action = self.registry.get_action(&node.action_type)?;
        
        // Prepare input
        let input = self.prepare_input(node, context).await?;
        
        // Execute action
        let result = action.execute(input, context).await?;
        
        // Handle result
        self.handle_result(node, result, context).await
    }
}

// nebula-runtime/src/context.rs
pub struct ExecutionContext {
    pub execution_id: ExecutionId,
    pub workflow_id: WorkflowId,
    pub variables: Variables,
    pub node_outputs: HashMap<NodeId, WorkflowDataItem>,
    pub resources: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    pub supplied_instances: HashMap<String, Arc<dyn Any + Send + Sync>>,
}

impl ExecutionContext {
    pub fn get_resource<T: Resource>(&self) -> Option<Arc<T>> {
        self.resources
            .get(&TypeId::of::<T>())
            .and_then(|r| r.clone().downcast::<T>().ok())
    }
    
    pub fn get_supplied_instance<T: 'static>(&self, key: &str) -> Option<Arc<T>> {
        self.supplied_instances
            .get(key)
            .and_then(|r| r.clone().downcast::<T>().ok())
    }
}
```

## 5. nebula-registry

**Purpose**: Node registry and plugin management.

```rust
// nebula-registry/src/lib.rs
pub mod action;
pub mod plugin;
pub mod discovery;

// nebula-registry/src/action.rs
pub struct ActionRegistry {
    actions: RwLock<HashMap<String, Arc<dyn Action>>>,
    metadata: RwLock<HashMap<String, ActionMetadata>>,
}

impl ActionRegistry {
    pub fn register<A: Action + 'static>(&self, action: A) -> Result<(), Error> {
        let metadata = action.metadata();
        let id = metadata.id.clone();
        
        self.actions.write().unwrap().insert(id.clone(), Arc::new(action));
        self.metadata.write().unwrap().insert(id, metadata);
        
        Ok(())
    }
    
    pub fn get_action(&self, id: &str) -> Option<Arc<dyn Action>> {
        self.actions.read().unwrap().get(id).cloned()
    }
    
    pub fn list_actions(&self) -> Vec<ActionMetadata> {
        self.metadata.read().unwrap().values().cloned().collect()
    }
}

// nebula-registry/src/plugin.rs
pub struct PluginManager {
    plugins: Vec<LoadedPlugin>,
    sandbox: Arc<PluginSandbox>,
}

pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub handle: PluginHandle,
    pub actions: Vec<String>,
}

impl PluginManager {
    pub async fn load_plugin(&mut self, path: &Path) -> Result<(), Error> {
        // Load and validate plugin
        let manifest = self.read_manifest(path)?;
        let handle = self.sandbox.load(path).await?;
        
        // Register plugin actions
        self.register_plugin_actions(&handle).await?;
        
        Ok(())
    }
}
```

## 6. nebula-memory

**Purpose**: In-memory state management and caching.

```rust
// nebula-memory/src/lib.rs
pub mod cache;
pub mod state;
pub mod pool;

// nebula-memory/src/state.rs
pub struct InMemoryStateStore {
    states: Arc<RwLock<HashMap<ExecutionId, ExecutionState>>>,
    indexes: Arc<RwLock<StateIndexes>>,
}

impl InMemoryStateStore {
    pub async fn save_state(&self, state: ExecutionState) -> Result<(), Error> {
        let mut states = self.states.write().unwrap();
        let execution_id = state.execution_id.clone();
        
        states.insert(execution_id.clone(), state);
        self.update_indexes(&execution_id).await?;
        
        Ok(())
    }
    
    pub async fn get_state(&self, id: &ExecutionId) -> Option<ExecutionState> {
        self.states.read().unwrap().get(id).cloned()
    }
}

// nebula-memory/src/cache.rs
pub struct CacheManager {
    l1_cache: Arc<Mutex<LruCache<CacheKey, CachedValue>>>,
    stats: Arc<CacheStats>,
}

impl CacheManager {
    pub async fn get<T: DeserializeOwned>(&self, key: &CacheKey) -> Option<T> {
        let mut cache = self.l1_cache.lock().unwrap();
        
        if let Some(value) = cache.get(key) {
            self.stats.record_hit();
            serde_json::from_value(value.data.clone()).ok()
        } else {
            self.stats.record_miss();
            None
        }
    }
}

// nebula-memory/src/pool.rs
pub struct ResourcePool {
    pools: HashMap<TypeId, Box<dyn TypedPool>>,
}

impl ResourcePool {
    pub fn register_pool<T: 'static>(&mut self, pool: impl TypedPool<Resource = T> + 'static) {
        self.pools.insert(TypeId::of::<T>(), Box::new(pool));
    }
    
    pub async fn acquire<T: 'static>(&self) -> Result<PooledResource<T>, Error> {
        let pool = self.pools
            .get(&TypeId::of::<T>())
            .ok_or_else(|| Error::ResourceNotFound)?;
            
        pool.acquire().await
    }
}
```

## 7. nebula-storage & nebula-storage-postgres

**Purpose**: Storage abstraction and PostgreSQL implementation.

```rust
// nebula-storage/src/lib.rs
#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn save_workflow(&self, workflow: &Workflow) -> Result<(), Error>;
    async fn load_workflow(&self, id: &WorkflowId) -> Result<Workflow, Error>;
    async fn list_workflows(&self, filter: WorkflowFilter) -> Result<Vec<WorkflowSummary>, Error>;
    
    async fn save_execution(&self, execution: &ExecutionState) -> Result<(), Error>;
    async fn load_execution(&self, id: &ExecutionId) -> Result<ExecutionState, Error>;
    async fn update_execution_status(&self, id: &ExecutionId, status: ExecutionStatus) -> Result<(), Error>;
}

// nebula-storage-postgres/src/lib.rs
use sqlx::{PgPool, postgres::PgPoolOptions};

pub struct PostgresStorage {
    pool: PgPool,
}

impl PostgresStorage {
    pub async fn new(database_url: &str) -> Result<Self, Error> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(database_url)
            .await?;
            
        Ok(Self { pool })
    }
    
    pub async fn migrate(&self) -> Result<(), Error> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl StorageBackend for PostgresStorage {
    async fn save_workflow(&self, workflow: &Workflow) -> Result<(), Error> {
        let workflow_json = serde_json::to_value(workflow)?;
        
        sqlx::query!(
            r#"
            INSERT INTO workflows (id, name, version, definition, created_at, updated_at)
            VALUES ($1, $2, $3, $4, NOW(), NOW())
            ON CONFLICT (id) DO UPDATE SET
                definition = EXCLUDED.definition,
                updated_at = NOW()
            "#,
            workflow.id.as_str(),
            workflow.name,
            workflow.version.to_string(),
            workflow_json
        )
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
    
    // Other implementations...
}
```

## 8. nebula-template

**Purpose**: Template engine for expressions and text generation.

```rust
// nebula-template/src/lib.rs
pub mod engine;
pub mod expression;
pub mod functions;

// nebula-template/src/engine.rs
pub struct TemplateEngine {
    handlebars: Handlebars<'static>,
    custom_functions: HashMap<String, Box<dyn TemplateFunction>>,
}

impl TemplateEngine {
    pub fn new() -> Self {
        let mut handlebars = Handlebars::new();
        
        // Register custom helpers
        handlebars.register_helper("json", Box::new(json_helper));
        handlebars.register_helper("base64", Box::new(base64_helper));
        
        Self {
            handlebars,
            custom_functions: HashMap::new(),
        }
    }
    
    pub fn render(&self, template: &str, context: &TemplateContext) -> Result<String, Error> {
        self.handlebars.render_template(template, context)
            .map_err(|e| Error::TemplateError(e.to_string()))
    }
}

// nebula-template/src/expression.rs
pub struct ExpressionParser {
    parser: pest::Parser,
}

impl ExpressionParser {
    pub fn parse(&self, expression: &str) -> Result<Expression, Error> {
        // Parse expressions like {{ $node("http_request").json.data }}
    }
}

// nebula-template/src/functions.rs
pub trait TemplateFunction: Send + Sync {
    fn name(&self) -> &str;
    fn execute(&self, args: &[Value]) -> Result<Value, Error>;
}

pub struct DateFormatFunction;

impl TemplateFunction for DateFormatFunction {
    fn name(&self) -> &str {
        "date_format"
    }
    
    fn execute(&self, args: &[Value]) -> Result<Value, Error> {
        // Implementation
    }
}
```

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

## 11. nebula-ui

**Purpose**: egui-based UI application.

```rust
// nebula-ui/src/lib.rs
use eframe::egui;
use egui_node_graph::{NodeGraph, NodeId as EguiNodeId};

pub struct NebulaApp {
    workflow_editor: WorkflowEditor,
    node_palette: NodePalette,
    properties_panel: PropertiesPanel,
    execution_viewer: ExecutionViewer,
    api_client: ApiClient,
}

impl eframe::App for NebulaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Top menu bar
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New Workflow").clicked() {
                        self.workflow_editor.new_workflow();
                    }
                    if ui.button("Open...").clicked() {
                        self.open_workflow_dialog();
                    }
                    if ui.button("Save").clicked() {
                        self.save_current_workflow();
                    }
                });
            });
        });
        
        // Left panel - Node palette
        egui::SidePanel::left("node_palette").show(ctx, |ui| {
            self.node_palette.render(ui);
        });
        
        // Right panel - Properties
        egui::SidePanel::right("properties").show(ctx, |ui| {
            self.properties_panel.render(ui, &mut self.workflow_editor);
        });
        
        // Central panel - Workflow editor
        egui::CentralPanel::default().show(ctx, |ui| {
            self.workflow_editor.render(ui);
        });
    }
}

// nebula-ui/src/editor.rs
pub struct WorkflowEditor {
    graph: NodeGraph,
    selected_node: Option<EguiNodeId>,
    connection_in_progress: Option<ConnectionInProgress>,
}

impl WorkflowEditor {
    pub fn render(&mut self, ui: &mut egui::Ui) {
        let response = self.graph.show(ui);
        
        // Handle node selection
        if let Some(node_id) = response.selected_nodes.first() {
            self.selected_node = Some(*node_id);
        }
        
        // Handle connection creation
        if let Some(connection) = &response.connection_in_progress {
            self.connection_in_progress = Some(connection.clone());
        }
        
        // Handle node deletion
        for node_id in response.deleted_nodes {
            self.graph.remove_node(node_id);
        }
    }
}
```

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

## Cargo.toml Structure

```toml
[workspace]
members = [
    "crates/nebula-api",
    "crates/nebula-core",
    "crates/nebula-derive",
    "crates/nebula-log",
    "crates/nebula-memory",
    "crates/nebula-registry",
    "crates/nebula-runtime",
    "crates/nebula-storage",
    "crates/nebula-storage-postgres",
    "crates/nebula-template",
    "crates/nebula-ui",
    "crates/nebula-worker",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
authors = ["Your Name <your.email@example.com>"]
license = "MIT OR Apache-2.0"
repository = "https://github.com/yourusername/nebula"

[workspace.dependencies]
# Async runtime
tokio = { version = "1.35", features = ["full"] }
async-trait = "0.1"

# Serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Error handling
thiserror = "1.0"
anyhow = "1.0"

# Web framework
axum = "0.7"
tower = "0.4"
tower-http = { version = "0.5", features = ["trace"] }

# Database
sqlx = { version = "0.7", features = ["runtime-tokio-rustls", "postgres", "json", "uuid", "chrono"] }

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# UI
eframe = "0.25"
egui = "0.25"
egui_node_graph = "0.4"

# Testing
mockall = "0.12"
proptest = "1.4"
criterion = "0.5"
```

## Development Workflow

### 1. Start with Core Types
```bash
cd crates/nebula-core
cargo build
cargo test
```

### 2. Implement Derive Macros
```bash
cd crates/nebula-derive
cargo build
cargo test
```

### 3. Build Runtime Components
```bash
cd crates/nebula-runtime
cargo build
cargo test
```

### 4. Add Storage Layer
```bash
cd crates/nebula-storage-postgres
# Run PostgreSQL in Docker
docker run -d -p 5432:5432 -e POSTGRES_PASSWORD=nebula postgres:16
cargo test
```

### 5. Create Example Nodes
```rust
// examples/basic_node.rs
use nebula_core::prelude::*;
use nebula_derive::{Action, Parameters};

#[derive(Action)]
#[action(
    id = "http_request",
    name = "HTTP Request",
    category = "Network"
)]
pub struct HttpRequestNode;

#[derive(Parameters)]
pub struct HttpRequestParams {
    #[param(required)]
    url: String,
    
    #[param(default = "GET")]
    method: String,
    
    #[param(optional)]
    headers: HashMap<String, String>,
}

#[async_trait]
impl ExecutableNode for HttpRequestNode {
    type Input = HttpRequestParams;
    type Output = serde_json::Value;
    
    async fn execute(
        &self,
        input: Self::Input,
        context: &mut ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, NodeError> {
        // Implementation
        Ok(ActionResult::Success(json!({"status": "ok"})))
    }
}
```

## Testing Strategy

### Unit Tests
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_node_execution() {
        let node = HttpRequestNode;
        let params = HttpRequestParams {
            url: "https://api.example.com".to_string(),
            method: "GET".to_string(),
            headers: HashMap::new(),
        };
        
        let mut context = ExecutionContext::new();
        let result = node.execute(params, &mut context).await;
        
        assert!(result.is_ok());
    }
}
```

### Integration Tests
```rust
// tests/integration/workflow_execution.rs
#[tokio::test]
async fn test_full_workflow_execution() {
    let engine = create_test_engine().await;
    let workflow = create_test_workflow();
    
    engine.deploy_workflow(workflow).await.unwrap();
    
    let result = engine.execute_workflow(
        &workflow.id,
        json!({"test": "data"})
    ).await;
    
    assert!(result.is_ok());
}
```

## Next Steps

1. **Implement Core Types** - Start with `nebula-core`
2. **Build Derive Macros** - Create the parameter system
3. **Create Basic Nodes** - HTTP, Transform, Log nodes
4. **Implement Runtime** - Basic execution engine
5. **Add Storage** - PostgreSQL backend
6. **Build UI** - Basic workflow editor
7. **Test Integration** - End-to-end tests
8. **Add Advanced Features** - Streaming, distributed execution