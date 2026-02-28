# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Core Layer

### nebula-execution
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-workflow = { workspace = true }
nebula-expression = { workspace = true }
nebula-memory = { workspace = true }
nebula-eventbus = { workspace = true }
nebula-log = { workspace = true }
nebula-metrics = { workspace = true }
serde_json = "1.0"
tokio = { version = "1.0", features = ["full"] }
dashmap = "5.5"
```

## Node Layer

