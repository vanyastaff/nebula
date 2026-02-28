# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Node Layer

### nebula-action
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-parameter = { workspace = true }
nebula-credential = { workspace = true, optional = true }
nebula-derive = { workspace = true, optional = true }
async-trait = "0.1"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

