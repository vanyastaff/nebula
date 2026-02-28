# Getting Started

## Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | 1.93+ | Install via [rustup](https://rustup.rs) |
| PostgreSQL | 14+ | Required for persistence |
| sqlx-cli | latest | `cargo install sqlx-cli --no-default-features --features postgres` |

Kafka is optional during development — use the in-process queue driver instead.

## Setup

```bash
git clone https://github.com/vanyastaff/nebula.git
cd nebula

# Build the workspace
cargo build

# Set up the database
export DATABASE_URL="postgres://nebula:nebula@localhost:5432/nebula"
sqlx migrate run

# Run all tests
cargo test --workspace
```

## Your First Node

```rust
use nebula_sdk::prelude::*;

/// A simple node that greets the user.
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

    async fn execute(&self, _ctx: &ExecutionContext) -> Result<Self::Output> {
        Ok(format!("Hello, {}!", self.name))
    }
}
```

### Registering the node

```rust
use nebula_plugin::PluginRegistry;

let mut registry = PluginRegistry::new();
registry.register(HelloWorldNode)?;
```

### Defining a workflow (JSON)

```json
{
  "id": "hello-workflow",
  "name": "Hello Workflow",
  "nodes": [
    {
      "id": "greet",
      "type": "hello_world",
      "parameters": { "name": "Nebula" }
    }
  ]
}
```

## Development Loop

```bash
# Check everything compiles (fastest feedback)
cargo check --workspace --all-targets

# Run tests for a single crate
cargo test -p nebula-engine

# Run with output
cargo test -p nebula-parameter -- --nocapture

# Clippy (must pass in CI)
cargo clippy --workspace -- -D warnings

# Format
cargo fmt --all
```

## Pre-commit Checklist

```bash
cargo check --all-features
cargo test --workspace
cargo fmt --all
cargo clippy --all-features -- -D warnings
```

## Next Steps

- [Node Development Guide](./node-dev.md) — build production-quality nodes
- [CI/CD Guide](./cicd.md) — understand the pipeline
- [crates/sdk.md](../crates/sdk.md) — full SDK reference
