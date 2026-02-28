# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Developer Tools Layer

### nebula-sdk
```toml
[dependencies]
# Реэкспортирует публичное API
nebula-core = { workspace = true }
nebula-workflow = { workspace = true }
nebula-action = { workspace = true }
nebula-parameter = { workspace = true }
nebula-derive = { workspace = true, optional = true }
serde_json = "1.0"

[features]
default = ["derive"]
derive = ["nebula-derive"]
```

## Presentation Layer

