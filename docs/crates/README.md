# Crate Map

Quick reference for all crates in the Nebula workspace. See
[ARCHITECTURE.md](../ARCHITECTURE.md) for layer diagrams and dependency rules.

## Directory → Package mapping

| Directory | Package | Layer | Description |
|-----------|---------|-------|-------------|
| `crates/core` | `nebula-core` | Core | Identifiers, scope, shared traits |
| `crates/workflow` | `nebula-workflow` | Core | Workflow definition, graph model |
| `crates/execution` | `nebula-execution` | Core | Execution state machine |
| `crates/memory` | `nebula-memory` | Core | In-memory state, arenas, caching |
| `crates/expression` | `nebula-expression` | Core | Expression evaluation |
| `crates/parameter` | `nebula-parameter` | Core | Parameter schema and builder API |
| `crates/validator` | `nebula-validator` | Core | Validation combinators |
| `crates/storage` | `nebula-storage` | Infrastructure | KV storage abstraction |
| `crates/config` | `nebula-config` | Cross-cutting | Configuration, hot-reload |
| `crates/log` | `nebula-log` | Cross-cutting | Structured logging, tracing |
| `crates/system` | `nebula-system` | Cross-cutting | Platform utils, memory pressure |
| `crates/resilience` | `nebula-resilience` | Cross-cutting | Circuit breaker, retry, rate-limiting |
| `crates/telemetry` | `nebula-telemetry` | Cross-cutting | Metrics, distributed tracing |
| `crates/action` | `nebula-action` | Business | Action trait, execution context |
| `crates/resource` | `nebula-resource` | Business | Resource lifecycle and pooling |
| `crates/credential` | `nebula-credential` | Business | Encrypted credential storage |
| `crates/plugin` | `nebula-plugin` | Business | Plugin discovery and loading |
| `crates/engine` | `nebula-engine` | Execution | DAG scheduler, workflow orchestration |
| `crates/runtime` | `nebula-runtime` | Execution | Trigger management |
| `crates/drivers/queue-memory` | `nebula-queue-memory` | Execution | In-process work queue |
| `crates/drivers/sandbox-inprocess` | `nebula-sandbox-inprocess` | Execution | In-process Action sandbox |
| `crates/sdk` | `nebula-sdk` | Dev Tools | All-in-one SDK and testing utilities |
| `crates/macros` | `nebula-macros` | Dev Tools | `#[node]`, `#[action]` proc-macros |
| `crates/api` | `nebula-api` | API/App | REST + WebSocket server (axum) |
| `crates/app` | `nebula-app` | API/App | egui desktop application |
| `crates/webhook` | `nebula-webhook` | API/App | Inbound webhook ingestion |
| `crates/ports` | `nebula-ports` | API/App | Port/adapter abstractions |
| `plugins/github` | `nebula-plugin-github` | Plugin | GitHub integration |
| `plugins/telegram` | `nebula-plugin-telegram` | Plugin | Telegram integration |

## Key Dependency Chains

```
nebula-core
  └── nebula-workflow, nebula-execution, nebula-parameter
        └── nebula-action
              └── nebula-engine, nebula-runtime
                    └── nebula-api, nebula-app
```

```
nebula-storage ◄── nebula-credential, nebula-execution
nebula-resilience ◄── nebula-resource, nebula-engine
nebula-log ◄── everything (no business logic)
```

## Per-Crate Documentation

- [core.md](./core.md) — `nebula-core`
- [execution/README.md](./execution/README.md) — `nebula-execution`
- [action/README.md](./action/README.md) — `nebula-action`
- [parameter/README.md](./parameter/README.md) — `nebula-parameter`
- [resource/README.md](./resource/README.md) — `nebula-resource`
- [../../crates/log/docs/README.md](../../crates/log/docs/README.md) — `nebula-log` (internal docs)
- [credential.md](./credential.md) — `nebula-credential`
- [sdk/README.md](./sdk/README.md) — `nebula-sdk`
- [validator/README.md](./validator/README.md) — `nebula-validator`

