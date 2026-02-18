# nebula-sdk

Public SDK for building workflows and actions with the Nebula workflow engine.

## Overview

The `nebula-sdk` crate provides a unified public API for developers building on the Nebula platform. It re-exports core functionality from internal crates and provides convenient builders, macros, and testing utilities.

## Features

- **Prelude** - Commonly used types and traits
- **Action Builders** - Helper for creating action metadata
- **Workflow Builders** - Programmatic workflow construction
- **Testing Utilities** - Test helpers and fixtures
- **Macros** - Convenient macros for common patterns
- **Validator Derive** - Field-based validation via `#[derive(Validator)]`

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
nebula-sdk = { path = "../crates/sdk" }
```

## Quick Start

### Creating an Action

```rust
use nebula_sdk::prelude::*;

#[derive(Action)]
#[action(
    key = "example.greet",
    name = "Greet",
    description = "A simple greeting action"
)]
struct GreetAction;

#[derive(Parameters)]
struct GreetInput {
    #[param(description = "Name to greet", required)]
    name: String,
}

#[derive(Parameters)]
struct GreetOutput {
    #[param(description = "Greeting message")]
    message: String,
}

#[async_trait]
impl ProcessAction for GreetAction {
    type Input = GreetInput;
    type Output = GreetOutput;
    
    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &ActionContext,
    ) -> std::result::Result<ActionResult<Self::Output>, ActionError> {
        Ok(ActionResult::Success(GreetOutput {
            message: format!("Hello, {}!", input.name),
        }))
    }
}
```

### Building a Workflow

```rust
use nebula_sdk::workflow::WorkflowBuilder;

let workflow = WorkflowBuilder::new("data_pipeline")
    .with_description("ETL pipeline for user data")
    .add_node("extract", "550e8400-e29b-41d4-a716-446655440000")
    .add_node("transform", "550e8400-e29b-41d4-a716-446655440001")
    .add_node("load", "550e8400-e29b-41d4-a716-446655440002")
    .connect("extract", "transform")
    .connect("transform", "load")
    .build()
    .expect("valid workflow");
```

### Using the Workflow Macro

```rust
use nebula_sdk::workflow;

let workflow = workflow! {
    name: "my_workflow",
    nodes: [
        start: StartAction => process,
        process: ProcessAction => end,
        end: EndAction
    ]
};
```

### Testing

```rust
use nebula_sdk::testing::{ActionTester, assert_success};

#[tokio::test]
async fn test_greet_action() {
    let tester = ActionTester::new(GreetAction);
    let result = tester.execute(GreetInput {
        name: "World".into(),
    }).await;
    
    assert_success(&result);
}
```

## Modules

### `prelude`

Re-exports commonly used types:

```rust
use nebula_sdk::prelude::*;

// Actions
Action, ProcessAction, SimpleAction, TriggerAction, etc.

// Workflow
Workflow, WorkflowBuilder, Node, Edge

// Parameters
Parameters, ParameterDef, ParameterValue, ParameterCollection

// Macros
#[derive(Action)], #[derive(Parameters)], #[derive(Validator)], etc.
```

### `action`

Builders and helpers for action development:

```rust
use nebula_sdk::action::ActionBuilder;

let metadata = ActionBuilder::new("http.request", "HTTP Request")
    .with_description("Makes HTTP requests")
    .with_version(2, 0)
    .with_capability(Capability::Network)
    .with_isolation(IsolationLevel::Sandbox)
    .build();
```

### `workflow`

Builders for constructing workflows:

```rust
use nebula_sdk::workflow::{WorkflowBuilder, NodeBuilder};

let workflow = WorkflowBuilder::new("my_flow")
    .add_node_with_inputs(
        "process",
        "action.process",
        params! {
            "input1" => "value1",
            "input2" => 42
        }
    )
    .with_node_position("process", 100.0, 200.0)
    .connect("start", "process")
    .build()?;
```

### `testing`

Testing utilities:

```rust
use nebula_sdk::testing::{TestContext, ActionTester, fixtures};

let mut ctx = TestContext::new();
ctx.log("Test started");
ctx.record_metric("duration", 100.0);
ctx.set_variable("key", "value");

let exec_id = fixtures::execution_id();
```

## Macros

### `params!`

Create parameter values:

```rust
use nebula_sdk::params;

let values = params! {
    "name" => "test",
    "count" => 42,
    "enabled" => true
};
```

### `json!`

Create JSON values (re-exported from `serde_json`):

```rust
use nebula_sdk::json;

let data = json!({
    "name": "test",
    "items": [1, 2, 3]
});
```

### `workflow!`

Define workflows declaratively:

```rust
use nebula_sdk::workflow;

let wf = workflow! {
    name: "pipeline",
    nodes: [
        extract: ExtractAction => transform,
        transform: TransformAction => load,
        load: LoadAction
    ]
};
```

## License

MIT OR Apache-2.0
