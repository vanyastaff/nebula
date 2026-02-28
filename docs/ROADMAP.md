# Roadmap

## Phase 1 — Core Foundation ✅

**Goal:** Establish the base crates that all other components depend on.

| Component | Status |
|-----------|--------|
| `nebula-core` — identifiers, scope, shared traits | Done |
| `nebula-workflow` — workflow definition types | Done |
| `nebula-execution` — execution state types | Done |
| `nebula-memory` — in-memory state and caching | Done |
| `nebula-expression` — expression evaluation | Done |
| `nebula-parameter` — parameter schema | Done |
| `nebula-validator` — validation combinators | Done |
| `nebula-config` — configuration, hot-reload | Done |
| `nebula-log` — structured logging | Done |
| `nebula-system` — platform utilities | Done |
| `nebula-resilience` — circuit breaker, retry | Done |
| `nebula-storage` — storage abstraction | Done |
| `nebula-macros` — procedural macros | Done |

## Phase 2 — Execution Engine 🔄

**Goal:** Working end-to-end execution of workflows.

| Component | Status |
|-----------|--------|
| `nebula-action` — Action trait and context | In progress |
| `nebula-resource` — resource lifecycle, pooling | In progress |
| `nebula-engine` — DAG scheduler | In progress |
| `nebula-runtime` — trigger management | In progress |
| `drivers/queue-memory` — in-process work queue | In progress |
| `drivers/sandbox-inprocess` — execution sandbox | In progress |

**Acceptance criteria:**
- [ ] Single-node workflow executes end-to-end
- [ ] Multi-node DAG with dependencies resolves correctly
- [ ] Execution state persists to PostgreSQL
- [ ] Cancellation and timeout work correctly

## Phase 3 — Credential & Plugin System ⬜

**Goal:** Secure credential storage and extensible plugin loading.

| Component | Status |
|-----------|--------|
| `nebula-credential` — encrypted secrets | Planned |
| `nebula-plugin` — plugin discovery and loading | Planned |
| `nebula-webhook` — inbound webhooks | Planned |
| First-party plugins (GitHub, Telegram) | Planned |

## Phase 4 — Developer Experience ⬜

**Goal:** Great SDK, testing utilities, and code generation.

| Component | Status |
|-----------|--------|
| `nebula-sdk` — all-in-one developer SDK | Planned |
| Testing framework — `TestContext`, mock utilities | Planned |
| CLI — `nebula init`, `nebula build`, `nebula test` | Planned |
| OpenAPI spec generation | Planned |
| Dev server with hot-reload | Planned |

## Phase 5 — API & UI ⬜

**Goal:** Production-ready REST/WebSocket API and visual workflow editor.

| Component | Status |
|-----------|--------|
| `nebula-api` — REST + WebSocket server | Planned |
| `nebula-app` — egui desktop editor | Planned |
| `nebula-ports` — port/adapter layer | Planned |
| `nebula-telemetry` — metrics and tracing | Planned |
| Kubernetes / Docker deployment | Planned |

## Non-Goals

- **GraphQL** — not planned; REST + WebSocket covers all use cases.
- **nebula-value** — removed; `serde_json::Value` is used directly everywhere.
