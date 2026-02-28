# Archived From "docs/archive/crates-architecture.md"

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

---

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

---

## Next Steps

1. **Implement Core Types** - Start with `nebula-core`
2. **Build Derive Macros** - Create the parameter system
3. **Create Basic Nodes** - HTTP, Transform, Log nodes
4. **Implement Runtime** - Basic execution engine
5. **Add Storage** - PostgreSQL backend
6. **Build UI** - Basic workflow editor
7. **Test Integration** - End-to-end tests
8. **Add Advanced Features** - Streaming, distributed execution

