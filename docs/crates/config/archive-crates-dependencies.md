# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Cross-Cutting Concerns Layer

### nebula-config
```toml
[dependencies]
nebula-core = { workspace = true }
config = "0.13"
serde = { version = "1.0", features = ["derive"] }
notify = "6.0"  # для hot-reload
tokio = { version = "1.0", features = ["fs", "sync"] }
```

