# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Cross-Cutting Concerns Layer

### nebula-resilience
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-metrics = { workspace = true }
tokio = { version = "1.0", features = ["time", "sync"] }
futures = "0.3"
```

## Core Layer

