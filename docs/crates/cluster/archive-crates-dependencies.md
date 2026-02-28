# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Multi-tenancy & Clustering Layer

### nebula-cluster
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-engine = { workspace = true }
nebula-worker = { workspace = true }
nebula-storage = { workspace = true }
nebula-eventbus = { workspace = true }
nebula-metrics = { workspace = true }
raft = "0.7"
tonic = "0.10"  # для gRPC
prost = "0.12"
```

## Developer Tools Layer

