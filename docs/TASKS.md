[← Previous Page](ROADMAP.md) · [Back to README](../README.md) · [Next Page →](contributing.md)

# Nebula — Master Task List

Orchestration file for sequential progress across all crates. Each crate has its own `TASKS.md` with detailed tasks. Use this file to navigate the project, track current priorities, and find the next thing to work on.

**How to use**: Work through groups in order. Within each group, crates with `[P]` are parallelizable. After finishing a phase in one crate, check if it unblocks work in another.

---

## Current Priority Tasks (Cross-Crate)

These are the highest-impact tasks across the entire project right now:

- [ ] **P1** — Credential–Resource integration (CRD Phase 1 + RSC Phase 1)
- [ ] **P2** — Storage Postgres backend (STG Phase 1 → T001–T005)
- [ ] **P3** — Action context model stabilization (ACT Phase 2 → complete)
- [ ] **P4** — Engine state integration (ENG Phase 1 — needs STG done first)
- [ ] **P5** — Runtime isolation routing (RTM Phase 1)
- [ ] **P6** — Desktop typed IPC foundation (DSK Phase 1)

---

## Group 1: Core Foundation

*Status: Phase 1 complete for most crates. Remaining work is Phase 2+ hardening.*

| Crate | Current Phase | Status | Tasks |
|-------|--------------|--------|-------|
| [nebula-core](crates/core/TASKS.md) | Phase 4 | Phases 1–3 ✅ | [Tasks](crates/core/TASKS.md) |
| [nebula-config](crates/config/TASKS.md) | Phase 4 | Phases 1–3 ✅ | [Tasks](crates/config/TASKS.md) |
| [nebula-execution](crates/execution/TASKS.md) | Phase 2 | Phase 1 ✅ | [Tasks](crates/execution/TASKS.md) |
| [nebula-telemetry](crates/telemetry/TASKS.md) | Phase 2 | Phase 1 ✅ | [Tasks](crates/telemetry/TASKS.md) |
| [nebula-expression](crates/expression/TASKS.md) | Phase 3 | Phases 1–2 ✅ | [Tasks](crates/expression/TASKS.md) |
| [nebula-validator](../crates/validator/docs/ROADMAP.md) | Complete | Phases 1–4 ✅ | [Roadmap](../crates/validator/docs/ROADMAP.md) |
| [nebula-storage](crates/storage/TASKS.md) | Phase 1 | 🔄 In Progress | [Tasks](crates/storage/TASKS.md) |
| [nebula-metrics](crates/metrics/TASKS.md) | Phase 4 | Phases 1–3 ✅ | [Tasks](crates/metrics/TASKS.md) |
| [nebula-workflow](crates/workflow/TASKS.md) | Phase 1 | ⬜ Planned | [Tasks](crates/workflow/TASKS.md) |
| [nebula-memory](crates/memory/TASKS.md) | Phase 1 | ⬜ Planned | [Tasks](crates/memory/TASKS.md) |
| [nebula-parameter](crates/parameter/EVOLUTION_PLAN.md) | Phase 1 | ⬜ Planned | [Plan](crates/parameter/EVOLUTION_PLAN.md) |
| [nebula-system](crates/system/TASKS.md) | Phase 1 | ⬜ Planned | [Tasks](crates/system/TASKS.md) |
| [nebula-resilience](crates/resilience/TASKS.md) | Phase 9 (Integration) | 🔄 In Progress | [Tasks](crates/resilience/TASKS.md) |
| [nebula-macros](crates/macros/TASKS.md) | Phase 1 | ⬜ Planned | [Tasks](crates/macros/TASKS.md) |

**Recommended order for remaining Group 1 work**:
1. `nebula-storage` Phase 1 (Postgres) — blocks engine integration
2. `nebula-expression` Phase 3 (cache tuning) — independent
3. `nebula-validator` Phase 2 (compatibility/governance) — independent
4. `nebula-metrics` Phase 4 (OTLP) — independent, wire `/metrics` in api
5. Remaining crates (workflow, memory, parameter, system, macros) — can proceed in parallel
6. `nebula-resilience` post-Phase backlog (runtime/engine integration, telemetry, CI perf gates)

---

## Group 2: Execution Engine 🔄

*Status: Active development. This is the current primary focus.*

**Prerequisites**: Group 1 storage Postgres backend (nebula-storage Phase 1).

| Crate | Current Phase | Status | Tasks |
|-------|--------------|--------|-------|
| [nebula-action](crates/action/TASKS.md) | Phase 2 | 🔄 In Progress | [Tasks](crates/action/TASKS.md) |
| [nebula-resource](crates/resource/TASKS.md) | Phase 1 | 🔄 In Progress | [Tasks](crates/resource/TASKS.md) |
| [nebula-engine](crates/engine/TASKS.md) | Phase 1 | ⬜ Waiting on storage | [Tasks](crates/engine/TASKS.md) |
| [nebula-runtime](crates/runtime/TASKS.md) | Phase 1 | 🔄 In Progress | [Tasks](crates/runtime/TASKS.md) |

**Recommended order**:
1. `nebula-action` Phase 2 — finish context model + capability modules
2. `nebula-resource` Phase 1 — contract docs + scope invariants
3. `nebula-runtime` Phase 1 — isolation routing + SpillToBlob (parallel with resource)
4. `nebula-engine` Phase 1 — wire to storage (needs STG Phase 1 done)
5. Runtime stabilization and execution hardening after engine wiring

**Group 2 acceptance criteria** (from main ROADMAP):
- [ ] Single-node workflow executes end-to-end
- [ ] Multi-node DAG with dependencies resolves correctly
- [ ] Execution state persists to PostgreSQL
- [ ] Cancellation and timeout work correctly

---

## Group 3: Credential and Plugin System ⬜

*Status: Credential Phase 1 in active progress. Plugin planned.*

**Prerequisites**: Group 2 action/resource contracts stable.

| Crate | Current Phase | Status | Tasks |
|-------|--------------|--------|-------|
| [nebula-credential](crates/credential/TASKS.md) | Phase 1 | 🔄 In Progress | [Tasks](crates/credential/TASKS.md) |
| [nebula-plugin](crates/plugin/TASKS.md) | Phase 1 | ⬜ Planned | [Tasks](crates/plugin/TASKS.md) |

**Recommended order**:
1. `nebula-credential` Phase 1 → Phase 2 → Phase 3 (can overlap with Group 2)
2. `nebula-plugin` Phase 1 (after credential Phase 1 and action Phase 2 are stable)

---

## Group 4: Developer Experience ⬜

*Status: Planned. Requires Groups 2–3 APIs to be stable.*

| Crate | Current Phase | Status | Tasks |
|-------|--------------|--------|-------|
| [nebula-sdk](crates/sdk/TASKS.md) | Phase 1 | ⬜ Planned | [Tasks](crates/sdk/TASKS.md) |
| [nebula-macros](crates/macros/TASKS.md) | Phase 1 | ⬜ Planned | [Tasks](crates/macros/TASKS.md) |

---

## Group 5: API and UI ⬜

*Status: API foundation started. Desktop Phase 1 in progress.*

**Prerequisites**: Group 2 engine working end-to-end (for workflow execution features).

| Crate | Current Phase | Status | Tasks |
|-------|--------------|--------|-------|
| [nebula-api](crates/api/TASKS.md) | Phase 1 | 🔄 In Progress | [Tasks](crates/api/TASKS.md) |
| [Desktop App](../apps/desktop/README.md) | Phase 1 | 🔄 In Progress | [App Docs](../apps/desktop/README.md) |

**Recommended order**:
- `nebula-api` Phase 1 (workflow + execution REST) can proceed in parallel with Group 2
- Desktop Phase 1 (typed IPC + stores) can proceed now; Phase 2+ waits for Backend Phase 2

---

## Group 6: Expansion Tracks ⬜

*Status: Planned for later phases after core execution and API milestones.*

| Crate | Current Phase | Status | Tasks |
|-------|--------------|--------|-------|
| [nebula-eventbus](crates/eventbus/TASKS.md) | Phase 1 | ⬜ Planned | [Tasks](crates/eventbus/TASKS.md) |
| [nebula-auth](../crates/auth/rfcs) | RFC phase | 🔄 In Progress | [RFCs](../crates/auth/rfcs) |

**Recommended order**:
1. `nebula-eventbus` Phase 1 — consolidate telemetry/resource EventBus (can start soon)
2. `nebula-auth` stabilization — converge RFCs into implementation milestones
3. Expand event-driven integrations after core API/runtime milestones

---

## Sequential Progress Guide

### Right now (next 2–4 weeks)

1. **Finish `nebula-storage` Phase 1** (STG tasks) — unblocks engine
2. **Complete `nebula-action` Phase 2** (ACT tasks) — unblocks runtime and credential integration
3. **Complete `nebula-resource` Phase 1** (RSC tasks) — credential–resource integration
4. **Complete `nebula-runtime` Phase 1** (RTM tasks) — isolation routing
5. **Start Desktop Phase 1** (DSK tasks T004–T017) — typed IPC independent of backend

### After storage + action are done

6. **Start `nebula-engine` Phase 1** (ENG tasks) — wire state to Postgres
7. **Continue `nebula-credential` Phase 1** (CRD tasks) — contract consolidation
8. **Continue runtime hardening** (timeouts, cancellation, memory pressure)

### After Group 2 acceptance criteria are met

9. **`nebula-credential` Phase 2–4** — rotation reliability, provider hardening, production infra
10. **`nebula-plugin` Phase 1** — registry contract
11. **`nebula-sdk` Phase 1** — prelude stability
12. **Desktop Phase 2** — workflow canvas (requires Backend Phase 2 done)

---

## Dependency Graph (simplified)

```
nebula-core
  └─ nebula-storage ─────────────────→ nebula-engine
  └─ nebula-execution                       ↑
  └─ nebula-action ──────────────→ nebula-runtime
  └─ nebula-resource ──────────────╯         ↓
  └─ nebula-credential (uses action/resource)
  └─ nebula-plugin (uses action/credential)
  └─ nebula-sdk (re-exports all above)
  └─ nebula-api (uses engine/runtime/plugin/credential)
  └─ nebula-eventbus (used by telemetry/resource/engine)
```

---

## Legend

| Symbol | Meaning |
|--------|---------|
| ✅ Done | Phase complete |
| 🔄 In Progress | Actively being worked |
| ⬜ Planned | Not started |
| `[P]` | Can run in parallel with other [P] tasks in same phase |

## See Also

- [Roadmap](ROADMAP.md) - Milestones and dependency order
- [Project Status](PROJECT_STATUS.md) - Current progress snapshot
- [Contributing](contributing.md) - Contribution and review process
