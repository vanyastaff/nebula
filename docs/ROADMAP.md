[← Previous Page](PROJECT_STATUS.md) · [Back to README](../README.md) · [Next Page →](TASKS.md)

# Roadmap

High-level phase plan for Nebula. **Crate-level detail** lives in each crate’s ROADMAP under `docs/crates/<crate>/ROADMAP.md` — see [Crate-level roadmaps](#crate-level-roadmaps) below.

**Current focus:** Phase 2 (Execution Engine) and credential–resource integration. See [Recommended next tasks](#recommended-next-tasks).

---

## Recommended next tasks

Based on current priorities in `PROJECT_STATUS.md` and crate ROADMAPs, the most concrete next steps:

| Priority | Task | Source | Notes |
|----------|------|--------|--------|
| **1** | **Credential–Resource integration** | [credential/ROADMAP.md](crates/credential/ROADMAP.md) | Typed `CredentialRef<C>`, `RotationStrategy`, `HasResourceComponents`, rotation subscription. Replaces broken `TypeId`-based refs and `credentials.rs` pull model. |
| **2** | **Credential Phase 1 exit** | [credential/ROADMAP.md](crates/credential/ROADMAP.md) | Contract consolidation: ARCHITECTURE/API/INTERACTIONS aligned with code, stable API surface, scope enforcement documented and tested. |
| **3** | **Storage Postgres backend** | [storage/ROADMAP.md](crates/storage/ROADMAP.md) | `PostgresStorage`, KV table, sqlx pool, feature `postgres`. Required for execution state and credential Postgres provider ([POSTGRES_STORAGE_SPEC](crates/credential/POSTGRES_STORAGE_SPEC.md)). |
| **4** | **Runtime Phase 1** | [runtime/ROADMAP.md](crates/runtime/ROADMAP.md) | Isolation level routing, SandboxRunner, SpillToBlob, `max_total_execution_bytes`. |
| **5** | **Desktop foundation** | [desktop/ROADMAP.md](crates/desktop/ROADMAP.md) | Tauri typed IPC (tauri-specta), AppError, service layer, Zustand, TanStack Query. |

**Phase 2 acceptance criteria** (from main roadmap): single-node workflow end-to-end, multi-node DAG resolution, execution state in PostgreSQL, cancellation and timeout working.

---

## Phase 1 — Core Foundation ✅

**Goal:** Establish the base crates that all other components depend on.

| Component | Status | Crate ROADMAP |
|-----------|--------|----------------|
| `nebula-core` — identifiers, scope, shared traits | Done | [core/ROADMAP.md](crates/core/ROADMAP.md) |
| `nebula-workflow` — workflow definition types | Done | [workflow/ROADMAP.md](crates/workflow/ROADMAP.md) |
| `nebula-execution` — execution state types | Done | [execution/ROADMAP.md](crates/execution/ROADMAP.md) |
| `nebula-memory` — in-memory state and caching | Done | [memory/ROADMAP.md](crates/memory/ROADMAP.md) |
| `nebula-expression` — expression evaluation | Done | [expression/ROADMAP.md](crates/expression/ROADMAP.md) |
| `nebula-parameter` — parameter schema | Done | [parameter/EVOLUTION_PLAN.md](crates/parameter/EVOLUTION_PLAN.md) |
| `nebula-validator` — validation combinators | Done | [validator docs roadmap](../crates/validator/docs/ROADMAP.md) |
| `nebula-config` — configuration, hot-reload | Done | [config/ROADMAP.md](crates/config/ROADMAP.md) |
| `nebula-log` — structured logging | Done | — |
| `nebula-system` — platform utilities | Done | [system/ROADMAP.md](crates/system/ROADMAP.md) |
| `nebula-resilience` — circuit breaker, retry | Done | [resilience/PLAN.md](crates/resilience/PLAN.md) |
| `nebula-storage` — storage abstraction | Done | [storage/ROADMAP.md](crates/storage/ROADMAP.md) |
| `nebula-macros` — procedural macros | Done | [macros/ROADMAP.md](crates/macros/ROADMAP.md) |

---

## Phase 2 — Execution Engine 🔄

**Goal:** Working end-to-end execution of workflows.

| Component | Status | Crate ROADMAP |
|-----------|--------|----------------|
| `nebula-action` — Action trait and context | In progress | [action/ROADMAP.md](crates/action/ROADMAP.md) |
| `nebula-resource` — resource lifecycle, pooling | In progress | [resource/ROADMAP.md](crates/resource/ROADMAP.md) |
| `nebula-engine` — DAG scheduler | In progress | [engine/ROADMAP.md](crates/engine/ROADMAP.md) |
| `nebula-runtime` — trigger management | In progress | [runtime/ROADMAP.md](crates/runtime/ROADMAP.md) |
| `nebula-runtime` — task queue + in-process sandbox | In progress | [runtime/ROADMAP.md](crates/runtime/ROADMAP.md) |

**Acceptance criteria:**
- [ ] Single-node workflow executes end-to-end
- [ ] Multi-node DAG with dependencies resolves correctly
- [ ] Execution state persists to PostgreSQL
- [ ] Cancellation and timeout work correctly

**Dependencies:** Storage Postgres backend (Phase 1 of [storage/ROADMAP.md](crates/storage/ROADMAP.md)); credential–resource integration unblocks actions that need credentials and resources.

---

## Phase 3 — Credential & Plugin System ⬜

**Goal:** Secure credential storage and extensible plugin loading.

| Component | Status | Crate ROADMAP |
|-----------|--------|----------------|
| `nebula-credential` — encrypted secrets, rotation | In progress | [credential/ROADMAP.md](crates/credential/ROADMAP.md) (8 phases to v1.0) |
| `nebula-plugin` — plugin discovery and loading | In progress | [plugin/ROADMAP.md](crates/plugin/ROADMAP.md) |
| `nebula-webhook` — inbound webhooks | In progress | — |
| First-party plugins (GitHub, Telegram) | Planned | — |

**Note:** Credential crate has a long-horizon roadmap (Contract consolidation → Rotation reliability → Provider hardening → Production infra → Security → Performance → Protocols → Toolchain). Current priority: Phase 1 exit + credential–resource integration.

---

## Phase 4 — Developer Experience ⬜

**Goal:** Great SDK, testing utilities, and code generation.

| Component | Status | Crate ROADMAP |
|-----------|--------|----------------|
| `nebula-sdk` — all-in-one developer SDK | In progress | [sdk/ROADMAP.md](crates/sdk/ROADMAP.md) |
| Testing framework — `TestContext`, mock utilities | Planned | — |
| CLI — `nebula init`, `nebula build`, `nebula test` | Planned | — |
| OpenAPI spec generation | Planned | — |
| Dev server with hot-reload | Planned | — |

---

## Phase 5 — API & UI ⬜

**Goal:** Production-ready REST/WebSocket API and visual workflow editor.

| Component | Status | Crate ROADMAP |
|-----------|--------|----------------|
| `nebula-api` — REST + WebSocket server | In progress | [api/ROADMAP.md](crates/api/ROADMAP.md) |
| **Desktop app (Tauri)** — `apps/desktop` | In progress | [desktop/ROADMAP.md](crates/desktop/ROADMAP.md) |
| `nebula-telemetry` — metrics and tracing | In progress | [telemetry/ROADMAP.md](crates/telemetry/ROADMAP.md), [metrics/ROADMAP.md](crates/metrics/ROADMAP.md) |
| Kubernetes / Docker deployment | Planned | — |

**Desktop:** The desktop client is the **Tauri app in `apps/desktop`** (React + TypeScript frontend, Rust backend). Not `nebula-app` (egui). Phases aligned with backend and tracked in the desktop roadmap and tasks files.

---

## Crate-level roadmaps

Each crate’s ROADMAP holds phased deliverables, risks, exit criteria, and readiness metrics. Use these for “what to do next” inside a crate.

| Crate | Path |
|-------|------|
| action | [docs/crates/action/ROADMAP.md](crates/action/ROADMAP.md) |
| api | [docs/crates/api/ROADMAP.md](crates/api/ROADMAP.md) |
| config | [docs/crates/config/ROADMAP.md](crates/config/ROADMAP.md) |
| core | [docs/crates/core/ROADMAP.md](crates/core/ROADMAP.md) |
| credential | [docs/crates/credential/ROADMAP.md](crates/credential/ROADMAP.md) |
| engine | [docs/crates/engine/ROADMAP.md](crates/engine/ROADMAP.md) |
| eventbus | [docs/crates/eventbus/ROADMAP.md](crates/eventbus/ROADMAP.md) |
| execution | [docs/crates/execution/ROADMAP.md](crates/execution/ROADMAP.md) |
| expression | [docs/crates/expression/ROADMAP.md](crates/expression/ROADMAP.md) |
| idempotency | [docs/crates/idempotency/ROADMAP.md](crates/idempotency/ROADMAP.md) |
| memory | [docs/crates/memory/ROADMAP.md](crates/memory/ROADMAP.md) |
| macros | [docs/crates/macros/ROADMAP.md](crates/macros/ROADMAP.md) |
| metrics | [docs/crates/metrics/ROADMAP.md](crates/metrics/ROADMAP.md) |
| parameter | [docs/crates/parameter/EVOLUTION_PLAN.md](crates/parameter/EVOLUTION_PLAN.md) |
| plugin | [docs/crates/plugin/ROADMAP.md](crates/plugin/ROADMAP.md) |
| resource | [docs/crates/resource/ROADMAP.md](crates/resource/ROADMAP.md) |
| resilience | [crates/resilience/docs/README.md](../crates/resilience/docs/README.md) |
| runtime | [docs/crates/runtime/ROADMAP.md](crates/runtime/ROADMAP.md) |
| sandbox | [docs/crates/sandbox/ROADMAP.md](crates/sandbox/ROADMAP.md) |
| sdk | [docs/crates/sdk/ROADMAP.md](crates/sdk/ROADMAP.md) |
| storage | [docs/crates/storage/ROADMAP.md](crates/storage/ROADMAP.md) |
| system | [docs/crates/system/ROADMAP.md](crates/system/ROADMAP.md) |
| telemetry | [docs/crates/telemetry/ROADMAP.md](crates/telemetry/ROADMAP.md) |
| validator | [crates/validator/docs/ROADMAP.md](../crates/validator/docs/ROADMAP.md) |
| worker | [docs/crates/worker/ROADMAP.md](crates/worker/ROADMAP.md) |
| workflow | [docs/crates/workflow/ROADMAP.md](crates/workflow/ROADMAP.md) |
| tenant | [docs/crates/tenant/ROADMAP.md](crates/tenant/ROADMAP.md) |
| cluster | [docs/crates/cluster/ROADMAP.md](crates/cluster/ROADMAP.md) |
| locale | [docs/crates/locale/ROADMAP.md](crates/locale/ROADMAP.md) |
| **Desktop app** | [docs/crates/desktop/ROADMAP.md](crates/desktop/ROADMAP.md) |

Crates without a roadmap file yet (log, webhook) should add one when the crate doc set is created.

Implementation plans can be tracked in dedicated roadmap/task files as they are introduced.

## See Also

- [Project Status](PROJECT_STATUS.md) - Snapshot of current crate delivery
- [Tasks](TASKS.md) - Ordered execution backlog
- [Architecture](ARCHITECTURE.md) - Layer and dependency model

---

## Non-Goals

- **GraphQL** — not planned; REST + WebSocket cover our use cases.
- **nebula-value** — removed; `serde_json::Value` is used everywhere.
- **nebula-app (egui)** — superseded by the Tauri desktop app at `apps/desktop`.
