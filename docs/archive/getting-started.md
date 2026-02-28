# Getting Started with Nebula

Welcome to Nebula! This guide will help you get started with creating your first workflow.

## Installation

### Prerequisites
- Rust 1.93 or higher
- PostgreSQL 14+
- Kafka (optional for development)

### Clone and Build
```bash
git clone https://github.com/yourusername/nebula.git
cd nebula
cargo build --release
```

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

## Next Steps
- Read the [Node Development Guide](./node-development.md)
- Explore [Standard Nodes](../crates/)
- Join our [Community](https://github.com/yourusername/nebula/discussions)

