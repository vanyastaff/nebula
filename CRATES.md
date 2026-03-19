# Nebula Workspace Crates

This document lists all 26 crates in the Nebula workspace with their paths, descriptions, and internal dependency relationships.

## Summary

| Crate | Path | Description |
|-------|------|-------------|
| `nebula-core` | `crates/core` | Core types and traits for the Nebula workflow engine |
| `nebula-eventbus` | `crates/eventbus` | Generic event distribution layer: broadcast EventBus with backpressure policy |
| `nebula-log` | `crates/log` | Fast and beautiful logging library |
| `nebula-resilience` | `crates/resilience` | Resilience patterns (retry, circuit-breaker, compose) |
| `nebula-system` | `crates/system` | Cross-platform system information and utilities |
| `nebula-validator` | `crates/validator` | Input validation framework |
| `nebula-action` | `crates/action` | Action trait and built-in action types |
| `nebula-api` | `crates/api` | Unified API server: status and workflow endpoints |
| `nebula-auth` | `crates/auth` | Authentication and authorization |
| `nebula-config` | `crates/config` | Configuration loading and validation |
| `nebula-credential` | `crates/credential` | Credential management and encryption |
| `nebula-engine` | `crates/engine` | Workflow execution engine |
| `nebula-execution` | `crates/execution` | Execution state, journals, idempotency, and planning |
| `nebula-expression` | `crates/expression` | Expression language compatible with n8n syntax |
| `nebula-macros` | `crates/macros` | Proc-macros for Nebula |
| `nebula-memory` | `crates/memory` | High-performance memory management |
| `nebula-metrics` | `crates/metrics` | Unified metric naming and export adapters |
| `nebula-parameter` | `crates/parameter` | Parameter definition system for workflow nodes |
| `nebula-plugin` | `crates/plugin` | Plugin system with versioning, registry, dynamic loading |
| `nebula-resource` | `crates/resource` | Resource management and pooling |
| `nebula-runtime` | `crates/runtime` | Action runtime and execution orchestration |
| `nebula-sdk` | `crates/sdk` | Public SDK for developers |
| `nebula-storage` | `crates/storage` | Storage abstraction (memory, Redis, Postgres, S3) |
| `nebula-telemetry` | `crates/telemetry` | Event bus, metrics, and telemetry |
| `nebula-webhook` | `crates/webhook` | Webhook server infrastructure |
| `nebula-workflow` | `crates/workflow` | Workflow definition, DAG graph, and validation |

## Dependency Details

Internal dependencies (other `nebula-*` crates) for each workspace member.

### Leaf crates (no internal deps)

- **`nebula-core`** — no internal dependencies
- **`nebula-eventbus`** — no internal dependencies
- **`nebula-log`** — no internal dependencies
- **`nebula-resilience`** — no internal dependencies
- **`nebula-system`** — no internal dependencies
- **`nebula-macros`** — no internal dependencies
- **`nebula-validator`** — no internal dependencies

### Crates with internal deps

**`nebula-action`**
Depends on: `nebula-core`, `nebula-credential`, `nebula-parameter`, `nebula-resource`

**`nebula-api`**
Depends on: `nebula-storage`, `nebula-config`, `nebula-core`, `nebula-validator`

**`nebula-auth`**
Depends on: `nebula-core`, `nebula-parameter`

**`nebula-config`**
Depends on: `nebula-log`, `nebula-validator`

**`nebula-credential`**
Depends on: `nebula-core`, `nebula-log`, `nebula-parameter`, `nebula-eventbus`

**`nebula-engine`**
Depends on: `nebula-core`, `nebula-action`, `nebula-expression`, `nebula-plugin`, `nebula-parameter`, `nebula-workflow`, `nebula-execution`, `nebula-resource`, `nebula-runtime`, `nebula-metrics`, `nebula-telemetry`

**`nebula-execution`**
Depends on: `nebula-core`, `nebula-workflow`, `nebula-action`

**`nebula-expression`**
Depends on: `nebula-core`, `nebula-log`, `nebula-memory`

**`nebula-memory`**
Depends on: `nebula-core`, `nebula-log`, `nebula-system`

**`nebula-metrics`**
Depends on: `nebula-eventbus`, `nebula-telemetry`

**`nebula-parameter`**
Depends on: `nebula-validator`

**`nebula-plugin`**
Depends on: `nebula-core`, `nebula-action`, `nebula-credential`, `nebula-resource`

**`nebula-resource`**
Depends on: `nebula-core`, `nebula-eventbus`, `nebula-metrics`, `nebula-telemetry`, `nebula-resilience`

**`nebula-runtime`**
Depends on: `nebula-core`, `nebula-action`, `nebula-plugin`, `nebula-metrics`, `nebula-telemetry`

**`nebula-sdk`**
Depends on: `nebula-core`, `nebula-action`, `nebula-workflow`, `nebula-parameter`, `nebula-credential`, `nebula-plugin`, `nebula-macros`, `nebula-validator`

**`nebula-storage`**
Depends on: `nebula-core`

**`nebula-telemetry`**
Depends on: `nebula-core`, `nebula-eventbus`

**`nebula-webhook`**
Depends on: `nebula-core`, `nebula-resource`

**`nebula-workflow`**
Depends on: `nebula-core`
