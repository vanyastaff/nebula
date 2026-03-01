# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Execution Layer

### nebula-engine
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-workflow = { workspace = true }
nebula-execution = { workspace = true }
nebula-runtime = { workspace = true }
nebula-worker = { workspace = true }
nebula-resource = { workspace = true }
nebula-registry = { workspace = true }
nebula-storage = { workspace = true }
nebula-eventbus = { workspace = true }
nebula-idempotency = { workspace = true }
nebula-log = { workspace = true }
nebula-metrics = { workspace = true }
nebula-config = { workspace = true }
tokio = { version = "1.0", features = ["full"] }
```

## Multi-tenancy & Clustering Layer

