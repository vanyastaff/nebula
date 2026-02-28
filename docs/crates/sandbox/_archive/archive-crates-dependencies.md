# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Execution Layer

### nebula-sandbox
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-action = { workspace = true }
nebula-credential = { workspace = true }
nebula-resilience = { workspace = true }
nebula-log = { workspace = true }
async-trait = "0.1"
tokio = { version = "1.0", features = ["time", "sync"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

[dependencies.wasmtime]
version = "17.0"
optional = true

[features]
default = ["in-process"]
in-process = []
wasm = ["wasmtime"]
```

