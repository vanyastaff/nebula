# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Infrastructure Layer

### nebula-binary
```toml
[dependencies]
nebula-core = { workspace = true }
serde = { version = "1.0" }
bincode = "1.3"
rmp-serde = "1.1"  # MessagePack
bytes = "1.0"
tokio = { version = "1.0", features = ["io-util"] }
```

