# Deep Pass Checklist — Per-Crate Documentation

This checklist describes how to do a **full documentation pass** for each crate in `docs/crates/{crate}`, so that every crate has the full set of docs aligned with the codebase and with [SPEC.md](./SPEC.md). The first completed example is **nebula-execution**.

## Goal

For each crate:

1. **Mine archives** — read `_archive/` (or move top-level archive-*.md into `_archive/`) and use insights in CONSTITUTION and DECISIONS.
2. **CONSTITUTION.md** — already created; refine so Production Vision and Non-Negotiables match real code (types, modules, transitions).
3. **Update/rebuild all 11 standard docs** from template, filled with **real** content from `crates/{crate}/src/` and existing docs.
4. **Preserve history** — keep all archived material in `_archive/`; add `_archive/README.md` describing contents.

## Required Output Files (per SPEC)

| File | Purpose |
|------|--------|
| CONSTITUTION.md | Platform role, principles, production vision, non-negotiables, governance |
| README.md | Scope, current/target state, document map (include link to CONSTITUTION) |
| ARCHITECTURE.md | Problem statement, module map, data/control flow, invariants, target, comparative analysis |
| API.md | Public surface (stable/experimental), usage patterns, minimal example, error semantics, compatibility |
| INTERACTIONS.md | Ecosystem map, upstream/downstream, interaction matrix, runtime sequence, ownership, versioning |
| DECISIONS.md | Numbered decisions (D-00X) with context, decision, alternatives, consequences, migration, validation |
| ROADMAP.md | Phases with deliverables, risks, exit criteria; metrics of readiness |
| PROPOSALS.md | Open proposals (P-00X) with type, motivation, proposal, impact, status |
| SECURITY.md | Threat model, controls, abuse cases, requirements, test plan |
| RELIABILITY.md | SLO, failure modes, resilience, runbook, capacity |
| TEST_STRATEGY.md | Test pyramid, critical invariants, scenario matrix, tooling, exit criteria |
| MIGRATION.md | Versioning policy, breaking changes template, rollout/rollback, validation checklist |
| _archive/README.md | What is in the archive; rules (do not delete; mine for insights) |

## Deep Pass Steps (per crate)

1. **List crate modules and public API**  
   - `crates/{crate}/src/lib.rs` and `src/*.rs`  
   - Note public types, traits, and key functions.

2. **List dependencies**  
   - `crates/{crate}/Cargo.toml`  
   - Upstream: which crates this one depends on.  
   - Downstream: who depends on this crate (grep or workspace doc).

3. **Read existing docs and archive**  
   - `docs/crates/{crate}/*.md` and `docs/crates/{crate}/_archive/*.md`  
   - Move any `archive-*.md` from parent into `_archive/` if not already there.  
   - Write `_archive/README.md` listing and describing archive files.

4. **Align CONSTITUTION with code**  
   - Production Vision: real type names, real state/status enums, real modules.  
   - Key gaps table: reflect actual gaps (e.g. “persistent idempotency”, “resume token”).  
   - Non-Negotiables: check they are enforceable and match crate behavior.

5. **Write README.md**  
   - Scope (in/out) from code and CONSTITUTION.  
   - Current state: maturity, strengths, risks.  
   - Target state: production criteria, compatibility.  
   - Document map with CONSTITUTION first.

6. **Write ARCHITECTURE.md**  
   - Problem statement (business + technical).  
   - Module map: table Module | File | Responsibility from `src/`.  
   - Data/control flow: who calls whom; no I/O vs I/O.  
   - Key invariants from code (e.g. transition rules, key format).  
   - Target architecture; design reasoning; comparative analysis (Adopt/Reject/Defer).

7. **Write API.md**  
   - Public surface: stable vs experimental; no exhaustive method list (per SPEC).  
   - Usage patterns (2–3); minimal example from real code.  
   - Error semantics: retryable vs fatal vs validation.  
   - Compatibility: what triggers major bump; deprecation.

8. **Write INTERACTIONS.md**  
   - Ecosystem map (existing + planned).  
   - Upstream/downstream; interaction matrix (contract, sync/async, failure handling).  
   - Runtime sequence (numbered steps).  
   - Cross-crate ownership; versioning and compatibility; contract tests needed.

9. **Write DECISIONS.md**  
   - 3–6 decisions (D-001, D-002, …) from code and CONSTITUTION.  
   - Use template: Context, Decision, Alternatives, Trade-offs, Consequences, Migration, Validation.

10. **Write ROADMAP.md**  
    - Phases (e.g. Phase 1: current; Phase 2: API/schema; Phase 3: persistence/resume).  
    - Deliverables, risks, exit criteria per phase.  
    - Metrics of readiness.

11. **Write PROPOSALS.md**  
    - 2–4 open proposals (P-001, …) from CONSTITUTION or backlog.  
    - Type (Breaking/Non-breaking), Motivation, Proposal, Impact, Status.

12. **Write SECURITY.md**  
    - Threat model (assets, boundaries, attacker).  
    - Controls (authn, isolation, secrets, validation).  
    - Abuse cases; requirements; test plan.

13. **Write RELIABILITY.md**  
    - SLO (if applicable); failure modes; resilience; runbook; capacity.

14. **Write TEST_STRATEGY.md**  
    - Pyramid (unit/integration/contract/e2e).  
    - Critical invariants (from code).  
    - Scenario matrix; tooling; exit criteria.

15. **Write MIGRATION.md**  
    - Versioning policy; breaking-change template; rollout/rollback; validation checklist.

## Reference: Execution Crate (First Full Pass)

- **Location:** `docs/crates/execution/`
- **Code:** `crates/execution/src/` — status, state, transition, output, attempt, plan, context, journal, idempotency, error.
- **Done:** All 11 docs + CONSTITUTION + _archive/README.md; archive files moved into _archive/.

Use execution as the template for structure and level of detail when doing the next crate (e.g. core, action, workflow, engine, runtime).

## Order Suggestion

1. **execution** — done.  
2. **core** — foundation; many consumers.  
3. **workflow** — DAG and plan dependency.  
4. **action** — contract for runtime/engine.  
5. **engine** — orchestrator; uses execution, workflow, action.  
6. **runtime** — execution layer; uses action, execution.  
7. Then: expression, storage, config, credential, resource, memory, system, eventbus, parameter, validator, resilience, api, worker, telemetry, tenant, sandbox, cluster, locale, metrics, idempotency, sdk, etc.

## Definition of Done (per crate)

- [ ] All 11 standard docs present and filled (no empty template placeholders).  
- [ ] CONSTITUTION matches code (types, statuses, modules).  
- [ ] Archive in `_archive/` with README.  
- [ ] Document map in README includes CONSTITUTION.  
- [ ] INTERACTIONS lists upstream/downstream and matrix.  
- [ ] DECISIONS reference real decisions; ROADMAP has phases and exit criteria.  
- [ ] Content aligned with `crates/{crate}/src/` (not aspirational-only).
