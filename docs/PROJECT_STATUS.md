# Project Status

**Last updated:** 2026-03-01
**Overall:** 🟡 Alpha — core crates implemented, execution engine in active development

## Component Status

### Core Layer

| Crate | Status | Notes |
|-------|--------|-------|
| `nebula-core` | ✅ Done | Identifiers, scope, shared traits |
| `nebula-workflow` | ✅ Done | Workflow definition types |
| `nebula-execution` | ✅ Done | Execution state machine |
| `nebula-memory` | ✅ Done | Arenas, LRU/TTL caching, pressure detection |
| `nebula-expression` | ✅ Done | Expression evaluation on `serde_json::Value` |
| `nebula-parameter` | ✅ Done | Parameter schema, builder API |
| `nebula-validator` | ✅ Done | Validation combinators |

### Infrastructure & Cross-Cutting

| Crate | Status | Notes |
|-------|--------|-------|
| `nebula-config` | ✅ Done | Configuration, hot-reload |
| `nebula-log` | ✅ Done | Structured logging, tracing |
| `nebula-system` | ✅ Done | Cross-platform utils, memory pressure |
| `nebula-resilience` | ✅ Done | Circuit breaker, retry, rate-limiting |
| `nebula-storage` | ✅ Done | KV storage abstraction |
| `nebula-macros` | ✅ Done | `#[node]`, `#[action]` proc-macros |
| `nebula-eventbus` | 🔄 In progress | Pub/sub event bus (planned abstraction) |
| `nebula-metrics` | 🔄 In progress | Metrics collection and export |
| `nebula-telemetry` | 🔄 In progress | Distributed tracing, observability |

### Execution Engine

| Crate | Status | Notes |
|-------|--------|-------|
| `nebula-action` | 🔄 In progress | Action trait, execution context |
| `nebula-resource` | 🔄 In progress | Lifecycle, pooling, health monitoring |
| `nebula-engine` | 🔄 In progress | DAG scheduler, workflow orchestration |
| `nebula-runtime` | 🔄 In progress | Trigger management |

### Business Logic

| Crate | Status | Notes |
|-------|--------|-------|
| `nebula-credential` | 🔄 In progress | Encrypted credential storage (AES-256-GCM) |
| `nebula-plugin` | 🔄 In progress | Plugin discovery and loading |
| `nebula-webhook` | 🔄 In progress | Inbound webhook ingestion |

### Developer Tools

| Crate | Status | Notes |
|-------|--------|-------|
| `nebula-sdk` | 🔄 In progress | All-in-one SDK, testing utilities |

### API / Application

| Crate | Status | Notes |
|-------|--------|-------|
| `nebula-api` | 🔄 In progress | REST + WebSocket (axum) |
| `nebula-ports` | 🔄 In progress | Port/adapter layer |
| `nebula-app` | ⬜ Planned | egui desktop editor |

## CI

| Check | Status |
|-------|--------|
| `cargo fmt --check` | ✅ |
| `cargo clippy -D warnings` | ✅ |
| `cargo test --workspace` | ✅ |
| `cargo doc --no-deps` | ✅ |
| `cargo audit` | ✅ |
| Miri (unsafe checks) | ✅ |
