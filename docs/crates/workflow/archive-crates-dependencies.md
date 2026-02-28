# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Core Layer

### nebula-workflow
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-validator = { workspace = true }
nebula-derive = { workspace = true, optional = true }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
petgraph = "0.6"  # для графов workflow
```

