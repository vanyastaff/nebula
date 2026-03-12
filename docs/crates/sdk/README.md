# nebula-sdk

All-in-one developer toolkit for building custom nodes. Re-exports the most commonly needed
types and adds builder APIs, testing utilities, and code generation on top.

## Prelude

```rust
use nebula_sdk::prelude::*;
// Brings in:
//   nebula_core::prelude::*   (identifiers, traits, metadata)
//   serde_json::Value
//   nebula_macros::{node, action}
//   NodeBuilder, TriggerBuilder
//   TestContext, MockExecution
```

## Proc-macro path (via `nebula-macros`)

The quickest way to define a node:

```rust
use nebula_sdk::prelude::*;

/// A node built with derive macros.
#[derive(Action, Parameters)]
#[action(
    id   = "text.uppercase",
    name = "Uppercase",
    category = "Text",
)]
pub struct UppercaseNode {
    #[param(required, label = "Input text")]
    pub input: String,
}

#[async_trait]
impl ExecutableNode for UppercaseNode {
    type Output = String;

    async fn execute(&self, _ctx: &ActionContext) -> Result<Self::Output> {
        Ok(self.input.to_uppercase())
    }
}
```

## Builder API

Use when you need runtime-computed metadata or cannot use derive macros.

```rust
use nebula_sdk::builders::*;

let node = NodeBuilder::new("http.request")
    .name("HTTP Request")
    .description("Send an HTTP request and return the response.")
    .category("Network")
    .parameter(
        ParameterBuilder::new("url")
            .label("URL")
            .required(true)
            .kind(ParameterKind::String),
    )
    .parameter(
        ParameterBuilder::new("method")
            .label("Method")
            .required(false)
            .default(json!("GET"))
            .options(["GET", "POST", "PUT", "DELETE", "PATCH"]),
    )
    .implement(|input, ctx| async move {
        let url    = input.get_required_str("url")?;
        let method = input.get_str("method").unwrap_or("GET");
        // ...
        Ok(json!({ "status": 200 }))
    })
    .build()?;
```

## Testing Utilities

```rust
use nebula_sdk::testing::*;

#[tokio::test]
async fn test_uppercase_node() {
    let node = UppercaseNode { input: "hello".into() };

    let ctx = TestContext::builder()
        .resource(MockHttpClient::new())
        .credential("my_cred", MockToken::new("test-token"))
        .build();

    let result = node.execute(&ctx).await.unwrap();
    assert_eq!(result, "HELLO");
}

#[tokio::test]
async fn test_node_with_real_execution() {
    let harness = ExecutionHarness::new()
        .with_node(UppercaseNode { input: "hello".into() })
        .build();

    let output = harness.run().await.unwrap();
    assert_eq!(output.as_str().unwrap(), "HELLO");
}
```

### `MockExecution`

```rust
let mock = MockExecution::builder()
    .node_output("upstream_node", json!({ "email": "user@example.com" }))
    .variable("base_url", json!("https://api.example.com"))
    .build();

// ctx.node_output("upstream_node") returns the mocked value
let result = my_node.execute(&mock.context()).await?;
```

## Module Structure

```
nebula-sdk/src/
├── lib.rs           Re-exports and feature flags
├── prelude.rs       Everything a node author needs
├── builders/
│   ├── node.rs      NodeBuilder
│   ├── parameter.rs ParameterBuilder
│   ├── workflow.rs  WorkflowBuilder (testing / CLI)
│   └── trigger.rs   TriggerBuilder
└── testing/
    ├── context.rs   TestContext + builder
    ├── mock.rs      MockExecution, MockResource, MockToken
    ├── harness.rs   ExecutionHarness
    └── assertions.rs assert_node_output!, assert_node_error!
```

## Crate Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `testing` | ✅ | Testing utilities |
| `builders` | ✅ | Builder APIs |
| `codegen` | ❌ | OpenAPI spec + type generators |
| `dev-server` | ❌ | Hot-reload dev server |

Disable `testing` and `builders` in release builds to reduce binary size:

```toml
[dependencies]
nebula-sdk = { version = "0.1", default-features = false }
```

## Document map

- [ARCHITECTURE.md](./ARCHITECTURE.md) — problem, current/target architecture
- [API.md](./API.md) — public surface, prelude, compatibility
- [ROADMAP.md](./ROADMAP.md) — phases, risks, exit criteria
- [MIGRATION.md](./MIGRATION.md) — versioning, breaking changes

