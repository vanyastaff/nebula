# Archived From "docs/archive/nebula-all-docs.md"

## FILE: docs/guides/node-development.md
---

# Node Development Guide

This guide covers everything you need to know about developing custom nodes for Nebula.

## Basic Node Structure

### Simple Function Node
```rust
use nebula_sdk::prelude::*;

#[node]
async fn uppercase(input: String) -> Result<String> {
    Ok(input.to_uppercase())
}
```

### Parameterized Node
```rust
#[derive(Action, Parameters)]
pub struct HttpRequestNode {
    #[param(required, label = "URL")]
    url: String,
    
    #[param(
        label = "Method",
        default = "GET",
        options = ["GET", "POST", "PUT", "DELETE"]
    )]
    method: String,
    
    #[param(label = "Headers", optional)]
    headers: HashMap<String, String>,
}
```

## Parameter Types

### Text Parameters
```rust
#[param(
    type = "text",
    label = "API Key",
    placeholder = "Enter your API key",
    validation = "min_length:10"
)]
api_key: String,
```

### Number Parameters
```rust
#[param(
    type = "number",
    label = "Timeout",
    min = 1,
    max = 300,
    default = 30
)]
timeout_seconds: u32,
```

### Select Parameters
```rust
#[param(
    type = "select",
    label = "Region",
    options = ["us-east-1", "eu-west-1", "ap-south-1"],
    default = "us-east-1"
)]
region: String,
```

## Advanced Features

### Using Resources
```rust
impl ExecutableNode for DatabaseQueryNode {
    async fn execute(&self, ctx: &ExecutionContext) -> Result<Value> {
        // Get database connection from pool
        let db = ctx.resource_pool()
            .get::<DatabaseConnection>()
            .await?;
            
        let result = sqlx::query(&self.query)
            .fetch_all(&db)
            .await?;
            
        Ok(json!(result))
    }
}
```

### Error Handling
```rust
#[derive(Debug, thiserror::Error)]
pub enum MyNodeError {
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
    
    #[error("External API error: {0}")]
    ApiError(#[from] reqwest::Error),
}
```

### Testing Your Node
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_sdk::testing::*;
    
    #[tokio::test]
    async fn test_my_node() {
        let ctx = MockContext::new();
        let node = MyNode { param: "test".into() };
        
        let result = node.execute(&ctx).await;
        assert!(result.is_ok());
    }
}
```

## Best Practices

1. **Keep nodes focused** - Each node should do one thing well
2. **Use descriptive names** - Both for the node and its parameters
3. **Handle errors gracefully** - Provide helpful error messages
4. **Document your node** - Include examples and edge cases
5. **Test thoroughly** - Include unit and integration tests

---

