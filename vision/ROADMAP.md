# Roadmap

The complete phase plan for Nebula. Each phase has a goal, a component list, and **exit criteria** that must be met before Phase N+1 starts.

**Current focus:** Phase 2 — Execution Engine. Blocker: PostgreSQL storage backend.

---

## Immediate Next Tasks (top 5)

| # | Task | Why it matters |
|---|------|----------------|
| 1 | **Storage: PostgreSQL backend** | Phase 2 blocker — engine needs persistent execution state |
| 2 | **Credential–Resource integration** | Unblocks actions that need credentials; typed `CredentialRef<C>`, rotation subscription |
| 3 | **Action context stabilization** | `ActionContext` / `TriggerContext` concrete structs; unblocks runtime, sandbox, plugin |
| 4 | **Runtime isolation routing** | Wire `IsolationLevel` → `SandboxRunner`; capability-checked context |
| 5 | **Desktop foundation** | Tauri typed IPC (`tauri-specta`), `AppError`, service layer, Zustand, TanStack Query |

---

## Phase 1 — Core Foundation ✅

**Goal:** Establish base crates that all other components depend on.

| Component | Status |
|-----------|--------|
| `nebula-core` — IDs, scope, shared traits | ✅ Done |
| `nebula-workflow` — workflow definition types | ✅ Done |
| `nebula-execution` — execution state machine | ✅ Done |
| `nebula-memory` — arenas, caching | ✅ Done |
| `nebula-expression` — expression evaluation | ✅ Done |
| `nebula-parameter` — parameter schema | ✅ Done |
| `nebula-validator` — validation combinators | ✅ Done |
| `nebula-config` — configuration, hot-reload | ✅ Done |
| `nebula-log` — structured logging | ✅ Done |
| `nebula-system` — platform utilities | ✅ Done |
| `nebula-resilience` — circuit breaker, retry | ✅ Done |
| `nebula-storage` — memory backend | ✅ Done |
| `nebula-macros` — `#[node]`, `#[action]` | ✅ Done |

---

## Phase 2 — Execution Engine 🔄

**Goal:** Working end-to-end workflow execution.

**Exit criteria (all must pass):**
- [ ] Single-node workflow executes end-to-end
- [ ] Multi-node DAG with dependencies resolves correctly
- [ ] Execution state persists to PostgreSQL
- [ ] Cancellation and timeout work correctly
- [ ] Credential–Resource integration complete (typed refs, rotation)

| Component | Status | Notes |
|-----------|--------|-------|
| `nebula-storage` — PostgreSQL backend | 🔄 In progress | Blocker for engine state |
| `nebula-action` — context model (`ActionContext`/`TriggerContext`) | 🔄 In progress | Phase 2 of action crate |
| `nebula-resource` — lifecycle contracts, credential integration | 🔄 In progress | |
| `nebula-engine` — DAG scheduler wired to storage | 🔄 Waiting on storage | |
| `nebula-runtime` — isolation routing, SandboxRunner | 🔄 In progress | SpillToBlob TODO |
| `nebula-eventbus` — consolidate event channels | 🔄 In progress | |

**Dependencies:** `nebula-storage` PostgreSQL unblocks `nebula-engine`.

---

## Phase 3 — Credential & Plugin System ⬜

**Goal:** Secure credential storage hardened to production; extensible plugin loading.

**Exit criteria:**
- [ ] Credential rotation is reliable under concurrent access
- [ ] At least two storage providers (local + PostgreSQL) pass the provider contract tests
- [ ] Plugin registry loads first-party plugins (GitHub, Telegram) without unsafe code changes
- [ ] Webhook trigger works end-to-end with credential verification

| Component | Status | Notes |
|-----------|--------|-------|
| `nebula-credential` — Phase 2 (rotation reliability) | ⬜ Planned | After Phase 2 action/resource stable |
| `nebula-credential` — Phase 3 (provider hardening) | ⬜ Planned | PostgreSQL provider |
| `nebula-plugin` — registry contract | ⬜ Planned | After action Phase 2 |
| `nebula-webhook` — trigger end-to-end | ⬜ Planned | |
| First-party plugins (GitHub, Telegram) | ⬜ Planned | |

---

## Phase 4 — Developer Experience ⬜

**Goal:** Great SDK, testing utilities, CLI, and code generation so plugin authors can be productive.

**Exit criteria:**
- [ ] `nebula-sdk` prelude is stable; no breaking changes needed after this
- [ ] `TestContext` and mock utilities allow action unit tests without a running engine
- [ ] `nebula-idempotency` provides persistent idempotency keys for the engine
- [ ] CLI: `nebula init`, `nebula build`, `nebula test` work end-to-end
- [ ] OpenAPI spec is auto-generated from `nebula-api`

| Component | Status |
|-----------|--------|
| `nebula-sdk` — stable prelude + testing utilities | ⬜ Planned |
| `nebula-idempotency` — persistent idempotency keys | ⬜ Planned |
| CLI (`nebula-cli`) | ⬜ Planned |
| OpenAPI spec generation | ⬜ Planned |
| Dev server with hot-reload | ⬜ Planned |

---

## Phase 5 — Scale & Ecosystem ⬜

**Goal:** Production-ready multi-tenant deployment, distributed execution, visual workflow editor.

**Exit criteria:**
- [ ] Multi-tenant isolation enforced at API and execution layer
- [ ] Distributed worker pool (`nebula-worker`) executes across N nodes
- [ ] Desktop app (Tauri) has a working visual workflow canvas
- [ ] Kubernetes / Docker deployment documented and tested

| Component | Status |
|-----------|--------|
| `nebula-api` — full production REST + WebSocket | 🔄 In progress |
| Desktop app (Tauri) — workflow canvas | 🔄 In progress (foundation) |
| `nebula-worker` — distributed worker pool | ⬜ Planned |
| `nebula-tenant` — multi-tenancy | ⬜ Planned |
| `nebula-cluster` — cluster coordination | ⬜ Planned |
| `nebula-locale` — localized error responses | ⬜ Planned |
| `nebula-telemetry` — full OTLP export | 🔄 In progress |
| Kubernetes / Docker deployment | ⬜ Planned |

---

## Non-Goals

- **GraphQL** — REST + WebSocket cover our use cases; no plans to add GraphQL.
- **`nebula-value`** — removed; `serde_json::Value` is used everywhere for simplicity.
- **`nebula-app` (egui)** — superseded by the Tauri desktop app at `apps/desktop`.
- **Distributed transactions** — workflow-level sagas via `TransactionalAction`; no distributed DB transactions.
