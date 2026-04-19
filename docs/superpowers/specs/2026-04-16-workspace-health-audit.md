# Workspace Health Audit — Nebula

**Date:** 2026-04-16
**Author:** Claude (Opus 4.7) + 5 parallel Explore agents
**Authority:** Subordinate to `docs/PRODUCT_CANON.md`. This audit proposes breaking changes; any conflict with canon is resolved by updating canon deliberately or adjusting this spec.
**Status:** APPROVED — all open questions resolved 2026-04-16; ready to transition to `writing-plans` for Sprint A1 + Sprint B (parallel tracks)
**Parent specs (informing this audit):**
- `docs/plans/2026-04-15-arch-specs/21-schema-crate.md` — canonical decision: `nebula-schema` replaces `nebula-parameter`
- `docs/plans/2026-04-15-arch-specs/21-schema-crate-implementation-plan.md` — Tasks 1–5 done, Task 6 (migration + parameter deletion) open

---

## TL;DR

Nebula's architecture (per canon §3.5, §3.10, §12) is sound. The workspace is suffering from **incomplete migrations** and **unverified honesty**: several crates advertise capabilities the engine does not yet own end-to-end (§14 false capabilities), a storage refactor is mid-flight with two coexisting APIs (§12.2 two-truths), and the API layer breaks the §10 golden path at two steps (activation + cancel). Individual crate designs are largely correct — the pain is in the **seams between them**.

**Product impact right now:**

- Knife scenario §13 does **not** pass end-to-end. Steps 2 (activation validation) and 5 (engine-visible cancel) are broken.
- Credential isolation claimed but **not enforced** (fail-open allowlist in engine).
- **Two coexisting typed-config systems** (parameter + schema) are mid-migration per spec 21 — Tasks 1–5 done, Task 6 (migration + deletion of parameter) open.
- Resource concept breaks the uniform §3.5 `*Metadata + ParameterCollection` pattern — will be resolved by schema migration.

**Single biggest win:** closing the §12.2 loop (API cancel → control queue → engine consumer → terminal state) would flip knife §13 from failing to passing. Everything else is quality/hygiene around a working core, with the schema migration (spec 21 Task 6) as the largest breaking-change axis.

**Revision 2026-04-16 (post-A2.4 investigation):** §2.3's "storage two-truths" framing was corrected — the `repos/*` layer is not a pending migration but a planned architectural pivot (spec-16 multi-tenant row model) with no implementations yet. Audit solution is canon §11.6 status clarification; full adoption moved to Sprint E (§8) as a separate track. See §2.3 for the corrected narrative.

---

## 1. Crate health matrix

Legend: **S** = stable, **H** = half-done, **B** = broken, **O** = orphan (no real consumer). Priority: **P0** blocks knife / false capability; **P1** blocks other crates or is upward-violating; **P2** SRP/DX; **P3** polish.

### Core layer

| Crate | SLOC | Health | Top issues | Pri | Recommendation |
|---|---|---|---|---|---|
| `nebula-core` | ~2 000 | S | — | — | keep |
| `nebula-validator` | ~13 500 | S | `rule.rs` god file 2 549 lines; macro 930 lines; no trybuild | P1 | refactor `rule.rs` → `rule/{types,eval,serde}` |
| `nebula-parameter` | ~13 000 | **deprecated-for-migration** | Being replaced by `nebula-schema` per spec 21. All issues (local `Rule`, god file, orphan loaders, silent `normalize.rs`) are **resolved by deletion** at end of Task 6. No further investment. | — | **delete after spec 21 Task 6 migration completes** |
| `nebula-expression` | ~8 000 | S | `eval.rs` 2 545 lines (cohesive, not a god file) | P3 | keep |
| `nebula-workflow` | ~5 000 | H | **upward violation**: deps on `nebula-action` for `InterfaceVersion` (§12.1) | P1 | move `InterfaceVersion` to `nebula-core` or pass at activation |
| `nebula-execution` | ~4 000 | S | — | — | keep (reference quality) |
| `nebula-schema` | ~1 500 | **H (intentional WIP)** | Tasks 1–5 of spec 21 done; Task 6 (consumer migration + `nebula-parameter` deletion) not started. Not an orphan — it is the canonical future of typed config. | P0 | **execute spec 21 Task 6 (see §8 Sprint 4)** |

### Business layer

| Crate | SLOC | Health | Top issues | Pri | Recommendation |
|---|---|---|---|---|---|
| `nebula-credential` | ~4 500 | S | rotation lacks concurrent refresh/resolve stress tests | P2 | keep; add adversarial tests |
| `nebula-resource` | ~3 800 | H | **§3.5 violation**: `ResourceMetadata` has no typed config schema; 6+ per-topology `Config` types scattered. **Resolved by schema migration** (spec 21 Task 6 Step 2 migrates `resource` → `Schema`). | P0 | migrate as part of spec 21 Task 6 |
| `nebula-action` | ~5 500 | S | **§14 risk**: `ActionResult::Retry`/`Wait` — engine wiring unverified (§11.2) | P0 | verify end-to-end OR remove variants |
| `nebula-plugin` | ~700 | S | thin descriptor types; relies on authors to avoid §7.1 double-declaration | P2 | docs/lint |
| `nebula-plugin-sdk` | ~600 | S | protocol Slice 1c only; `PluginCtx` is placeholder (acceptable, staged) | P3 | keep |

### Exec layer

| Crate | SLOC | Health | Top issues | Pri | Recommendation |
|---|---|---|---|---|---|
| `nebula-engine` | ~4 400 | H | `engine.rs` ~4 000 lines god file; ~~**fail-open credential allowlist** (§4.5)~~ resolved via `7b811372` (deny-by-default); no resource allowlist (intentional per Q4=B); 4 undocumented panic sites | P0 | split orchestrator; replace panics |
| `nebula-runtime` | ~2 700 | H | **`src/sandbox.rs` is a self-documented dead compat shim (§14)**; 4 panic sites | P0 | **delete `sandbox.rs`**; typed errors for panics |
| `nebula-storage` | ~2 000 | H | **§12.2 two-truths**: old `execution_repo.rs`/`workflow_repo.rs` + new `repos/` coexist; new control queue only in new layer | P0 | finish migration; delete old trait files; verify engine calls new API |
| `nebula-sandbox` | ~1 600 | S | no e2e test of in-process + process paths together | P3 | keep; add integration test |
| `nebula-sdk` | ~1 300 | S | testing harness feature-gated; thin | P3 | keep |

### API + Apps

| Crate | SLOC | Health | Top issues | Pri | Recommendation |
|---|---|---|---|---|---|
| `nebula-api` | ~2 000 | **B** on knife | **§10 violation**: `activate_workflow` skips `validate_workflow()`; **§12.2 violation**: `cancel_execution` transitions DB state without `execution_control_queue::enqueue`; stringly-typed status; orphan `services/` | P0 | wire validation in activation; wire control queue in cancel; type status; delete or implement `services/` |
| `apps/cli` | ~1 000 | S | — | — | keep |
| `examples/` | ~500 | S | — | — | keep |

### Cross-cutting

| Crate | SLOC | Health | Top issues | Pri | Recommendation |
|---|---|---|---|---|---|
| `nebula-log` | ~6 300 | S | `observability/*` modules 572/645 lines | P3 | keep |
| `nebula-system` | ~2 800 | H | weak test coverage on platform-specific code | P1 | add tests |
| `nebula-eventbus` | ~1 400 | S | — | — | keep |
| `nebula-telemetry` | ~1 400 | S | **zero unit tests**; histogram/interner correctness untested | P1 | add test suite |
| `nebula-metrics` | ~1 600 | S | — | — | keep |
| `nebula-config` | ~5 900 | H | `Config` 1 143 lines mixes builder/access/validation | P1 | split `ConfigStore` / `ConfigLoader` |
| `nebula-config-macros` | ~150 | **O** | zero in-tree consumers; zero tests | P1 | **delete** (re-add when needed) |
| `nebula-resilience` | ~8 000 | S | baseline — reference quality | — | keep |
| `nebula-error` | ~3 100 | S | — | — | keep |
| `nebula-error-macros` | ~300 | S | — | — | keep |
| `nebula-validator-macros` | ~400 | S | — | — | keep |

**Totals:** 34 crates. **1 dead shim** (`runtime/src/sandbox.rs`). **1 orphan macro crate** (`config-macros`). **3 god files** (`rule.rs`, `parameter.rs`, `engine.rs` — `parameter.rs` goes away with the migration, not worth splitting). **5 P0 knife/false-capability blockers + 1 P0 migration** (spec 21 Task 6).

**Note on `nebula-schema`:** previously marked orphan in this audit's first draft. Corrected after user direction + spec 21 reading — it is intentional WIP awaiting Task 6 (consumer migration + `nebula-parameter` deletion).

---

## 2. P0 — knife & false-capability blockers

These block §13 knife or violate §14 "implement end-to-end or delete." Every item here has a concrete location and a concrete fix.

### 2.1 API activation skips validation — §10 violation

**Location:** `crates/api/src/handlers/workflow.rs::activate_workflow` (line ~396–465).

**Current behavior:** flips the `active` flag without calling `nebula_workflow::validate_workflow()`. A separate `/validate` endpoint exists but activation does not invoke it.

**Canon:** §10 step 2 — "Activation runs `nebula_workflow::validate_workflow` and **rejects** invalid definitions with structured RFC 9457 errors."

**Fix:** in `activate_workflow`, call validator before the state transition; on failure return RFC 9457 `422` mapped from structured validator output. Add integration test `activate_invalid_returns_422` + `activate_valid_returns_200`.

---

### 2.2 API cancel does not enqueue control signal — §12.2 violation

**Location:** `crates/api/src/handlers/execution.rs::cancel_execution` (line ~261–353).

**Current behavior:** reads via CAS, transitions state to `cancelled`, sets `finished_at`. **Does not** `execution_control_queue.enqueue(Cancel { ... })` in the same logical operation. Engine never sees the cancel.

**Canon:** §12.2 — "the signal is written to `execution_control_queue` (outbox) in the same logical operation as the corresponding state transition."

**Fix:** in the cancel handler, enqueue to control queue atomically with the CAS transition. Add integration test that observes both the durable row and a mock consumer receiving the `Cancel` command.

---

### 2.3 Storage — two trait layers, NOT a mechanical migration (superseded 2026-04-16)

**First-draft diagnosis (wrong):** this audit originally framed the coexistence of legacy `execution_repo.rs` / `workflow_repo.rs` and new `repos/*` as a §14 two-truths "finish the migration" task (Q5=A single atomic PR).

**Corrected diagnosis (after deep investigation by A2.4 agent):** the two layers are not the same contract in two presentations — they encode **two different product architectures**:

| Legacy layer (implemented, production) | New layer (`repos/*`, planned) |
|---|---|
| `nebula_storage::ExecutionRepo` / `WorkflowRepo` — state-as-JSON-blob model | `repos::ExecutionRepo` / `WorkflowRepo` — spec-16 structured-row model |
| Typed IDs (`ExecutionId`, etc.), `u64` CAS version | Raw `&[u8]` IDs, `i64` version |
| `transition(id, version, state_json) → bool` | `transition(id, version, status_str, Option<patch>) → ExecutionRow` |
| No multi-tenancy | Mandatory `workspace_id` + `org_id` on every create |
| `workflows.definition` JSON column | `WorkflowRow` + `WorkflowVersionRow` split |
| Idempotency as method on `ExecutionRepo` | Idempotency as column on `ExecutionNodeRow` (different repo) |
| Implementations: `InMemoryExecutionRepo`, `InMemoryWorkflowRepo`, `PgExecutionRepo`, `PgWorkflowRepo` | **Zero implementations** for `ExecutionRepo`, `WorkflowRepo`, `ExecutionNodeRepo`, `JournalRepo`. Only `InMemoryControlQueueRepo` is wired. |
| Engine, API, runtime, tests, examples — all depend on this | **Only** `InMemoryControlQueueRepo` has a real consumer (API cancel — A1.2) |
| 8 flat migrations | 19+19 spec-16 migrations (Postgres + SQLite) |

**Why the original "delete legacy" fix is wrong:** the new traits cannot compile without engine + API + runtime rewrite (different method signatures, multi-tenancy requirement, split workflow model, new idempotency model). There is no mechanical path and no adapter path that honours Q1=A.

**Decision (user, 2026-04-16): Option C — explicit canon §11.6 status split.** Keep both layers, mark them honestly:

1. **Layer 1 (top-level `ExecutionRepo` / `WorkflowRepo` re-exports)** — status **`implemented`**. Production path. Knife §13 runs against this. New consumers default here.
2. **Layer 2 (`repos/*` module)** — status **`planned`** for Execution/Workflow/Node/Journal/etc. **Exception:** `ControlQueueRepo` + `InMemoryControlQueueRepo` are `implemented` (A1.2 wired them into the API cancel handler).
3. No deletion. No adapter. Docs make the split explicit (canon §11.6): crate-level doc in `crates/storage/src/lib.rs`, module-level doc in `crates/storage/src/repos/mod.rs`, audit spec (this §2.3), and the roadmap (§8 Sprint E).

**Why this does not violate §14 two-truths:** canon §14 anti-pattern is "execution state in DB says X, channel/queue says Y, with no formal reconciliation story." Here the reconciliation story is explicit: layer 1 is the production contract, layer 2 is a planned future contract with one already-shipped construct (control queue). There is exactly one authoritative path per concern at any given time.

**Follow-up work:** "Sprint E — adopt spec-16 row model" (§8) is a dedicated future plan: write in-memory implementations of all new-layer traits, rewrite engine/API/runtime to use them, wire multi-tenancy, delete layer 1 in a final PR. Not in scope for this audit's Sprints A–D.

**Implementation of decision C (shipped with audit update):**

- `crates/storage/src/lib.rs` — crate doc explicitly labels both layers
- `crates/storage/src/repos/mod.rs` — module doc table of per-trait status
- This audit §2.3 — corrected narrative (above)
- §6 breaking-changes list — "storage two-truths resolution" removed; replaced with "storage status markers + Sprint E adoption plan"
- §8 roadmap — Sprint A2 renamed to "storage canon status documentation" (docs only); Sprint E added as the real adoption plan

---

### 2.4 Engine credential allowlist was fail-open — RESOLVED (§4.5 + §12.5)

**Status:** **RESOLVED** — landed via commit `7b811372 fix(engine): credential access denies by default without declaration`. `EngineCredentialAccessor` now denies every request when the allowlist is empty; per-action allowlists are populated via `WorkflowEngine::with_action_credentials` and merge on repeated declarations. TDD coverage: `credential_accessor::tests::{get_denies_every_key_when_allowlist_is_empty, has_returns_false_for_empty_allowlist, denied_request_never_invokes_resolver}` plus five engine-level integration tests (`credential_access_denied_without_declaration`, `credential_access_allowed_with_declaration`, `credential_access_denied_for_mismatched_key`, `credential_declaration_is_per_action_key`, `action_credentials_merge_across_builder_calls`).

**Location (historical):** `crates/engine/src/engine.rs::EngineCredentialAccessor` (around line 1312).

**Prior behavior:** empty allowlist = "allow all." Docs said enforcement would come when per-node declarations are wired from action dependency metadata — unimplemented at audit time.

**Canon:** §4.5 "operational honesty — no false capabilities" + §12.5 "secrets and auth."

**Fix (landed):** `EngineCredentialAccessor::is_allowed` now checks membership unconditionally — no "empty means all" escape hatch. Per-action allowlists are populated via `WorkflowEngine::with_action_credentials(ActionKey, [credential_ids])`; nodes whose action was never declared to the engine fall through to the deny baseline.

**Resource access scoping — decision Q4 (resolved):** engine does **not** grow a resource allowlist. Resource scoping lives at the **topology** layer (e.g. pool scope, daemon scope) per user direction. The current engine-side `resource_accessor.rs` stays "allow all" intentionally; document this in its module docs so it's not read as a false-capability stub. Remove any dead allowlist-shaped code that implies enforcement.

---

### 2.5 Runtime dead compat shim — §14 violation

**Location:** `crates/runtime/src/sandbox.rs`.

**Current behavior:** self-documented as a dead compat shim re-exporting `nebula_sandbox::*`. Canon §14 explicitly lists "compatibility shims that preserve bad shapes 'for now'" as an anti-pattern.

**Fix:** delete the file; update any remaining imports to `nebula_sandbox::*` directly. Single-PR change.

---

### 2.6 ActionResult phantom variants — DEFERRED to spec 27

**Location:** `crates/action/src/result.rs`, variants `Retry` and `Wait`.

**Status (resolved Q3):** this audit does **not** own the `ActionResult` design. [`docs/plans/2026-04-15-arch-specs/27-nebula-action-redesign.md`](../../plans/2026-04-15-arch-specs/27-nebula-action-redesign.md) owns the redesign of the action surface, including `ActionResult` variants. Any false-capability resolution for `Retry` / `Wait` lands there, not here.

**Consequence for this audit:** removed from the Sprint roadmap. If spec 27 resolves the variants before this audit's Sprints complete, the knife test (§8 item #3) will pick up the new shape automatically. No blocking dependency on spec 27 from Sprints 1–5 of this audit.

---

### 2.7 Resource breaks §3.5 integration model — resolved by schema migration

**Location:** `crates/resource/src/metadata.rs` — `ResourceMetadata` has no typed config schema. Topologies carry their own `Config` types (`DaemonConfig`, `PoolConfig`, `ServiceConfig`, `ExclusiveConfig`, …).

**Canon §3.5:** "Every concept in Nebula's integration layer is described by `*Metadata + ParameterCollection`." Credential and Action satisfy this shape (using `nebula-parameter` today). Resource does not.

**Fix:** **no separate action needed** — spec 21 Task 6 Step 2 migrates `nebula-resource` to `nebula-schema::Schema`. `ResourceMetadata` gains a `config: Schema` field during that migration. This closes the §3.5 gap as a side-effect of migration, not as an extra plan.

**Watch during migration:** confirm that topology `Config` types either (a) project to a single `Schema` via derive, or (b) each variant gets its own `Schema` — pick one in the migration plan, don't leave both paths alive.

---

### 2.8 Two Rule systems — resolved by schema migration

**Location:** `nebula_validator::Rule` enum (rule.rs, ~2 500 lines) and `nebula_parameter::rules::Rule` (local copy, diverged).

**Canon §3.9:** one shared parameter-validation system.

**Fix:** **no separate action** — spec 21 §2.6 mandates "all validation predicates are `nebula_validator::Rule`." Once `nebula-schema` replaces `nebula-parameter`, the divergent `parameter::rules::Rule` is deleted with the rest of the crate. Single source of truth comes for free at migration end.

**Residual work after migration:** split `nebula_validator::rule.rs` (still 2 549 lines) into `rule/{types,eval,serde}` — this is an independent P1 refactor (§3.1) because the validator itself still benefits from it regardless of schema migration.

---

### 2.9 `nebula-schema` replaces `nebula-parameter` — RESOLVED

**Decision (from user direction + spec 21):** Option B (activate). `nebula-schema` is the canonical future typed-config crate. `nebula-parameter` is deprecated-for-migration and will be deleted at the end of spec 21 Task 6.

**Status:**

- Tasks 1–5 of spec 21 implementation plan: **done** (scaffold, primitives, field model, values, tests).
- Task 6 (migrate consumers: `action`, `credential`, `resource`, `engine`, `apps/cli`; delete `parameter`): **not started** — this is the largest remaining breaking-change axis.

**Key design properties from spec 21 (for reviewers who haven't read it):**

- Pattern 4 — `Field` enum wrapper + 18 per-type structs (`StringField`, `SecretField`, `NumberField`, …). Makes structurally invalid states unrepresentable at compile time (e.g. `StringField { min: 5 }` is a compile error).
- Renames: `ParameterCollection → Schema`, `Parameter → Field`, `ParameterValue → FieldValue`, `Condition → nebula_validator::Rule` (unified), `ParameterId → FieldKey` (validated newtype).
- `SecretField` is a dedicated type (replaces the footgun `secret: bool` flag).
- Widget hints become typed per-field enums (7 of them).

**Canon implication:** once migration completes, canon §3.9 should be updated to name `nebula-schema` as the typed-config crate (replacing `nebula-parameter` in the §3.10 table). This is a deliberate canon revision, not spec theater — covered in §10 of this audit.

**User-memory flag:** your memory instructs "never propose adapters/bridges/shims — replace the wrong thing directly." Spec 21 Task 6 Step 1 currently says *"Introduce dual-world adapter layer in `nebula-action`."* This is exactly the kind of shim you asked me to flag. Proposal in §7 open question #1 below: drop the adapter step and migrate callsites in a single PR.

---

## 3. P1 — SRP / DRY / upward violations

### 3.1 God files to split

| File | Lines | Split into |
|---|---|---|
| `crates/engine/src/engine.rs` | ~4 000 | `orchestrator.rs` (frontier) + `dispatch.rs` + keep accessors in modules |
| `crates/validator/src/rule.rs` | 2 549 | `rule/types.rs` + `rule/eval.rs` + `rule/serde.rs` |
| `crates/parameter/src/parameter.rs` | 1 538 | `parameter/definition.rs` + `parameter/builder.rs` + `parameter/metadata.rs` |
| `crates/expression/src/eval.rs` | 2 545 | cohesive — keep unless hot-path profiling demands split |
| `crates/config/src/core.rs` | 1 143 | `ConfigStore` (access) + `ConfigLoader` (loading) |

### 3.2 Upward violations

- `nebula-workflow` → `nebula-action` (for `InterfaceVersion`). Move `InterfaceVersion` to `nebula-core`.

### 3.3 Runtime panic sites

4 panic sites in `crates/runtime` should become typed `RuntimeError` variants. No silent panics in orchestration paths.

---

## 4. P1–P2 — tests & observability gaps

- **`nebula-telemetry`** — zero unit tests. Add coverage for histogram quantiles, `LabelInterner` correctness, concurrent `MetricsRegistry`.
- **`nebula-system`** — weak coverage for platform-specific code. Add tests for each `#[cfg]` branch.
- **`nebula-api`** — no integration test exercising the full knife §13 scenario. Add `knife_scenario.rs` e2e test (define → activate → start → get → cancel → terminal).
- **`nebula-validator`** — no `trybuild` tests for the derive macro. Add them.
- **Macros broadly** — `trybuild` compile-tests missing for `parameter`, `action`, `resource`, `plugin`, `credential` derive macros.

---

## 5. Cross-crate duplication map

- **Metadata types** — `CredentialMetadata`, `ActionMetadata`, `ResourceMetadata` are per-crate with no shared base. Action + Credential carry `ParameterCollection`; Resource does not. Proposal: shared `IntegrationMetadata` trait OR a consistent inherent shape enforced by the §3.5 audit in §2.7.
- **Rule enum** — validator vs parameter. Proposal in §2.8.
- **Parameter / Schema** — two typed-config systems. Proposal in §2.9.
- **Transformer** — `nebula-parameter::Transformer` and `nebula-schema::Transformer` represent similar concept. Converges with §2.9 decision.
- **API `ExecutionResponse`** — duplicates fields from `nebula-execution` domain model. Acceptable as boundary DTO, but stringly-typed `status` should become the `ExecutionStatus` enum.

No harmful cross-crate duplication found in cross-cutting (telemetry vs metrics is correctly separated, not duplicated).

---

## 6. Proposed breaking changes (summary)

These are the breaking changes implied by §2 + §3. Each is an independent plan candidate.

**Track A — knife & honesty:**

1. **API: activation validation + cancel control-queue wiring + typed status.** (§2.1, §2.2)
2. **Storage canon status documentation** (docs only) → explicitly mark layer 1 `implemented`, layer 2 `planned` per canon §11.6. See §2.3 corrected. The actual spec-16 adoption is Sprint E (§8), not this audit's scope. (§2.3)
3. **Engine credential allowlist** → deny-by-default from `ActionDependencies`. (§2.4)
4. **Delete `runtime/src/sandbox.rs`**. (§2.5)

**Track B — schema migration (spec 21 Task 6):**

5. **Execute spec 21 Task 6** atomically per consumer, no adapter layer, dependency order `action → credential → resource → engine → cli`, then delete `crates/parameter/*`. Ships with canon §3.5/§3.9/§3.10 revisions in the final PR. (§2.7, §2.8, §2.9, per Q1=A, Q2=confirmed, Q6=A)

**Sprint C — SRP + polish:**

6. **Workflow layering** → `InterfaceVersion` moved to `nebula-core`. (§3.2)
7. **God file splits** → `engine.rs`, `validator/rule.rs`, `config/core.rs`. (`parameter.rs` drops with schema migration.) (§3.1)
8. **Runtime panic replacement** → 4 panic sites → `RuntimeError` variants. (§3.3)
9. **Delete `nebula-config-macros`** (unless rescued with consumers + tests).
10. **Stringly-typed `ExecutionStatus` → typed enum** at API boundary. (§2.1 side-effect)
11. **Tests:** telemetry unit tests, system platform tests, validator trybuild, API knife e2e. (§4)

**Explicitly not in scope of this audit** (per decisions Q3, Q4):

- `ActionResult::Retry` / `Wait` — owned by spec 27.
- Engine-side resource allowlist — resource scoping lives in topology layer.

Each of these fits the "breaking changes allowed" mandate from CLAUDE.md "Development Mode".

---

## 7. Open questions — all resolved (2026-04-16)

All seven questions were answered in user chat. Summary of decisions and their impact on the roadmap:

| # | Question | Decision | Audit impact |
|---|---|---|---|
| Q1 | Drop schema adapter layer? | **A (drop)** — atomic migration PR per consumer, no dual-world adapter | §8 Sprint 4 sequence is strict dependency order; spec 21 Task 6 Step 1 is dropped |
| Q2 | Schema migration order | **confirmed**: `action → credential → resource → engine → apps/cli` | §8 Sprint 4 uses this order |
| Q3 | `ActionResult::Retry`/`Wait` | **deferred** — owned by [spec 27](../../plans/2026-04-15-arch-specs/27-nebula-action-redesign.md); this audit does not touch it | §2.6 rewritten as deferral; Sprint 3 item removed |
| Q4 | Resource allowlist in engine? | **B (no)** — resource scoping lives at topology layer, not engine. Engine resource-accessor stays "allow all" (documented as intentional) | §2.4 narrowed to credential allowlist only; resource-allowlist plan removed from roadmap |
| Q5 | Storage migration — one PR or incremental? | **A (one PR)** — audit callers, migrate everything, delete old in single atomic change | §8 Sprint 2 becomes a single plan file, not a sequence |
| Q6 | Resource topology configs unification | **A** — single `Schema` with `topology` discriminator + `Rule::When` per-topology gating | §8 Sprint 4's resource migration plan specifies this shape |
| Q7 | Sprint ordering | **C (parallel)** — Sprint 1 (API knife fixes) and Sprint 4 (schema migration) run in parallel; different PRs, different files | §8 rewritten to show parallel tracks |

---

## 8. Proposed roadmap (converts this audit into plan files)

Each numbered item below is a candidate for its own `docs/plans/2026-04-XX-*.md` plan via `writing-plans`. Reflects decisions Q1–Q7.

**Two parallel tracks (per Q7=C):**

- **Track A — knife & honesty** (Sprints A1–A3): quick wins that flip knife §13 from failing to passing.
- **Track B — schema migration** (Sprint B): the long-running breaking change (spec 21 Task 6) with atomic per-consumer PRs (per Q1=A).

Tracks don't collide — Track A touches `crates/api`, `crates/storage`, `crates/engine`, `crates/runtime`; Track B touches `crates/action`, `crates/credential`, `crates/resource`, eventually `crates/engine` and `apps/cli`. Engine gets touched by both tracks only at the tail of Track B (resource + cli migration land after engine is already fixed in Track A Sprint 3).

---

### Track A — knife & honesty

**Sprint A1 — flip the knife (1 week):**

1. `2026-04-17-api-activation-validate.md` — wire `validate_workflow` into `activate_workflow`; add tests. (§2.1)
2. `2026-04-17-api-cancel-control-queue.md` — wire control-queue enqueue into cancel handler. (§2.2)
3. `2026-04-18-knife-e2e-test.md` — end-to-end integration test covering §13 steps 1–6.

**Sprint A2 — storage canon status markers (docs only, ~30 min):**

4. `2026-04-21-storage-canon-status.md` — mark layer 1 `implemented` / layer 2 `planned` in crate docs, module docs, README. No code changes. **Supersedes original A2.4 "delete legacy" framing** (investigation showed layer 2 has no implementations and demands architectural refactor, not a mechanical migration). Adoption work moves to Sprint E.

**Sprint A3 — honesty closure (3–4 days):**

5. `2026-04-24-engine-credential-allowlist.md` — wire deny-by-default credential allowlist from `ActionDependencies`. (§2.4, credential only per Q4=B)
6. `2026-04-25-engine-resource-accessor-docs.md` — document that engine resource access is intentionally unscoped; topology owns scoping. Remove any dead allowlist-shaped code. (§2.4 resource part per Q4=B)
7. `2026-04-26-runtime-sandbox-shim-delete.md` — trivial. (§2.5)

**Note:** `ActionResult` verify/remove plan **dropped** (Q3 defers to spec 27).

---

### Track B — schema migration (spec 21 Task 6)

**Sprint B — atomic per-consumer migration (2–3 weeks):**

Dependency order (per Q2): action → credential → resource → engine → cli → deletion. No adapter layer (per Q1=A). Each PR atomic — workspace stays green on every merge.

8. `2026-04-28-schema-migrate-action.md` — `nebula-action` callsites flip from `ParameterCollection` to `Schema`. First, because downstream crates depend on it.
9. `2026-04-30-schema-migrate-credential.md` — `nebula-credential` → `Schema`.
10. `2026-05-02-schema-migrate-resource.md` — `nebula-resource` → `Schema`. **Design decision (Q6=A):** single `Schema` with `topology` discriminator field + `Rule::When` per-topology gating. **Closes §3.5 violation** (§2.7) as side-effect.
11. `2026-05-04-schema-migrate-engine-cli.md` — `nebula-engine` + `apps/cli` → `Schema`. Last consumers.
12. `2026-05-06-delete-nebula-parameter.md` — remove `crates/parameter/*`, workspace member, references. **Closes §2.8 (Rule unification)** and §2.9 as side-effects.
13. `2026-05-07-canon-3-9-update.md` — update `docs/PRODUCT_CANON.md` §3.5 + §3.9 + §3.10 to name `nebula-schema` canonical. Ships with #12 per §11.6 (no README drift).

---

### Shared follow-up (after both tracks)

**Sprint C — quality & SRP (1–2 weeks):**

14. `2026-05-09-engine-split.md` — god file refactor. (§3.1)
15. `2026-05-10-validator-rule-split.md` — split 2 549-line `rule.rs` into `rule/{types,eval,serde}`.
16. `2026-05-11-workflow-layering.md` — move `InterfaceVersion` to `nebula-core`. (§3.2)
17. `2026-05-12-runtime-panic-replacement.md` — 4 panic sites → typed errors. (§3.3)
18. `2026-05-13-config-core-split.md` — `ConfigStore` / `ConfigLoader` split.

**Sprint D — tests & hygiene (ongoing):**

19. `2026-05-16-telemetry-test-suite.md`.
20. `2026-05-17-trybuild-macro-tests.md` — all derive macros.
21. `2026-05-18-config-macros-decision.md` — delete or rescue.
22. `2026-05-19-system-platform-tests.md`.

**Note:** `parameter.rs` god-file split **dropped** — the crate goes away in Track B.

---

### Sprint E — adopt spec-16 storage row model (deferred, 2–4 weeks)

Separate design track, **not** part of this audit's closure. Added after A2.4 investigation revealed that the `repos/*` layer is a product-architecture redesign, not a mechanical migration (see §2.3).

**Scope:**

23. `YYYY-MM-DD-spec-16-inmemory-impls.md` — implement `InMemoryExecutionRepo`/`WorkflowRepo`/`WorkflowVersionRepo`/`ExecutionNodeRepo`/`JournalRepo` against the new trait signatures. Pure data structures, no consumers yet.
24. `YYYY-MM-DD-spec-16-engine-refactor.md` — rewrite `crates/engine` against `repos::*` traits. Split `ExecutionState` JSON blob into `(status column, patch)`, introduce `workspace_id`/`org_id` threading, rework node-output and idempotency key models, rework checkpoint model.
25. `YYYY-MM-DD-spec-16-api-refactor.md` — rewrite `crates/api` against `repos::*` traits. Split workflow persistence into `WorkflowRow` + `WorkflowVersionRow`. Decide multi-tenancy source (default single-tenant dev workspace? auth-provided? test scaffold?).
26. `YYYY-MM-DD-spec-16-postgres-impls.md` — wire Postgres implementations of all new repos. Validate feature matrix.
27. `YYYY-MM-DD-spec-16-delete-layer-1.md` — once every consumer is migrated, delete `execution_repo.rs`, `workflow_repo.rs`, `backend/`. Update canon §11.5 persistence matrix to reflect the new row model.
28. `YYYY-MM-DD-spec-16-canon-update.md` — update `docs/PRODUCT_CANON.md` §11.5 table + §3.10 storage description. Announce in release notes (canon §7.2).

**Pre-requisite:** a design decision on multi-tenancy defaults for single-binary local-first path (canon §12.3) — spec-16 demands `workspace_id`/`org_id` everywhere, local-first wants zero config. Reconcile before step 23.

**Explicit non-goal for Sprint E:** retaining any legacy `ExecutionRepo`/`WorkflowRepo` API surface after step 27. The point of Sprint E is to eliminate layer 1.

---

## 9. Explicitly out of scope

- Performance profiling / hot-path optimization (deferred — correctness first).
- FFI/stabby plugin path (canon-flagged as experimental; not a knife blocker).
- Desktop app (`apps/desktop`) — not audited here.
- Rotation subsystem adversarial tests — already marked P2 in credential crate, follow-up.
- New crates / new features — this audit is **subtractive** and **consolidating**, not additive.
- Benchmark regressions — covered by CodSpeed on existing paths; not a canon violation today.

---

## 10. Canon advancement summary

This audit, if adopted, advances these canon sections toward "implemented" status: §10, §11.2, §11.4, §12.1, §12.2, §12.5, §12.7, §13, §14, §3.5, §3.9.

**Required canon revisions (at end of Sprint 4):**

- **§3.9** — rewrite naming `nebula-schema` as the canonical typed-config crate. `nebula-parameter` is deleted, not deprecated.
- **§3.10 table** — replace the `nebula-parameter` row with `nebula-schema`; update the one-line role to reflect the Pattern 4 architecture (enum wrapper + per-type structs, compile-time-safe, unified rules).
- **§3.5 prose** — update the "parameter subsystem" sentence to reference `nebula-schema::Schema` instead of `ParameterCollection`, and the "`*Metadata + ParameterCollection`" phrasing to "`*Metadata + Schema`."
- Package these §3.9/§3.10/§3.5 updates into the **final** migration PR (#14 in the roadmap) so canon and code converge in the same commit — no README drift (canon §11.6).

No other canon revisions are required if the remaining §7 open questions resolve inside existing canon language.
