# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Infrastructure Layer

### nebula-storage
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-binary = { workspace = true }
async-trait = "0.1"
sqlx = { version = "0.7", features = ["postgres", "runtime-tokio"] }
redis = { version = "0.23", features = ["aio", "tokio-comp"] }
aws-sdk-s3 = "0.35"
```
