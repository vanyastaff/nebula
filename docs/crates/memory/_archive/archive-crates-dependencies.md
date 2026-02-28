# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Core Layer

### nebula-memory
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-storage = { workspace = true, optional = true }
serde_json = "1.0"
lru = "0.11"
dashmap = "5.5"
tokio = { version = "1.0", features = ["sync"] }
```

