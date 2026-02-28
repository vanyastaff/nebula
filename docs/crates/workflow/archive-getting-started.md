# Archived From "docs/archive/getting-started.md"

## Your First Workflow

### 1. Create a Simple Node
```rust
use nebula_sdk::prelude::*;

#[derive(Action, Parameters)]
#[action(
    id = "hello_world",
    name = "Hello World",
    category = "Examples"
)]
pub struct HelloWorldNode {
    #[param(label = "Name", default = "World")]
    name: String,
}

#[async_trait]
impl ExecutableNode for HelloWorldNode {
    type Output = String;
    
    async fn execute(&self, ctx: &ExecutionContext) -> Result<Self::Output> {
        Ok(format!("Hello, {}!", self.name))
    }
}
```

### 2. Register Your Node
```rust
use nebula_node_registry::NodeRegistry;

let mut registry = NodeRegistry::new();
registry.register(HelloWorldNode)?;
```

### 3. Create a Workflow
```json
{
  "id": "my-first-workflow",
  "name": "My First Workflow",
  "nodes": [
    {
      "id": "hello",
      "type": "hello_world",
      "parameters": {
        "name": "Nebula User"
      }
    }
  ]
}
```

