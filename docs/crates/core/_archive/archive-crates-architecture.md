# Archived From "docs/archive/crates-architecture.md"

# Nebula Crates Architecture & Implementation Guide

---

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

---

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

---

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

---

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

