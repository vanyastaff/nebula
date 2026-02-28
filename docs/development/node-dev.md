# Node Development Guide

## Basic Node Structure

### Simple functional node

```rust
use nebula_sdk::prelude::*;

#[node]
async fn uppercase(input: String) -> Result<String> {
    Ok(input.to_uppercase())
}
```

### Parameterized node

```rust
use nebula_sdk::prelude::*;
use std::collections::HashMap;

#[derive(Action, Parameters)]
#[action(id = "http_request", name = "HTTP Request", category = "Network")]
pub struct HttpRequestNode {
    #[param(required, label = "URL")]
    url: String,

    #[param(label = "Method", default = "GET", options = ["GET", "POST", "PUT", "DELETE", "PATCH"])]
    method: String,

    #[param(label = "Headers", optional)]
    headers: Option<HashMap<String, String>>,

    #[param(label = "Timeout (s)", min = 1, max = 300, default = 10)]
    timeout_secs: u32,
}
```

## Parameter Types

### String / text

```rust
#[param(label = "API Key", placeholder = "sk-...", validation = "min_length:10")]
api_key: String,
```

### Number

```rust
#[param(label = "Timeout", min = 1, max = 300, default = 30)]
timeout_seconds: u32,
```

### Select

```rust
#[param(label = "Region", options = ["us-east-1", "eu-west-1", "ap-south-1"], default = "us-east-1")]
region: String,
```

### Optional / nullable

```rust
#[param(label = "Body", optional)]
body: Option<serde_json::Value>,
```

## Implementing Execution

```rust
use nebula_action::ExecutionContext;
use serde_json::{json, Value};

#[async_trait]
impl ExecutableNode for HttpRequestNode {
    type Output = Value;

    async fn execute(&self, ctx: &ExecutionContext) -> Result<Self::Output> {
        let client = ctx.resource::<reqwest::Client>()?;

        let resp = client
            .request(self.method.parse()?, &self.url)
            .timeout(std::time::Duration::from_secs(self.timeout_secs as u64))
            .send()
            .await?;

        let status = resp.status().as_u16();
        let body: Value = resp.json().await.unwrap_or(Value::Null);

        Ok(json!({ "status": status, "body": body }))
    }
}
```

## Using Resources

Resources (database connections, HTTP clients, etc.) are injected via the execution context.
They are managed by `nebula-resource` and shared across node executions.

```rust
async fn execute(&self, ctx: &ExecutionContext) -> Result<Value> {
    // Typed resource access
    let db = ctx.resource::<sqlx::PgPool>()?;

    let rows = sqlx::query("SELECT id, name FROM items WHERE active = true")
        .fetch_all(db)
        .await?;

    Ok(json!(rows.len()))
}
```

## Error Handling

Define a typed error for your node:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MyNodeError {
    #[error("invalid configuration: {0}")]
    Config(String),

    #[error("external API error: {0}")]
    Api(#[from] reqwest::Error),

    #[error("parse error: {0}")]
    Parse(String),
}
```

## Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_sdk::testing::*;

    #[tokio::test]
    async fn uppercase_converts_correctly() {
        let ctx = TestContext::new();
        let result = uppercase("hello".to_string(), &ctx).await.unwrap();
        assert_eq!(result, "HELLO");
    }

    #[tokio::test]
    async fn http_node_validates_url() {
        let node = HttpRequestNode {
            url: "not-a-url".to_string(),
            method: "GET".to_string(),
            headers: None,
            timeout_secs: 10,
        };
        let ctx = TestContext::new();
        assert!(node.execute(&ctx).await.is_err());
    }
}
```

## Async and Cancellation

Long-running nodes should respect the cancellation token:

```rust
async fn execute(&self, ctx: &ExecutionContext) -> Result<Value> {
    tokio::select! {
        result = self.do_work(ctx) => result,
        _ = ctx.cancelled() => Err(Error::Cancelled),
    }
}
```

## Publishing a Plugin

1. Create a crate in `plugins/<name>/`.
2. Declare all nodes in a `plugin_manifest()` function.
3. Place the compiled `.so` / `.dll` where `nebula-plugin` can discover it.

```rust
#[no_mangle]
pub extern "C" fn plugin_manifest() -> PluginManifest {
    PluginManifest::new("my-plugin", "1.0.0")
        .node::<HttpRequestNode>()
        .node::<WebhookTriggerNode>()
}
```
