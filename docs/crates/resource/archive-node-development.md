# Archived From "docs/archive/node-development.md"

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

