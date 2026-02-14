---
title: nebula-workflow — Overview
tags: [nebula, nebula-workflow, crate, docs, workflows, orchestration]
status: published
created: 2025-08-17
updated: 2025-11-09
---

# nebula-workflow — Overview

**nebula-workflow** provides the core workflow orchestration engine for Nebula. It enables building, executing, and managing complex workflows as directed acyclic graphs (DAGs) of actions with advanced control flow, error handling, and state management.

## What is a Workflow?

A workflow in Nebula is a **directed acyclic graph (DAG)** of nodes that execute actions in a coordinated manner. Each workflow:

- **Orchestrates** multiple actions in sequence or parallel
- **Manages** data flow between actions
- **Handles** errors and retries automatically
- **Tracks** execution state and progress
- **Supports** conditional branching and loops
- **Provides** observability through metrics and tracing

```rust
use nebula_workflow::prelude::*;

// Define a simple ETL workflow
let workflow = WorkflowBuilder::new("user_etl")
    .add_node("extract", ExtractUsersAction)
    .add_node("transform", TransformUsersAction)
    .add_node("load", LoadUsersAction)
    .add_edge("extract", "transform", |output| output.users)
    .add_edge("transform", "load", |output| output.transformed)
    .build()?;

// Execute workflow
let result = workflow.execute(trigger_data, context).await?;
```

## Why Use nebula-workflow?

### Without nebula-workflow

❌ **Manual orchestration is error-prone:**

```rust
// Manually coordinate actions - fragile and hard to maintain
async fn process_order(order: Order) -> Result<()> {
    // No automatic retry
    let validated = validate_order(order).await?;

    // No parallelization
    let inventory = check_inventory(validated).await?;

    // Manual error handling
    match charge_payment(validated).await {
        Ok(payment) => {
            ship_order(payment).await?;
        }
        Err(e) => {
            // Manual compensation
            release_inventory(inventory).await?;
            return Err(e);
        }
    }

    Ok(())
}
```

### With nebula-workflow

✅ **Declarative, reliable orchestration:**

```rust
let workflow = WorkflowBuilder::new("order_processing")
    .add_node("validate", ValidateOrderAction)
    .add_node("check_inventory", CheckInventoryAction)
    .add_node("charge_payment", ChargePaymentAction)
    .add_node("ship_order", ShipOrderAction)
    .add_node("release_inventory", ReleaseInventoryAction) // Compensation

    // Sequential flow
    .add_edge("validate", "check_inventory", |o| o)
    .add_edge("check_inventory", "charge_payment", |o| o)
    .add_edge("charge_payment", "ship_order", |o| o)

    // Automatic compensation on error
    .add_error_edge("charge_payment", "release_inventory", |_| json!({}))

    // Automatic retry with exponential backoff
    .with_retry(RetryConfig::exponential(3, Duration::from_secs(1)))

    .build()?;

// Everything handled automatically: retries, compensation, observability
workflow.execute(order, context).await?;
```

## Key Features

- 🎯 **DAG Execution** - Directed acyclic graph with automatic dependency resolution
- 🔀 **Control Flow** - Conditional branches, loops, fan-out/fan-in patterns
- ⚡ **Parallel Execution** - Automatic parallelization of independent nodes
- 🔄 **Error Handling** - Retry policies, compensation, saga pattern support
- 📊 **State Management** - Persistent workflow state with checkpointing
- 🔍 **Observability** - Built-in metrics, logging, distributed tracing
- 🎨 **Composability** - Nest workflows, reuse subworkflows
- 🔐 **Security** - Integration with [[02-Crates/nebula-credential/README|nebula-credential]]
- 🧪 **Testability** - Testing utilities and mocks

## Quick Start

### Installation

```toml
[dependencies]
nebula-workflow = "0.2"
nebula-action = "0.2"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
```

### Basic Example: Sequential Workflow

```rust
use nebula_workflow::prelude::*;
use nebula_action::prelude::*;

// Define actions
#[derive(Serialize, Deserialize)]
struct FetchDataInput {
    user_id: i64,
}

#[derive(Serialize, Deserialize)]
struct FetchDataOutput {
    user: User,
}

struct FetchDataAction;

#[async_trait]
impl ProcessAction for FetchDataAction {
    type Input = FetchDataInput;
    type Output = FetchDataOutput;

    async fn process(&self, input: Self::Input, ctx: &Context) -> Result<Self::Output> {
        let db = ctx.get_resource::<Database>().await?;
        let user = db.fetch_user(input.user_id).await?;
        Ok(FetchDataOutput { user })
    }
}

// Build workflow
#[tokio::main]
async fn main() -> Result<()> {
    let workflow = WorkflowBuilder::new("user_pipeline")
        .add_node("fetch", FetchDataAction)
        .add_node("enrich", EnrichUserAction)
        .add_node("notify", NotifyUserAction)

        // Define data flow
        .add_edge("fetch", "enrich", |output: FetchDataOutput| {
            EnrichUserInput { user: output.user }
        })
        .add_edge("enrich", "notify", |output: EnrichUserOutput| {
            NotifyUserInput { user: output.enriched_user }
        })

        .build()?;

    // Execute
    let trigger = FetchDataInput { user_id: 123 };
    let context = ExecutionContext::new();
    let result = workflow.execute(trigger, context).await?;

    println!("Workflow completed: {:?}", result);
    Ok(())
}
```

## Core Concepts

### Workflow Structure

```rust
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub nodes: HashMap<String, Node>,
    pub edges: Vec<Edge>,
    pub config: WorkflowConfig,
}

pub struct Node {
    pub id: String,
    pub action: Arc<dyn Action>,
    pub config: NodeConfig,
}

pub struct Edge {
    pub from: String,
    pub to: String,
    pub transform: Box<dyn EdgeTransform>,
    pub condition: Option<Box<dyn EdgeCondition>>,
}
```

### Execution Model

Workflows execute in phases:

```
PENDING → RUNNING → COMPLETED
              ↓
           FAILED
              ↓
        RETRYING (if configured)
              ↓
         COMPENSATING (if saga)
```

### Node States

Each node tracks its execution state:

```rust
pub enum NodeState {
    Pending,         // Waiting for dependencies
    Ready,           // Dependencies met, ready to run
    Running,         // Currently executing
    Completed,       // Finished successfully
    Failed,          // Execution failed
    Skipped,         // Skipped due to condition
    Retrying,        // Waiting to retry
    Cancelled,       // Manually cancelled
}
```

## Workflow Patterns

### 1. Sequential Execution

Actions execute one after another:

```rust
let workflow = WorkflowBuilder::new("sequential")
    .add_node("step1", Action1)
    .add_node("step2", Action2)
    .add_node("step3", Action3)
    .add_edge("step1", "step2", |o| o)
    .add_edge("step2", "step3", |o| o)
    .build()?;
```

```
step1 → step2 → step3
```

### 2. Parallel Execution

Independent actions execute concurrently:

```rust
let workflow = WorkflowBuilder::new("parallel")
    .add_node("fetch_user", FetchUserAction)
    .add_node("fetch_posts", FetchPostsAction)
    .add_node("fetch_comments", FetchCommentsAction)
    .add_node("combine", CombineAction)

    // Parallel fan-out
    .add_edge("fetch_user", "combine", |o| o)
    .add_edge("fetch_posts", "combine", |o| o)
    .add_edge("fetch_comments", "combine", |o| o)

    .build()?;
```

```
        ┌─ fetch_posts ──┐
start ──┼─ fetch_comments├─→ combine
        └─ fetch_user ───┘
```

### 3. Conditional Branching

Execute different paths based on conditions:

```rust
let workflow = WorkflowBuilder::new("conditional")
    .add_node("check_user", CheckUserAction)
    .add_node("send_welcome", SendWelcomeAction)
    .add_node("send_reminder", SendReminderAction)

    // Branch on condition
    .add_edge_with_condition(
        "check_user",
        "send_welcome",
        |output| output.is_new_user
    )
    .add_edge_with_condition(
        "check_user",
        "send_reminder",
        |output| !output.is_new_user
    )

    .build()?;
```

```
            ┌─ send_welcome (if new)
check_user ─┤
            └─ send_reminder (if existing)
```

### 4. Fan-Out/Fan-In

Process items in parallel, then aggregate:

```rust
let workflow = WorkflowBuilder::new("fan_out")
    .add_node("fetch_users", FetchUsersAction)
    .add_fanout_node("process_user", ProcessUserAction) // Parallel processing
    .add_node("aggregate", AggregateAction)

    .add_edge("fetch_users", "process_user", |o| o)
    .add_edge("process_user", "aggregate", |o| o)

    .build()?;
```

```
              ┌─ process_user[0] ─┐
fetch_users ──┼─ process_user[1] ─┼─→ aggregate
              └─ process_user[n] ─┘
```

### 5. Error Handling with Saga

Automatic compensation on failure:

```rust
let workflow = WorkflowBuilder::new("saga")
    .add_transactional_node("reserve_inventory", ReserveInventoryAction)
    .add_transactional_node("charge_payment", ChargePaymentAction)
    .add_transactional_node("create_shipment", CreateShipmentAction)

    .add_edge("reserve_inventory", "charge_payment", |o| o)
    .add_edge("charge_payment", "create_shipment", |o| o)

    // Automatic compensation in reverse order
    .with_saga_compensation()

    .build()?;
```

If `charge_payment` fails:
1. Compensate `reserve_inventory` (release inventory)
2. Workflow fails gracefully

## Advanced Features

### Subworkflows

Compose workflows from other workflows:

```rust
let data_processing_workflow = WorkflowBuilder::new("data_processing")
    .add_node("validate", ValidateAction)
    .add_node("transform", TransformAction)
    .build()?;

let main_workflow = WorkflowBuilder::new("main")
    .add_node("fetch", FetchAction)
    .add_subworkflow("process", data_processing_workflow)
    .add_node("save", SaveAction)
    .add_edge("fetch", "process", |o| o)
    .add_edge("process", "save", |o| o)
    .build()?;
```

### Dynamic Workflows

Build workflows at runtime:

```rust
let mut builder = WorkflowBuilder::new("dynamic");

for step in config.steps {
    builder = builder.add_node(&step.name, step.action);

    if let Some(prev) = previous_step {
        builder = builder.add_edge(&prev, &step.name, |o| o);
    }
    previous_step = Some(step.name);
}

let workflow = builder.build()?;
```

### Workflow Hooks

Execute code at key lifecycle points:

```rust
let workflow = WorkflowBuilder::new("hooked")
    .add_node("process", ProcessAction)

    .on_start(|ctx| {
        ctx.log_info("Workflow starting");
    })
    .on_node_start("process", |ctx| {
        ctx.record_metric("node.started", 1.0);
    })
    .on_node_complete("process", |ctx, output| {
        ctx.log_info(&format!("Node completed: {:?}", output));
    })
    .on_complete(|ctx, result| {
        ctx.log_info(&format!("Workflow completed: {:?}", result));
    })
    .on_error(|ctx, error| {
        ctx.log_error(&format!("Workflow failed: {}", error));
    })

    .build()?;
```

### Workflow Variables

Share data across nodes:

```rust
let workflow = WorkflowBuilder::new("with_vars")
    .add_variable("user_id", 123)
    .add_variable("region", "us-west")

    .add_node("fetch", FetchAction)
    .configure_node("fetch", |config| {
        config.with_context_variable("user_id");
    })

    .build()?;

// In action
impl ProcessAction for FetchAction {
    async fn process(&self, input: Self::Input, ctx: &Context) -> Result<Self::Output> {
        let user_id: i64 = ctx.get_variable("user_id")?;
        // Use user_id...
    }
}
```

### Checkpointing

Save workflow state for recovery:

```rust
let workflow = WorkflowBuilder::new("checkpointed")
    .add_node("step1", Step1Action)
    .add_node("step2", Step2Action)
    .add_node("step3", Step3Action)

    .add_edge("step1", "step2", |o| o)
    .add_edge("step2", "step3", |o| o)

    // Enable checkpointing
    .with_checkpointing(CheckpointConfig {
        enabled: true,
        interval: Duration::from_secs(30),
        storage: CheckpointStorage::Database,
    })

    .build()?;

// If workflow crashes after step2, resume from there
let result = workflow.resume(execution_id, context).await?;
```

## Configuration

### Workflow Configuration

```rust
pub struct WorkflowConfig {
    /// Maximum execution time
    pub timeout: Option<Duration>,

    /// Retry configuration
    pub retry: Option<RetryConfig>,

    /// Maximum parallel nodes
    pub max_parallel_nodes: usize,

    /// Enable checkpointing
    pub checkpointing: Option<CheckpointConfig>,

    /// Observability settings
    pub observability: ObservabilityConfig,
}

let config = WorkflowConfig {
    timeout: Some(Duration::from_secs(300)),
    retry: Some(RetryConfig::exponential(3, Duration::from_secs(1))),
    max_parallel_nodes: 10,
    checkpointing: None,
    observability: ObservabilityConfig::default(),
};

let workflow = WorkflowBuilder::new("configured")
    .with_config(config)
    .build()?;
```

### Retry Configuration

```rust
pub struct RetryConfig {
    pub max_attempts: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub backoff_factor: f64,
    pub retry_on: Vec<ErrorKind>,
}

impl RetryConfig {
    /// Exponential backoff
    pub fn exponential(max_attempts: u32, initial_delay: Duration) -> Self {
        Self {
            max_attempts,
            initial_delay,
            max_delay: Duration::from_secs(60),
            backoff_factor: 2.0,
            retry_on: vec![ErrorKind::Transient],
        }
    }

    /// Fixed delay
    pub fn fixed(max_attempts: u32, delay: Duration) -> Self {
        Self {
            max_attempts,
            initial_delay: delay,
            max_delay: delay,
            backoff_factor: 1.0,
            retry_on: vec![ErrorKind::Transient],
        }
    }
}
```

## Integration

### With nebula-action

Actions are the building blocks:

```rust
use nebula_action::prelude::*;

struct MyAction;

#[async_trait]
impl ProcessAction for MyAction {
    type Input = MyInput;
    type Output = MyOutput;

    async fn process(&self, input: Self::Input, ctx: &Context) -> Result<Self::Output> {
        // Action logic
    }
}

// Use in workflow
let workflow = WorkflowBuilder::new("example")
    .add_node("my_action", MyAction)
    .build()?;
```

### With nebula-resource

Resources are automatically managed:

```rust
let workflow = WorkflowBuilder::new("with_resources")
    .add_node("query", QueryAction)
    .configure_node("query", |config| {
        config.with_resource("database");
        config.with_resource("cache");
    })
    .build()?;

// In action
impl ProcessAction for QueryAction {
    async fn process(&self, input: Self::Input, ctx: &Context) -> Result<Self::Output> {
        let db = ctx.get_resource::<Database>().await?;
        let cache = ctx.get_resource::<Cache>().await?;
        // Use resources...
    }
}
```

### With nebula-credential

Secure credential injection:

```rust
let workflow = WorkflowBuilder::new("with_credentials")
    .add_node("api_call", ApiCallAction)
    .configure_node("api_call", |config| {
        config.with_credential("api_key");
    })
    .build()?;

// In action
impl ProcessAction for ApiCallAction {
    async fn process(&self, input: Self::Input, ctx: &Context) -> Result<Self::Output> {
        let api_key: ApiKeyCredential = ctx.get_credential("api_key").await?;
        // Use credential...
    }
}
```

## Observability

### Metrics

Workflows automatically emit metrics:

```
workflow.executions.total
workflow.executions.duration
workflow.executions.success
workflow.executions.failure

node.executions.total
node.executions.duration
node.state.transitions
```

### Tracing

Distributed tracing with OpenTelemetry:

```rust
use opentelemetry::trace::Tracer;

let workflow = WorkflowBuilder::new("traced")
    .with_tracing(TracingConfig {
        enabled: true,
        service_name: "nebula-workflow",
        endpoint: "http://jaeger:14268/api/traces",
    })
    .build()?;

// Automatic span creation:
// - workflow.execute
// - node.execute
// - edge.transform
```

### Logging

Structured logging at all levels:

```rust
impl ProcessAction for MyAction {
    async fn process(&self, input: Self::Input, ctx: &Context) -> Result<Self::Output> {
        ctx.log_info("Processing started");
        ctx.log_debug(&format!("Input: {:?}", input));

        // ... processing

        ctx.log_info("Processing completed");
        Ok(output)
    }
}
```

## Testing

### Unit Testing Workflows

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_workflow::testing::*;

    #[tokio::test]
    async fn test_workflow_execution() {
        let workflow = WorkflowBuilder::new("test")
            .add_node("action1", MockAction::new(|_| Ok(Output1 { value: 42 })))
            .add_node("action2", MockAction::new(|input: Output1| {
                Ok(Output2 { doubled: input.value * 2 })
            }))
            .add_edge("action1", "action2", |o| o)
            .build()
            .unwrap();

        let context = TestContext::new();
        let result = workflow.execute((), context).await.unwrap();

        assert_eq!(result.get_node_output::<Output2>("action2").unwrap().doubled, 84);
    }
}
```

### Integration Testing

```rust
#[tokio::test]
async fn test_workflow_with_real_resources() {
    // Setup test database
    let db = setup_test_database().await;

    let workflow = WorkflowBuilder::new("integration_test")
        .add_node("query", QueryAction)
        .build()
        .unwrap();

    let context = ExecutionContext::builder()
        .with_resource("database", db)
        .build();

    let result = workflow.execute(QueryInput { id: 1 }, context).await;

    assert!(result.is_ok());
}
```

## Best Practices

1. **Keep nodes small** - Single responsibility per node
2. **Use descriptive IDs** - Clear node and edge names
3. **Handle errors explicitly** - Define error edges and compensation
4. **Enable observability** - Metrics, logs, and tracing
5. **Test workflows** - Unit and integration tests
6. **Use subworkflows** - Break complex workflows into reusable pieces
7. **Configure timeouts** - Prevent hanging workflows
8. **Document flows** - Add comments explaining complex logic
9. **Version workflows** - Track changes over time
10. **Monitor execution** - Set up alerts for failures

## Common Patterns

See [[Examples]] for detailed implementations:

- **ETL Pipeline** - Extract, transform, load pattern
- **API Orchestration** - Coordinate multiple API calls
- **Batch Processing** - Process large datasets in chunks
- **Event-Driven Workflows** - React to events
- **Long-Running Workflows** - Handle workflows that take hours/days
- **Human-in-the-Loop** - Workflows requiring manual approval

## Related Crates

- **[[02-Crates/nebula-action/README|nebula-action]]** - Actions executed by workflows
- **[[02-Crates/nebula-resource/README|nebula-resource]]** - Resource management
- **[[02-Crates/nebula-credential/README|nebula-credential]]** - Secure credentials
- **[[02-Crates/nebula-engine/README|nebula-engine]]** - Workflow execution engine
- **[[02-Crates/nebula-expression/README|nebula-expression]]** - Expression evaluation

## Getting Help

- **Concepts**: Read [[03-Concepts/Workflows|Workflows concept]] for mental models
- **How-to**: Follow [[04-Development/Building Workflows]] for step-by-step guidance
- **Examples**: Browse [[Examples]] for real-world patterns
- **API**: See [[API Reference]] for detailed trait documentation

---

**Next**: Start with [[Examples]] or explore [[API Reference]].
