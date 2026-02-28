# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Presentation Layer

### nebula-cli
```toml
[dependencies]
nebula-sdk = { workspace = true }
nebula-api = { workspace = true }  # для клиента
clap = { version = "4.0", features = ["derive"] }
tokio = { version = "1.0", features = ["full"] }
serde_json = "1.0"
```

