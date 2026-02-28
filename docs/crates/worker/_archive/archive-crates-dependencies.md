# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Execution Layer

### nebula-worker
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-runtime = { workspace = true }
nebula-metrics = { workspace = true }
nebula-log = { workspace = true }
tokio = { version = "1.0", features = ["full"] }
futures = "0.3"
crossbeam = "0.8"
```

