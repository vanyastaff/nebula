# runtara-core — Structure Summary

## Crate count

17 workspace crates under `crates/`:

| Crate | Role |
|-------|------|
| `runtara-core` | Persistence engine: checkpoints, signals, events, durable sleep (HTTP API on :8001) |
| `runtara-environment` | Management plane: image registry, instance lifecycle, runner backends |
| `runtara-server` | Full application server: workflows, connections, auth, MCP, object model |
| `runtara-sdk` | Client library linked into compiled workflow binaries |
| `runtara-sdk-macros` | `#[resilient]` proc macro (checkpoint + retry + durable/non-durable) |
| `runtara-dsl` | Workflow DSL types, step enum, agent capability metadata |
| `runtara-workflows` | Workflow compiler: JSON DSL → Rust AST → native/WASM binary |
| `runtara-agents` | Built-in agent capability library (HTTP, CSV, transform, crypto, SFTP, XLSX, integrations) |
| `runtara-agent-macro` | `#[capability]`, `#[derive(CapabilityInput)]`, `#[derive(CapabilityOutput)]` |
| `runtara-ai` | Synchronous LLM completion abstraction (`CompletionModel` trait, OpenAI provider) |
| `runtara-connections` | Connection CRUD, AES-256-GCM encryption, OAuth2 flows, rate limiting |
| `runtara-object-store` | Schema-driven dynamic PostgreSQL object store |
| `runtara-http` | Portable HTTP client (ureq native, WASI wasi-http) |
| `runtara-workflow-stdlib` | Standard library linked into compiled workflows (agents + SDK) |
| `runtara-management-sdk` | Management SDK + `runtara-ctl` CLI binary |
| `runtara-test-harness` | Isolated test binary for agent capability execution |
| `runtara-text-parser` | Text parsing utilities for DSL schema fields |

## LOC (estimated)

~202,000 lines of Rust (all .rs files). tokei unavailable in bash environment.

## Dependency graph (high-level)

```
runtara-server
  └── runtara-environment → runtara-core
  └── runtara-connections
  └── runtara-workflows → runtara-dsl, runtara-agents, runtara-ai
  └── runtara-object-store
  
runtara-workflow-stdlib
  └── runtara-sdk → runtara-core (embedded mode)
  └── runtara-agents → runtara-agent-macro, runtara-http
  └── runtara-ai → runtara-http
```

## Top external deps

sqlx (0.8), axum (0.8), tokio (1), serde_json (1), wasmtime (CLI), opentelemetry (0.31), aes-gcm (0.10), redis (0.27), croner (2), rmcp (1.2), minijinja (2.5), inventory (0.3).

## Test count

Integration tests use testcontainers for real PostgreSQL. Key test files in `runtara-core/src/persistence/postgres.rs`, `runtara-environment/tests/`, `runtara-server/tests/`. e2e tests in `e2e/` (shell-based). `runtara-core/src/persistence/common/ops/parity_harness.rs` runs identical ops against Postgres and SQLite.
