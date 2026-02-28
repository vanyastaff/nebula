# Полная карта зависимостей крейтов Nebula

## Базовые крейты (не зависят от других Nebula крейтов)

### nebula-core
```toml
[dependencies]
uuid = "1.0"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "1.0"
```
Экспортирует базовые типы для всей системы, включая `ParamValue` для разделения expressions и литеральных значений.

### nebula-derive
```toml
[dependencies]
syn = { version = "2.0", features = ["full", "derive", "extra-traits"] }
quote = "1.0"
proc-macro2 = "1.0"
```
Процедурные макросы, используется как optional dependency везде.

## Infrastructure Layer

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

## Cross-Cutting Concerns Layer


### nebula-config
```toml
[dependencies]
nebula-core = { workspace = true }
config = "0.13"
serde = { version = "1.0", features = ["derive"] }
notify = "6.0"  # для hot-reload
tokio = { version = "1.0", features = ["fs", "sync"] }
```

### nebula-log
```toml
[dependencies]
nebula-core = { workspace = true }
tracing = "0.1"
tracing-subscriber = "0.3"
serde_json = "1.0"
chrono = "0.4"
```

### nebula-metrics
```toml
[dependencies]
nebula-core = { workspace = true }
prometheus = "0.13"
lazy_static = "1.4"
```

### nebula-system
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-metrics = { workspace = true }
sysinfo = "0.29"
tokio = { version = "1.0", features = ["time"] }
```

### nebula-locale
```toml
[dependencies]
nebula-core = { workspace = true }
fluent = "0.16"
unic-langid = "0.9"
```

### nebula-validator
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-locale = { workspace = true, optional = true }
serde_json = "1.0"
regex = "1.9"
async-trait = "0.1"
```

### nebula-resilience
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-metrics = { workspace = true }
tokio = { version = "1.0", features = ["time", "sync"] }
futures = "0.3"
```

## Core Layer

### nebula-expression
```toml
[dependencies]
nebula-core = { workspace = true }
serde_json = "1.0"
pest = "2.7"  # парсер
pest_derive = "2.7"
async-trait = "0.1"
```

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

### nebula-eventbus
```toml
[dependencies]
nebula-core = { workspace = true }
async-trait = "0.1"
tokio = { version = "1.0", features = ["sync", "rt"] }
futures = "0.3"
```

### nebula-idempotency
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-storage = { workspace = true }
async-trait = "0.1"
```

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

### nebula-execution
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-workflow = { workspace = true }
nebula-expression = { workspace = true }
nebula-memory = { workspace = true }
nebula-eventbus = { workspace = true }
nebula-log = { workspace = true }
nebula-metrics = { workspace = true }
serde_json = "1.0"
tokio = { version = "1.0", features = ["full"] }
dashmap = "5.5"
```

## Node Layer

### nebula-parameter
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-validator = { workspace = true }
nebula-expression = { workspace = true }
nebula-derive = { workspace = true, optional = true }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

### nebula-credential
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-storage = { workspace = true }
nebula-derive = { workspace = true, optional = true }
async-trait = "0.1"
ring = "0.16"  # для шифрования
base64 = "0.21"
```

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

### nebula-node
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-action = { workspace = true }
nebula-credential = { workspace = true }
nebula-parameter = { workspace = true }
semver = "1.0"
```

## Business Logic Layer

### nebula-resource
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-credential = { workspace = true }
nebula-metrics = { workspace = true }
nebula-resilience = { workspace = true }
nebula-derive = { workspace = true, optional = true }
async-trait = "0.1"
dashmap = "5.5"
```

### nebula-registry
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-node = { workspace = true }
nebula-action = { workspace = true }
nebula-workflow = { workspace = true }
nebula-storage = { workspace = true }
tantivy = "0.20"  # для поиска
semver = "1.0"
```

## Execution Layer

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

### nebula-runtime
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-action = { workspace = true }
nebula-sandbox = { workspace = true }
nebula-resource = { workspace = true }
nebula-memory = { workspace = true }
nebula-metrics = { workspace = true }
nebula-resilience = { workspace = true }
tokio = { version = "1.0", features = ["full"] }
```

### nebula-worker
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-runtime = { workspace = true }
nebula-metrics = { workspace = true }
nebula-log = { workspace = true }
tokio = { version = "1.0", features = ["full"] }
futures = "0.3"
crossbeam = "0.8"
```

### nebula-engine
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-workflow = { workspace = true }
nebula-execution = { workspace = true }
nebula-runtime = { workspace = true }
nebula-worker = { workspace = true }
nebula-resource = { workspace = true }
nebula-registry = { workspace = true }
nebula-storage = { workspace = true }
nebula-eventbus = { workspace = true }
nebula-idempotency = { workspace = true }
nebula-log = { workspace = true }
nebula-metrics = { workspace = true }
nebula-config = { workspace = true }
tokio = { version = "1.0", features = ["full"] }
```

## Multi-tenancy & Clustering Layer

### nebula-tenant
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-resource = { workspace = true }
nebula-storage = { workspace = true }
nebula-config = { workspace = true }
async-trait = "0.1"
```

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

### nebula-testing
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-workflow = { workspace = true }
nebula-execution = { workspace = true }
nebula-action = { workspace = true }
nebula-engine = { workspace = true }
mockall = "0.11"
tokio-test = "0.4"
```

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

### nebula-api
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-engine = { workspace = true }
nebula-registry = { workspace = true }
nebula-tenant = { workspace = true }
nebula-locale = { workspace = true }
axum = "0.7"
tower = "0.4"
serde_json = "1.0"
```

### nebula-cli
```toml
[dependencies]
nebula-sdk = { workspace = true }
nebula-api = { workspace = true }  # для клиента
clap = { version = "4.0", features = ["derive"] }
tokio = { version = "1.0", features = ["full"] }
serde_json = "1.0"
```

### nebula-hub
```toml
[dependencies]
nebula-core = { workspace = true }
nebula-node = { workspace = true }
nebula-registry = { workspace = true }
nebula-storage = { workspace = true }
semver = "1.0"
tar = "0.4"
flate2 = "1.0"
```

### nebula-ui
```toml
# Frontend - отдельный стек (React/Vue/etc)
# Взаимодействует через nebula-api
```

## Правила зависимостей

1. **Никакие крейты не зависят от Presentation Layer**
2. **Developer Tools зависят только от нижних слоев**
3. **Execution Layer - центр координации, использует почти все Core и Node**
4. **Cross-cutting доступны всем через optional features**
5. **nebula-derive всегда optional**