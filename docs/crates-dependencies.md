# Nebula Crates & Dependencies

## Workspace Cargo.toml

```toml
[workspace]
resolver = "2"
members = [
  # Core
  "crates/core", "crates/workflow", "crates/execution",
  "crates/engine", "crates/runtime", "crates/action",
  "crates/expression", "crates/parameter", "crates/validator",
  "crates/telemetry", "crates/memory",
  # Ports
  "crates/ports",
  # Business
  "crates/credential", "crates/resource", "crates/registry",
  # Cross-cutting
  "crates/config", "crates/resilience", "crates/system", "crates/locale", "crates/derive",
  # Drivers
  "crates/drivers/storage-sqlite", "crates/drivers/storage-postgres",
  "crates/drivers/queue-memory", "crates/drivers/queue-redis",
  "crates/drivers/blob-fs", "crates/drivers/blob-s3",
  "crates/drivers/secrets-local",
  "crates/drivers/sandbox-inprocess", "crates/drivers/sandbox-wasm",
  # Bins
  "crates/bins/desktop", "crates/bins/server",
  "crates/bins/worker", "crates/bins/control-plane",
  # Optional
  "crates/cluster", "crates/tenant",
]

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
anyhow = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time", "sync"] }
tracing = "0.1"
tracing-subscriber = "0.3"
async-trait = "0.1"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
dashmap = "6"

[profile.release]
lto = true
codegen-units = 1
strip = true
```

---

## Core Layer (portable, без тяжёлых deps)

### core
```toml
[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
```

### workflow
```toml
[dependencies]
core = { path = "../core" }
validator = { path = "../validator" }
derive = { path = "../derive", optional = true }
serde = { workspace = true }
serde_json = { workspace = true }
petgraph = "0.6"

[features]
default = []
macros = ["derive"]
```

### execution
Включает: ExecutionContext, Journal, IdempotencyManager.
Зависит от `ports` для ExecutionRepo trait (CAS transitions + journal).
```toml
[dependencies]
core = { path = "../core" }
workflow = { path = "../workflow" }
expression = { path = "../expression" }
memory = { path = "../memory" }
ports = { path = "../ports" }        # для StateBackend, JournalStore traits
telemetry = { path = "../telemetry" }
serde_json = { workspace = true }
tokio = { workspace = true }
dashmap = { workspace = true }
async-trait = { workspace = true }
```

### engine
```toml
[dependencies]
core = { path = "../core" }
workflow = { path = "../workflow" }
execution = { path = "../execution" }
runtime = { path = "../runtime" }
ports = { path = "../ports" }         # Queue, StorageRepo traits
telemetry = { path = "../telemetry" }
resource = { path = "../resource" }
registry = { path = "../registry" }
config = { path = "../config" }
tokio = { workspace = true }
```

### runtime
```toml
[dependencies]
core = { path = "../core" }
action = { path = "../action" }
ports = { path = "../ports" }         # SandboxRunner trait
resource = { path = "../resource" }
memory = { path = "../memory" }
telemetry = { path = "../telemetry" }
resilience = { path = "../resilience" }
tokio = { workspace = true }
```

### action

Содержит: `ActionResult<T>`, `ActionError`, `ActionContext`, traits
(ProcessAction, StatefulAction, TriggerAction, DynamicActionHandler).

```toml
[dependencies]
core = { path = "../core" }
ports = { path = "../ports" }         # Capability, SandboxedContext
derive = { path = "../derive", optional = true }
serde = { workspace = true }
serde_json = { workspace = true }
async-trait = { workspace = true }
tokio-util = { workspace = true }     # CancellationToken
chrono = { workspace = true }         # DateTime<Utc> для Phase 2 types

[features]
default = []
macros = ["derive"]
```

### expression
```toml
[dependencies]
core = { path = "../core" }
serde_json = { workspace = true }
pest = "2.7"
pest_derive = "2.7"
```

### parameter
```toml
[dependencies]
core = { path = "../core" }
validator = { path = "../validator" }
derive = { path = "../derive", optional = true }
serde = { workspace = true }
serde_json = { workspace = true }

[features]
default = []
macros = ["derive"]
```

### validator
```toml
[dependencies]
core = { path = "../core" }
locale = { path = "../locale", optional = true }
serde_json = { workspace = true }
regex = "1.10"
async-trait = { workspace = true }
```

### telemetry
eventbus + logging + metrics + tracing в одном крейте.
```toml
[dependencies]
core = { path = "../core" }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
prometheus = "0.13"
async-trait = { workspace = true }
tokio = { workspace = true }
futures = "0.3"
serde_json = { workspace = true }
chrono = { workspace = true }
```

### memory
```toml
[dependencies]
core = { path = "../core" }
serde_json = { workspace = true }
lru = "0.12"
dashmap = { workspace = true }
tokio = { workspace = true }
```

---

## Ports Layer

### ports
Только traits + доменные типы. Ноль тяжёлых зависимостей.
```toml
[dependencies]
core = { path = "../core" }
action = { path = "../action" }
serde = { workspace = true }
serde_json = { workspace = true }
async-trait = { workspace = true }
tokio = { workspace = true }    # только для Duration
```

---

## Business / Node Layer

### credential
```toml
[dependencies]
core = { path = "../core" }
ports = { path = "../ports" }     # SecretsStore trait
async-trait = { workspace = true }
secrecy = "0.8"
```

### resource
```toml
[dependencies]
core = { path = "../core" }
credential = { path = "../credential" }
telemetry = { path = "../telemetry" }
resilience = { path = "../resilience" }
derive = { path = "../derive", optional = true }
async-trait = { workspace = true }
dashmap = { workspace = true }
```

### registry
```toml
[dependencies]
core = { path = "../core" }
action = { path = "../action" }
ports = { path = "../ports" }
serde = { workspace = true }
serde_json = { workspace = true }
semver = "1"
async-trait = { workspace = true }

[dependencies.tantivy]
version = "0.22"
optional = true

[features]
default = []
full-text-search = ["tantivy"]
```

---

## Cross-Cutting Concerns

### config
```toml
[dependencies]
core = { path = "../core" }
config = "0.14"
serde = { workspace = true }
notify = "6.0"
tokio = { workspace = true }
```

### resilience
```toml
[dependencies]
core = { path = "../core" }
telemetry = { path = "../telemetry" }
tokio = { workspace = true }
futures = "0.3"
```

### system
```toml
[dependencies]
core = { path = "../core" }
telemetry = { path = "../telemetry" }
sysinfo = "0.31"
tokio = { workspace = true }
```

### locale
```toml
[dependencies]
core = { path = "../core" }
fluent = "0.16"
unic-langid = "0.9"
```

### derive
```toml
[lib]
proc-macro = true

[dependencies]
syn = { version = "2", features = ["full"] }
quote = "1"
proc-macro2 = "1"
```

---

## Drivers (тяжёлые зависимости — ТОЛЬКО тут)

### drivers/storage-sqlite
```toml
[dependencies]
ports = { path = "../../ports" }
core = { path = "../../core" }
rusqlite = { version = "0.32", features = ["bundled"] }
tokio = { workspace = true }
async-trait = { workspace = true }
serde_json = { workspace = true }
```

### drivers/storage-postgres
```toml
[dependencies]
ports = { path = "../../ports" }
core = { path = "../../core" }
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono", "json"] }
async-trait = { workspace = true }
serde_json = { workspace = true }
```

### drivers/queue-memory
```toml
[dependencies]
ports = { path = "../../ports" }
core = { path = "../../core" }
tokio = { workspace = true }
async-trait = { workspace = true }
```

### drivers/queue-redis
```toml
[dependencies]
ports = { path = "../../ports" }
core = { path = "../../core" }
redis = { version = "0.27", features = ["aio", "tokio-comp"] }
tokio = { workspace = true }
async-trait = { workspace = true }
serde_json = { workspace = true }
```

### drivers/blob-fs
```toml
[dependencies]
ports = { path = "../../ports" }
core = { path = "../../core" }
tokio = { workspace = true, features = ["fs"] }
async-trait = { workspace = true }
```

### drivers/blob-s3
```toml
[dependencies]
ports = { path = "../../ports" }
core = { path = "../../core" }
aws-sdk-s3 = "1"
aws-config = "1"
tokio = { workspace = true }
async-trait = { workspace = true }
```

### drivers/secrets-local
```toml
[dependencies]
ports = { path = "../../ports" }
core = { path = "../../core" }
secrecy = "0.8"
aes-gcm = "0.10"
async-trait = { workspace = true }
serde_json = { workspace = true }
```

### drivers/sandbox-inprocess
```toml
[dependencies]
ports = { path = "../../ports" }
core = { path = "../../core" }
action = { path = "../../action" }
tokio = { workspace = true }
async-trait = { workspace = true }
```

### drivers/sandbox-wasm
```toml
[dependencies]
ports = { path = "../../ports" }
core = { path = "../../core" }
action = { path = "../../action" }
wasmtime = "25"
tokio = { workspace = true }
async-trait = { workspace = true }
serde_json = { workspace = true }
```

---

## Bins (Distribution Units)

### bins/desktop
Минимальные deps: SQLite + FS + in-memory queue + in-process sandbox.
**Не компилирует:** postgres, redis, s3, wasmtime.
```toml
[package]
name = "nebula-desktop"

[dependencies]
engine = { path = "../../engine" }
runtime = { path = "../../runtime" }
telemetry = { path = "../../telemetry" }
config = { path = "../../config" }
ports = { path = "../../ports" }

# Drivers — только лёгкие
storage-sqlite = { path = "../../drivers/storage-sqlite" }
blob-fs = { path = "../../drivers/blob-fs" }
queue-memory = { path = "../../drivers/queue-memory" }
sandbox-inprocess = { path = "../../drivers/sandbox-inprocess" }
secrets-local = { path = "../../drivers/secrets-local" }

tokio = { workspace = true }
tracing = { workspace = true }
```

### bins/server
Self-host: выбор backend'ов через features.
```toml
[package]
name = "nebula-server"

[dependencies]
engine = { path = "../../engine" }
runtime = { path = "../../runtime" }
telemetry = { path = "../../telemetry" }
config = { path = "../../config" }
ports = { path = "../../ports" }

# Всегда включены (минимум для работы)
secrets-local = { path = "../../drivers/secrets-local" }

# Optional drivers
storage-sqlite = { path = "../../drivers/storage-sqlite", optional = true }
storage-postgres = { path = "../../drivers/storage-postgres", optional = true }
blob-fs = { path = "../../drivers/blob-fs", optional = true }
blob-s3 = { path = "../../drivers/blob-s3", optional = true }
queue-memory = { path = "../../drivers/queue-memory", optional = true }
queue-redis = { path = "../../drivers/queue-redis", optional = true }
sandbox-inprocess = { path = "../../drivers/sandbox-inprocess", optional = true }
sandbox-wasm = { path = "../../drivers/sandbox-wasm", optional = true }

tokio = { workspace = true }
tracing = { workspace = true }
axum = "0.7"

[features]
default = ["lite"]
# Минимальная self-host сборка
lite = [
  "dep:storage-sqlite", "dep:blob-fs",
  "dep:queue-memory", "dep:sandbox-inprocess",
]
# Полная self-host сборка
full = [
  "dep:storage-postgres", "dep:blob-s3",
  "dep:queue-redis", "dep:sandbox-wasm",
]
```

### bins/worker
Cloud data plane: queue + sandbox + runtime.
```toml
[package]
name = "nebula-worker"

[dependencies]
runtime = { path = "../../runtime" }
telemetry = { path = "../../telemetry" }
config = { path = "../../config" }
ports = { path = "../../ports" }

queue-redis = { path = "../../drivers/queue-redis" }
sandbox-wasm = { path = "../../drivers/sandbox-wasm" }
blob-s3 = { path = "../../drivers/blob-s3" }
secrets-local = { path = "../../drivers/secrets-local" }

tokio = { workspace = true }
tracing = { workspace = true }
```

### bins/control-plane
Cloud control plane: API + scheduler + registry + storage.
```toml
[package]
name = "nebula-control-plane"

[dependencies]
engine = { path = "../../engine" }
telemetry = { path = "../../telemetry" }
config = { path = "../../config" }
ports = { path = "../../ports" }
registry = { path = "../../registry" }

storage-postgres = { path = "../../drivers/storage-postgres" }
queue-redis = { path = "../../drivers/queue-redis" }
blob-s3 = { path = "../../drivers/blob-s3" }

tokio = { workspace = true }
tracing = { workspace = true }
axum = "0.7"
```

---

## Optional / Phase 2

### cluster
> MVP: coordinator + queue + workers. Raft — phase 2.
```toml
[dependencies]
core = { path = "../core" }
engine = { path = "../engine" }
ports = { path = "../ports" }
telemetry = { path = "../telemetry" }
tonic = "0.12"

[dependencies.raft]
version = "0.7"
optional = true

[features]
default = []
raft-consensus = ["raft"]
```

### tenant
```toml
[dependencies]
core = { path = "../core" }
ports = { path = "../ports" }
resource = { path = "../resource" }
config = { path = "../config" }
async-trait = { workspace = true }
```

---

## Правила зависимостей

1. **Core → только `ports` (traits), НИКОГДА drivers**
2. **Drivers → только `ports` + `core`**
3. **Bins → core + drivers (composition root)**
4. **`derive` — всегда optional**
5. **Тяжёлые deps (sqlx, aws, redis, wasmtime) — ТОЛЬКО в drivers/**
6. **Desktop билд не компилирует postgres/redis/s3/wasmtime**
7. **`cluster` и `tenant` — optional, MVP работает без них**

## MVP: допустимые временные слияния крейтов

На ранней стадии границы ещё не устоялись. Чтобы не замедлять рефакторинг,
допускается физически держать некоторые крейты вместе (как модули одного крейта),
пока API не стабилизируется:

- `telemetry` + `resilience` + `system` → один крейт `observability` (разнести позже)
- `workflow` + `execution` → один крейт `model` (часто эволюционируют вместе)
- `parameter` + `validator` → один крейт `schema` (тесно связаны)

**Правило:** даже при слиянии — модули внутри должны иметь чистые границы
(pub mod, минимум pub API), чтобы разделение потом не ломало всё.

## Сборка по профилям

```bash
# Desktop (лёгкий)
cargo build -p nebula-desktop --release

# Self-host минимальный
cargo build -p nebula-server --release

# Self-host полный
cargo build -p nebula-server --release --no-default-features --features full

# Cloud
cargo build -p nebula-control-plane --release
cargo build -p nebula-worker --release
```
