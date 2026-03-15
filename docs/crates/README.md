[Back to README](../../README.md)

# Crate Map

Quick reference for all crates in the Nebula workspace. See
[ARCHITECTURE.md](../ARCHITECTURE.md) for layer diagrams and dependency rules.

This index focuses on active workspace crates and their primary documentation entry points.

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
| `crates/eventbus` | `nebula-eventbus` | Cross-cutting | Pub/sub event bus |
| `crates/metrics` | `nebula-metrics` | Cross-cutting | Metrics collection and export |
| `crates/action` | `nebula-action` | Business | Action trait, execution context |
| `crates/auth` | `nebula-auth` | Business | Authentication and authorization contracts |
| `crates/resource` | `nebula-resource` | Business | Resource lifecycle and pooling |
| `crates/credential` | `nebula-credential` | Business | Encrypted credential storage |
| `crates/plugin` | `nebula-plugin` | Business | Plugin discovery and loading |
| `crates/engine` | `nebula-engine` | Execution | DAG scheduler, workflow orchestration |
| `crates/runtime` | `nebula-runtime` | Execution | Trigger management |
| `crates/sdk` | `nebula-sdk` | Dev Tools | All-in-one SDK and testing utilities |
| `crates/macros` | `nebula-macros` | Dev Tools | `#[node]`, `#[action]` proc-macros |
| `crates/api` | `nebula-api` | API/App | REST + WebSocket server (axum) |
| `crates/webhook` | `nebula-webhook` | API/App | Inbound webhook ingestion |
## Key Dependency Chains

```
nebula-core
  └── nebula-workflow, nebula-execution, nebula-parameter
        └── nebula-action
              └── nebula-engine, nebula-runtime
                    └── nebula-api
```

```
nebula-storage ◄── nebula-credential, nebula-execution
nebula-resilience ◄── nebula-resource, nebula-engine
nebula-log ◄── everything (no business logic)
```

## Per-Crate Documentation

| Crate | Docs |
|-------|------|
| `nebula-core` | [core/README.md](./core/README.md), [core/API.md](./core/API.md) |
| `nebula-workflow` | [workflow/README.md](./workflow/README.md) |
| `nebula-execution` | [execution/README.md](./execution/README.md) |
| `nebula-memory` | [memory/README.md](./memory/README.md) |
| `nebula-expression` | [expression/README.md](./expression/README.md) |
| `nebula-parameter` | [parameter/EVOLUTION_PLAN.md](./parameter/EVOLUTION_PLAN.md) |
| `nebula-validator` | [crates/validator/docs/API.md](../../crates/validator/docs/API.md) (in-tree) |
| `nebula-storage` | [storage/README.md](./storage/README.md) |
| `nebula-config` | [config/README.md](./config/README.md) |
| `nebula-log` | [crates/log/docs/README.md](../../crates/log/docs/README.md) (in-tree) |
| `nebula-system` | [system/README.md](./system/README.md) |
| `nebula-resilience` | [resilience/PLAN.md](./resilience/PLAN.md) |
| `nebula-telemetry` | [telemetry/README.md](./telemetry/README.md) |
| `nebula-eventbus` | [eventbus/README.md](./eventbus/README.md) |
| `nebula-metrics` | [metrics/README.md](./metrics/README.md) |
| `nebula-action` | [action/README.md](./action/README.md) |
| `nebula-auth` | [crates/auth/rfcs](../../crates/auth/rfcs) (in-tree) |
| `nebula-resource` | [resource/README.md](./resource/README.md), [crates/resource/docs/Architecture.md](../../crates/resource/docs/Architecture.md) (in-tree), [crates/resource/docs/API.md](../../crates/resource/docs/API.md) (in-tree) |
| `nebula-credential` | [credential/README.md](./credential/README.md), [credential/API.md](./credential/API.md) |
| `nebula-plugin` | [plugin/README.md](./plugin/README.md) |
| `nebula-engine` | [engine/README.md](./engine/README.md) |
| `nebula-runtime` | [runtime/README.md](./runtime/README.md) |
| `nebula-sdk` | [sdk/README.md](./sdk/README.md) |
| `nebula-macros` | [macros/README.md](./macros/README.md) |
| `nebula-api` | [api/README.md](./api/README.md) |
| `nebula-webhook` | [crates/webhook/README.md](../../crates/webhook/README.md) (in-tree) |

## See Also

- [Architecture](../ARCHITECTURE.md) - Workspace layering and dependency rules
- [Roadmap](../ROADMAP.md) - Cross-crate delivery phases and priorities
- [Tasks](../TASKS.md) - Ordered execution backlog

