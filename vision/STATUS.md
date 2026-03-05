# Project Status

**Last updated:** 2026-03-05
**Overall:** 🟡 Alpha — core foundation complete, execution engine in active development

---

## Summary

| Phase | Name | State |
|-------|------|-------|
| 1 | Core Foundation | ✅ Complete |
| 2 | Execution Engine | 🔄 Active (blocked on PostgreSQL storage) |
| 3 | Credential & Plugin System | 🔄 In progress (credential Phase 1) |
| 4 | Developer Experience | ⬜ Planned |
| 5 | API & UI | 🔄 Partially started |

---

## Core Layer

| Crate | Status | Notes |
|-------|--------|-------|
| `nebula-core` | ✅ Done | IDs, scope hierarchy, shared traits — stable |
| `nebula-workflow` | ✅ Done | Workflow definition types, DAG graph model |
| `nebula-execution` | ✅ Done | Execution state machine, transition types |
| `nebula-memory` | ✅ Done | Arenas, LRU/TTL caching, memory pressure |
| `nebula-expression` | ✅ Done | Expression evaluation on `serde_json::Value` |
| `nebula-parameter` | ✅ Done | Parameter schema, builder API |
| `nebula-validator` | ✅ Done | Validation combinator library |

---

## Infrastructure & Cross-Cutting

| Crate | Status | Notes |
|-------|--------|-------|
| `nebula-config` | ✅ Done | Configuration, hot-reload |
| `nebula-log` | ✅ Done | Structured logging, tracing spans |
| `nebula-system` | ✅ Done | Cross-platform utils, memory pressure |
| `nebula-resilience` | ✅ Done | Circuit breaker, retry, rate-limiting |
| `nebula-storage` | 🔄 In progress | Memory backend done; **PostgreSQL backend is the Phase 2 blocker** |
| `nebula-macros` | ✅ Done | `#[node]`, `#[action]` proc-macros |
| `nebula-eventbus` | 🔄 In progress | Pub/sub bus design complete; implementation in progress |
| `nebula-metrics` | 🔄 In progress | Phases 1–3 done; OTLP export (Phase 4) remaining |
| `nebula-telemetry` | 🔄 In progress | Distributed tracing foundation; full OTLP integration pending |

---

## Execution Engine

| Crate | Status | Notes |
|-------|--------|-------|
| `nebula-action` | 🔄 In progress | Core traits stable; context model (`ActionContext`/`TriggerContext`) in Phase 2 |
| `nebula-resource` | 🔄 In progress | Lifecycle and pooling contracts; credential integration in progress |
| `nebula-resource-postgres` | 🔄 In progress | Reference PostgreSQL adapter; early stage |
| `nebula-engine` | 🔄 In progress | DAG scheduler exists; **waiting on storage PostgreSQL backend** |
| `nebula-runtime` | 🔄 In progress | ActionRuntime, ActionRegistry, DataPassingPolicy done; sandbox routing TODO |

**Known gaps in execution engine:**
- `InProcessSandbox` runs all actions directly — no capability checks yet
- `LargeDataStrategy::SpillToBlob` logs warning but does not spill; only `Reject` works
- `max_total_execution_bytes` defined but not enforced cross-node

---

## Business Logic

| Crate | Status | Notes |
|-------|--------|-------|
| `nebula-credential` | 🔄 In progress | AES-256-GCM crypto done; rotation engine, provider hardening in Phase 2+ |
| `nebula-plugin` | 🔄 In progress | Discovery and loading contract; early stage |
| `nebula-webhook` | 🔄 In progress | Inbound ingestion; basic routing done |

---

## Developer Tools

| Crate | Status | Notes |
|-------|--------|-------|
| `nebula-sdk` | 🔄 In progress | Prelude and re-exports; testing utilities planned |

---

## API / Application

| Crate | Status | Notes |
|-------|--------|-------|
| `nebula-api` | 🔄 In progress | Axum server foundation; workflow + execution REST endpoints in Phase 1 |
| Desktop app (Tauri) | 🔄 In progress | `apps/desktop` — React + TypeScript + Rust IPC; foundation phase |
| Web app | 🔄 In progress | `apps/web` — browser version; early stage |

---

## CI

| Check | Status |
|-------|--------|
| `cargo fmt --check` | ✅ |
| `cargo clippy -D warnings` | ✅ |
| `cargo test --workspace` | ✅ |
| `cargo doc --no-deps` | ✅ |
| `cargo audit` | ✅ |
| Miri (unsafe checks) | ✅ |

---

## Phase 2 Acceptance Criteria (blockers before Phase 3)

- [ ] Single-node workflow executes end-to-end
- [ ] Multi-node DAG with dependencies resolves correctly
- [ ] Execution state persists to PostgreSQL
- [ ] Cancellation and timeout work correctly
- [ ] Credential–resource integration wired (typed `CredentialRef<C>`, rotation subscription)
