# nebula-core

## Overview

`nebula-core` is the foundational crate that defines the core traits, types, and abstractions used throughout the Nebula system. It provides the fundamental building blocks that all other components depend on.

## Responsibilities

- **Core Traits**: Define the `Action` and `TriggerAction` traits
- **Identifier Types**: Type-safe IDs for workflows, nodes, executions, and triggers
- **Parameter System**: Framework for node parameter definition and validation
- **Error Handling**: Comprehensive error types and result patterns
- **Metadata System**: Node and workflow metadata structures

## Core Types

### Identifier Types

All identifiers in Nebula are strongly typed to prevent mixing different ID types:

```rust
use uuid::Uuid;

/// Unique identifier for a workflow definition
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowId(Uuid);

/// Unique identifier for a node within a workflow
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(String);

/// Unique identifier for a workflow execution instance
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExecutionId(Uuid);

/// Unique identifier for a trigger
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TriggerId(Uuid);

impl WorkflowId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
    
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}
```

### Result Types

Standardized result types for consistency across the system:

```rust
/// Standard result type for nebula operations
pub type Result<T> = std::result::Result<T, Error>;

/// Result type for async operations
pub type AsyncResult<T> = Pin<Box<dyn Future<Output = Result<T>> + Send>>;
```

## Core Traits

### Action Trait

The `Action` trait defines the interface for all executable nodes:

```rust
use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait Action: Send + Sync {
    /// Execute the action with given parameters and context
    async fn execute(
        &self,
        params: ParameterCollection,
        context: &ExecutionContext,
    ) -> Result<Value>;
    
    /// Get metadata about this action
    fn metadata(&self) -> ActionMetadata;
    
    /// Get the parameter schema for this action
    fn parameter_schema(&self) -> ParameterCollection;
    
    /// Validate parameters before execution (optional)
    fn validate_parameters(&self, params: &ParameterCollection) -> Result<()> {
        // Default implementation validates against schema
        self.parameter_schema().validate(params)
    }
}
```

### TriggerAction Trait

The `TriggerAction` trait defines the interface for workflow triggers:

```rust
#[async_trait]
pub trait TriggerAction: Send + Sync {
    /// Start the trigger, returning a stream of events
    async fn start(&self, context: &TriggerContext) -> Result<TriggerEventStream>;
    
    /// Stop the trigger gracefully
    async fn stop(&self) -> Result<()>;
    
    /// Get metadata about this trigger
    fn metadata(&self) -> TriggerMetadata;
    
    /// Get the configuration schema for this trigger
    fn config_schema(&self) -> ParameterCollection;
}

/// Stream of trigger events
pub type TriggerEventStream = Pin<Box<dyn Stream<Item = TriggerEvent> + Send>>;

/// Event emitted by a trigger
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerEvent {
    pub trigger_id: TriggerId,
    pub timestamp: DateTime<Utc>,
    pub data: Value,
    pub metadata: Option<serde_json::Value>,
}
```

## Parameter System

### Parameter Definition

Parameters are defined using a builder pattern with validation:

```rust
use serde_json::Value;
// ValueType / ParamKind — enum в core/parameter; Validator — nebula-validator

pub struct Parameter {
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub value_type: ParamKind,  // String, Number, Boolean, Array, Object, etc.
    pub required: bool,
    pub default: Option<Value>,
    pub validators: Vec<Box<dyn Validator>>,
    pub display_options: DisplayOptions,
}

pub struct ParameterCollection {
    parameters: HashMap<String, Parameter>,
}

impl ParameterCollection {
    pub fn new() -> Self {
        Self {
            parameters: HashMap::new(),
        }
    }
    
    pub fn add_parameter(mut self, parameter: Parameter) -> Self {
        self.parameters.insert(parameter.name.clone(), parameter);
        self
    }
    
    pub fn required_string(mut self, name: &str, display_name: &str) -> Self {
        self.parameters.insert(
            name.to_string(),
            Parameter {
                name: name.to_string(),
                display_name: display_name.to_string(),
                description: None,
                value_type: ParamKind::String,
                required: true,
                default: None,
                validators: vec![],
                display_options: DisplayOptions::default(),
            },
        );
        self
    }
    
    pub fn optional_integer(mut self, name: &str, display_name: &str, default: i64) -> Self {
        self.parameters.insert(
            name.to_string(),
            Parameter {
                name: name.to_string(),
                display_name: display_name.to_string(),
                description: None,
                value_type: ParamKind::Integer,
                required: false,
                default: Some(Value::Number(default.into())),
                validators: vec![],
                display_options: DisplayOptions::default(),
            },
        );
        self
    }
}
```

### Display Options

Control how parameters are displayed in the UI:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayOptions {
    pub widget: WidgetType,
    pub placeholder: Option<String>,
    pub help_text: Option<String>,
    pub show_when: Option<String>, // Expression for conditional display
    pub order: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WidgetType {
    TextInput,
    TextArea,
    NumberInput,
    Checkbox,
    Select { options: Vec<SelectOption> },
    MultiSelect { options: Vec<SelectOption> },
    DatePicker,
    ColorPicker,
    FilePicker,
    CodeEditor { language: Option<String> },
}
```

## Metadata System

### Action Metadata

Comprehensive metadata for actions:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub version: semver::Version,
    pub author: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub tags: Vec<String>,
    pub documentation_url: Option<String>,
    pub source_url: Option<String>,
    pub license: Option<String>,
    pub capabilities: CapabilityRequirements,
    pub resource_requirements: ResourceRequirements,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRequirements {
    pub network_access: bool,
    pub filesystem_access: bool,
    pub system_commands: bool,
    pub custom_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRequirements {
    pub memory_mb: Option<u64>,
    pub cpu_cores: Option<f32>,
    pub disk_mb: Option<u64>,
    pub execution_timeout: Option<std::time::Duration>,
}
```

## Error System

### Error Types

Comprehensive error handling with context:

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Parameter validation failed: {0}")]
    ParameterValidation(String),
    
    #[error("Execution failed: {0}")]
    Execution(String),
    
    #[error("Resource not found: {resource_type} with id {id}")]
    ResourceNotFound {
        resource_type: String,
        id: String,
    },
    
    #[error("Permission denied: {operation} requires capability {capability}")]
    PermissionDenied {
        operation: String,
        capability: String,
    },
    
    #[error("Resource limit exceeded: {resource} limit is {limit}")]
    ResourceLimitExceeded {
        resource: String,
        limit: String,
    },
    
    #[error("Timeout occurred after {duration:?}")]
    Timeout {
        duration: std::time::Duration,
    },
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Internal error: {0}")]
    Internal(String),
}

impl Error {
    /// Add context to an error
    pub fn with_context<S: Into<String>>(self, context: S) -> Self {
        match self {
            Error::Internal(msg) => Error::Internal(format!("{}: {}", context.into(), msg)),
            other => Error::Internal(format!("{}: {}", context.into(), other)),
        }
    }
}
```

## Execution Context

### Context Definition

The execution context provides access to system resources:

```rust
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ExecutionContext {
    pub execution_id: ExecutionId,
    pub workflow_id: WorkflowId,
    pub node_id: NodeId,
    pub capabilities: CapabilitySet,
    pub memory: Arc<RwLock<ExecutionMemory>>,
    pub resources: ResourcePool,
    pub metrics: MetricsCollector,
    pub logger: Logger,
}

impl ExecutionContext {
    /// Get a value from execution memory
    pub async fn get_value(&self, key: &str) -> Option<Value> {
        self.memory.read().await.get(key).cloned()
    }
    
    /// Set a value in execution memory
    pub async fn set_value(&self, key: String, value: Value) {
        self.memory.write().await.insert(key, value);
    }
    
    /// Check if the context has a specific capability
    pub fn has_capability(&self, capability: &str) -> bool {
        self.capabilities.has(capability)
    }
    
    /// Record a metric
    pub fn record_metric(&self, name: &str, value: f64, tags: Vec<(&str, &str)>) {
        self.metrics.record(name, value, tags);
    }
}
```

## Testing Utilities

### Mock Implementations

Utilities for testing actions:

```rust
pub struct MockExecutionContext {
    pub values: HashMap<String, Value>,
    pub capabilities: CapabilitySet,
}

impl MockExecutionContext {
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
            capabilities: CapabilitySet::all(),
        }
    }
    
    pub fn with_value(mut self, key: &str, value: Value) -> Self {
        self.values.insert(key.to_string(), value);
        self
    }
    
    pub fn with_capability(mut self, capability: &str) -> Self {
        self.capabilities.add(capability);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_action_execution() {
        let action = MyTestAction::new();
        let context = MockExecutionContext::new()
            .with_value("input", Value::String("test".to_string()));
        
        let params = ParameterCollection::new()
            .required_string("message", "Message");
        
        let result = action.execute(params, &context).await;
        assert!(result.is_ok());
    }
}
```

## Performance Considerations

### Allocation Strategy

- Use `Arc<str>` for immutable strings shared across threads
- Implement `Copy` for small identifier types where possible
- Use zero-cost abstractions for type safety
- Minimize allocations in hot paths

### Async Considerations

- All traits are async-compatible with `async_trait`
- Use `Pin<Box<dyn Future>>` for dynamic async operations
- Provide sync alternatives where blocking is acceptable
- Use `tokio::spawn` for CPU-intensive work

## Usage Examples

### Implementing a Simple Action

```rust
use nebula_core::*;
use serde_json::Value;
use async_trait::async_trait;

pub struct UppercaseAction;

#[async_trait]
impl Action for UppercaseAction {
    async fn execute(
        &self,
        params: ParameterCollection,
        _context: &ExecutionContext,
    ) -> Result<Value> {
        let input = params.get_required_string("input")?;
        Ok(Value::String(input.to_uppercase()))
    }
    
    fn metadata(&self) -> ActionMetadata {
        ActionMetadata {
            id: "text.uppercase".to_string(),
            name: "Uppercase Text".to_string(),
            description: "Convert text to uppercase".to_string(),
            category: "Text".to_string(),
            version: semver::Version::new(1, 0, 0),
            author: "Nebula Team".to_string(),
            icon: Some("text-uppercase".to_string()),
            color: Some("#3B82F6".to_string()),
            tags: vec!["text".to_string(), "transform".to_string()],
            documentation_url: None,
            source_url: None,
            license: Some("MIT".to_string()),
            capabilities: CapabilityRequirements {
                network_access: false,
                filesystem_access: false,
                system_commands: false,
                custom_capabilities: vec![],
            },
            resource_requirements: ResourceRequirements {
                memory_mb: Some(1),
                cpu_cores: Some(0.1),
                disk_mb: None,
                execution_timeout: Some(std::time::Duration::from_secs(1)),
            },
        }
    }
    
    fn parameter_schema(&self) -> ParameterCollection {
        ParameterCollection::new()
            .required_string("input", "Input Text")
    }
}
```

## Integration with Other Crates

### nebula-macros Integration

The `nebula-macros` crate provides procedural macros to reduce boilerplate:

```rust
use nebula_macros::{action, node};

#[derive(Parameters)]
pub struct UppercaseParams {
    #[param(required, display_name = "Input Text")]
    pub input: String,
}

#[derive(Action)]
#[action(
    id = "text.uppercase",
    name = "Uppercase Text",
    category = "Text",
    description = "Convert text to uppercase"
)]
pub struct UppercaseAction;

impl UppercaseAction {
    async fn execute(&self, params: UppercaseParams, _ctx: &ExecutionContext) -> Result<String> {
        Ok(params.input.to_uppercase())
    }
}
```

### serde / serde_json::Value

Значения в runtime — `serde_json::Value`; сериализация и параметры — через serde:

```rust
use serde_json::Value;

// Извлечение из params или контекста
let v: &Value = params.get("endpoint")?;
let url = v.as_str().ok_or_else(|| Error::expected_string("endpoint"))?;
let number = params.get("count").and_then(Value::as_i64)?;
let flag = params.get("enabled").and_then(Value::as_bool).unwrap_or(false);
```