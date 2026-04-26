# 03 — Scope Decision (LOCKED)

**Phase:** 2 — Scope narrowing (co-decision)
**Date:** 2026-04-24
**Decision body:** architect (propose) + tech-lead (priority-call) + security-lead (security-gate)
**Protocol:** co-decision — converged in round 1 (max was 3). Both reviewers' confidence estimates: 80-90% lock in round 1.
**Status:** **LOCKED** — Phase 3 may proceed.

**Inputs:**
- `03-scope-options.md` — architect's draft (3 options A/B/C + comparison matrix + 6 open questions)
- `phase-2-tech-lead-review.md` — priority-call: Option B + 2 amendments
- `phase-2-security-lead-review.md` — security-gate: BLOCK A, ENDORSE B with 3 amendments, ENDORSE C with same 3

---

## Chosen scope — Option B (Targeted) + merged amendments

**Headline:** Reshape the `Resource` trait to surface credential rotation per Tech Spec §3.6 verbatim. Extract Daemon + EventSource topologies. Split `manager.rs` file (not the type). Rewrite adapter docs. Wire drain-abort fix atomically. Land as 2-3 breaking-change PRs with per-resource migration.

**Convergence evidence:**
- architect recommended B (`03-scope-options.md` recommendation)
- tech-lead priority-called B (`phase-2-tech-lead-review.md`)
- security-lead BLOCKed A (🔴-1 silent revocation drop unfixable by deferral), ENDORSED B with amendments

Phase 2 co-decision body is **unanimously aligned** on Option B.

---

## 1. In-scope Phase 1 findings (addressed by Option B)

| # | Finding | Treatment |
|---|---------|-----------|
| 🔴-1 | Credential×Resource seam structurally wrong | **Primary deliverable.** Adopt Tech Spec §3.6 verbatim: `type Credential: Credential` on `Resource` trait; `type Credential = NoCredential;` idiomatic opt-out for unauthenticated resources. Implement per-resource blue-green swap pattern for `on_credential_refresh`. Atomic landing: reverse-index write + dispatcher + observability in one PR (per security amendment B-agnostic constraint #1). |
| 🔴-2 | Daemon has no public start path | **Extract Daemon + EventSource from crate** (per tech-lead priority-call; architect Option B design). Fold them into engine/scheduler layer OR keep as sibling crate — Phase 3 Strategy decides the target. |
| 🔴-3 | `api-reference.md` fabrication + `adapters.md` compile-fail | **Rewrite docs AFTER trait shape locks** (per dx-tester priority-call from Phase 1). Docs are a Phase 6 deliverable, not Phase 3. |
| 🔴-4 | Drain-abort phase corruption | **Wire `ManagedResource::set_failed()` in `graceful_shutdown::Abort` path.** Absorbed into Option B atomically (tech-lead did not amend to "defer to standalone PR"). |
| 🔴-5 | `Resource::Auth` dead weight | **Removed by §3.6 shape** — `Auth` is replaced by `Credential` per §3.6, not added alongside. Full rename, no deprecation shim (per memory `feedback_no_shims.md`). |
| 🔴-6 | EventSource same orphan-surface pattern as Daemon | **Extracted together with Daemon** (same fate). |
| 🟠-7 | `manager.rs` 2101 L | **Split the file, NOT the type.** Extract options/shutdown/gate-admission submodules. Keep `Manager` as single coordinator type. |
| 🟠-9 | Daemon + EventSource out-of-canon §3.5 | **Canon-consistent** after extraction. `PRODUCT_CANON.md §3.5` definition ("long-lived managed object: connection pool, SDK client") is preserved. |
| 🟠-12 | `register_pooled` silently requires `Auth = ()` | **Absorbed by §3.6** — `type Credential = NoCredential;` becomes the documented unauthenticated path; convenience methods may or may not be symmetric across Credential/NoCredential. Phase 3 Strategy decides final shape. |
| 🟠-14 | Missing observability on credential rotation path | **Phase 6 CP-review gate** (per tech-lead amendment 2 + security B-agnostic #3). Every rotation operation ships with tracing span + counter + `ResourceEvent::CredentialRefreshed` variant. DoD per `feedback_observability_as_completion.md`. |
| 🟠-15 | `Credential`/`Auth` naming contradiction across docs | **Resolved by rewrite** — all docs reflect §3.6 shape consistently after trait locks. |
| 🟡-17 | `warmup_pool` `R::Auth::default()` footgun | **Resolved, not reproduced** (per security amendment B-3). Under new `type Credential` shape, `warmup_pool` must not call `Scheme::default()`. |

**Score:** 6/6 🔴 + 5/9 🟠 + 1/9 🟡 addressed in cascade. Remaining 🟠 + 🟡 either out-of-scope (see §2) or standalone-fix (see §3).

## 2. Out-of-scope Phase 1 findings (deferred or accepted)

Deferred to **future cascade / sub-spec / follow-up project**. Each has an explicit pointer so nothing silently rots.

| # | Finding | Why deferred | Pointer |
|---|---------|--------------|---------|
| 🟠-8 | Reserved-but-unused public API (`AcquireOptions::intent/.tags`, `ErrorScope::Target`, `AcquireIntent::Critical`) | Engine integration (#391) drives the shape. Requires engine-side design first. | Future cascade coordinated with engine team. Mark fields `#[deprecated]` in Phase 6? **Phase 3 Strategy decides.** |
| 🟠-11 | 5-assoc-type friction: `Runtime == Lease` in 9/9 tests | Evidence supports collapse but decoupled from credential driver. Touching it here doubles cascade surface. | Future cascade. Phase 3 Strategy §5 may record as post-validation follow-up. |
| 🟠-13 | Transport topology 0 Manager-level integration tests | Test debt, not structural defect. | Follow-up task — issue filed post-cascade. |
| 🟡-16 | `AuthScheme: Clone` → secret cloneability | Requires cross-crate reshape at credential side. | Credential Tech Spec §3.6+ extension in credential's own cascade. |
| 🟡-18 | `CredentialId` split import | Cosmetic. | Drive-by fix in a future PR. |
| 🟡-19 | `_with` convention inconsistent | Resolved by file-split redesign. | Absorbed (effectively in-scope as cleanup). |
| 🟡-20 | `Resource::destroy` default no-op encourages leaks | Revisit when the trait shape settles in Phase 4 spike. | Phase 4 spike may surface as follow-up. |
| 🟡-21 | `integration/` module name collision | Rename as part of file-split. | Absorbed (effectively in-scope as rename). |
| 🟡-22 | Service vs Transport differentiation thin | Explicit priority-call: "the separation is defensible but low-value… keeping them separate at the trait level buys type-level clarity on `keepalive` and `close_session(healthy: bool)`" — not worth the churn this cascade. | Future cascade if evidence mounts. |
| 🟡-23 | `docs/dx-eval-real-world.rs` unclear purpose | Docs rewrite will address. | Absorbed into Phase 6 doc rewrite. |
| 🟡-24 | `ResourceMetadata` `#[non_exhaustive]` with one field | Cosmetic. | Leave as-is. `#[non_exhaustive]` preserves future-add safety. |
| 🟢-25, 🟢-26, 🟢-27 | Minor inconsistencies | Accepted as-is. | No action. |
| ✅-28 | Positive findings (`#[forbid(unsafe_code)]`, no CVEs, no leakage) | Preserve. | Phase 6 Tech Spec §6 preserves these invariants. |

**Out-of-scope budget:** ~5 of 15 non-🔴 findings deferred; explicit pointers prevent silent drop per `feedback_incomplete_work.md`.

---

## 3. Standalone-fix PRs (outside cascade)

Land independently **before or in parallel with** cascade completion.

| # | Finding | Dispatcher | Complexity | Landing order |
|---|---------|-----------|------------|---------------|
| SF-1 | `deny.toml` wrappers rule for `nebula-resource` | devops | Trivial (5-10 lines TOML) | Before cascade completion (security-lead confirmed still standalone-fix) |

**SF-2 (drain-abort phase corruption)** — originally drafted as standalone candidate. **Now absorbed into Option B** — tech-lead chose to land it atomically with the Manager file-split because the set_failed helper is near shutdown-path code being restructured anyway. Security-lead endorsed either path.

---

## 4. Locked design decisions

### 4.1 Credential reshape — adopt Tech Spec §3.6 VERBATIM

**Answer to architect Q2 (shape):** per tech-lead priority-call, adopt `type Credential: Credential` directly on `Resource` trait; `type Credential = NoCredential;` as idiomatic opt-out; **NO `AuthenticatedResource: Resource` sub-trait**.

- **Rationale (tech-lead):** "§3.6 is the ratified downstream contract, zero in-tree production `impl Resource` sites make migration cheap, and a sub-trait doubles the API learning surface for no benefit."
- **Security-lead endorsement:** "security-neutral relative to sub-trait variant; both fit atomic-landing + observability constraints."
- **Spike implication:** Phase 4 spike validates §3.6 shape. **Sub-trait fallback REMOVED from spike exit criteria.** If §3.6 ergonomics fail the spike, that escalates back to Phase 2, not a mid-flight shape change (tech-lead amendment 1).

### 4.2 Rotation dispatch concurrency — parallel with per-resource failure isolation

**Answer to architect Q3 (concurrency):** parallel dispatch across N resources sharing one credential.

- **Tech-lead:** "Parallel dispatch with per-resource failure isolation, unbounded `join_all` now with a `FuturesUnordered` cap as future optimization if fan-out becomes a concern."
- **Security-lead:** B-1 amendment — "isolation invariant — one resource's failing `on_credential_refresh` must not block sibling dispatches."
- **Merged:** parallel `join_all` (or `FuturesUnordered` for larger fan-outs in future), each per-resource future bounded by its own timeout + error isolation. Strategy §5 records this.

### 4.3 `on_credential_revoke` semantics — extend §3.6

**Security amendment B-2:** "`on_credential_revoke` dispatch is more security-critical than refresh but Tech Spec §3.6 is silent on it; Strategy must extend §3.6 and loop spec-auditor if needed."

- **Strategy §3 (Phase 3) to propose revoke semantics** — likely: destroy current pool instances, reject new acquires until new credential supplied. Revocation ≠ refresh — no blue-green swap possible without new credential.
- **Phase 3 handoff to spec-auditor:** confirm credential Tech Spec §3.6 needs extension or if §3.7+ covers revoke. If extension needed, credential-side spec update is a cascade dependency.

### 4.4 Observability — DoD, not follow-up (tech-lead amendment 2)

**Every rotation-path operation ships with:**
- `tracing::span!` instrumenting the dispatch
- Counter metric (`nebula_resource.credential_rotation_attempts` or similar; Phase 6 CP decides name)
- `ResourceEvent::CredentialRefreshed { credential_id, resources_affected, outcome }` broadcast variant

Phase 6 Tech Spec checkpoint review has an **explicit observability gate** before CP ratification. Per `feedback_observability_as_completion.md`.

### 4.5 `warmup_pool` under new shape

**Security amendment B-3:** "`warmup_pool` must not call `Scheme::default()` under the new `type Credential`."

- Phase 6 Tech Spec §5 (API surface) defines `warmup_pool` signature to explicitly accept a credential parameter.
- Footgun 🟡-17 resolved, not reproduced.

### 4.6 Daemon + EventSource extraction target

**Strategy §4 decides between:**
- (a) Fold into engine/scheduler layer (TriggerAction already covers event-driven ingress; long-running workers belong to scheduler)
- (b) Extract to separate sibling crate (e.g., `nebula-worker` / `nebula-background`)

Both options honor canon §3.5 "resource = pool/SDK client" by removing them from `nebula-resource`. Strategy draft picks one with rationale and evidence from `INTEGRATION_MODEL.md`.

### 4.7 Migration discipline

- **No shims, no adapters, no deprecation windows** per memory `feedback_no_shims.md` + `feedback_hard_breaking_changes.md`.
- **5 in-tree consumers (action, sdk, engine, plugin, sandbox) migrated in same PR wave** as trait reshape. Phase 6 Tech Spec §13 enumerates per-consumer changes.
- **MATURITY = `frontier`** — breaking changes are expected per Nebula stability policy; no external adopters to protect.

---

## 5. Phase 4 spike scope (if triggered)

**Trigger condition:** Phase 3 Strategy §5 decides whether a spike is needed. Likely yes — trait reshape touches all 5 consumers.

**Spike scope (per architect Option B + tech-lead amendment 1):**

- **Iter-1:** minimal `Resource` trait with `type Credential: Credential` per §3.6; compile-fail probes; `type Credential = NoCredential;` idiomatic opt-out verified to compile and call-site looks clean.
- **Iter-2:** compat sketches for 3 of 5 consumers (action, sdk, engine picked as representative); dispatch ergonomics of `on_credential_refresh` parallel `join_all`; basic perf sanity against current pool acquire (no regressions on the happy path).

**Exit criteria:**
- §3.6 shape compiles ✓
- `NoCredential` opt-out compiles and doesn't require `.unwrap()` / footgun syntax at call site ✓
- Parallel refresh dispatch doesn't deadlock in a realistic consumer example ✓
- No large perf regression on happy path (pool acquire) ✓

**If §3.6 shape fails ergonomics or perf criteria:** ESCALATE to orchestrator; Phase 2 round 2 required (reconsider scope — not a mid-flight shape change per tech-lead amendment 1).

---

## 6. Artefact plan (Phases 3-6)

Per architect's Option B.9 estimate + Phase 2 confirmations:

| Phase | Artefacts |
|---|---|
| **Phase 3 Strategy** | `docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md` — CP1 §1-§3 → CP2 §4-§5 → CP3 §6 cadence per credential pattern |
| **Phase 4 Spike** | Isolated worktree; `NOTES.md` + `final_shape_v2.rs` + test artefacts; commit hash recorded |
| **Phase 5 ADR** | At minimum 1 ADR: "`Resource::Credential` adoption and `Auth` retirement". Possibly a second for "Daemon/EventSource extraction decision" if tech-lead wants historical record. |
| **Phase 6 Tech Spec** | `docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md` — 16 sections, CP1 §0-§3 → CP2a §4-§5 → CP2b §6-§8 → CP3 §9-§13 → CP4 §14-§16. Every CP ratified by tech-lead. |
| **Phase 7 Register** | `docs/tracking/nebula-resource-concerns-register.md` — conditional if Phase 1 yielded 🔴 needing 3-stakeholder consensus. **Required** — 4 active 🔴 surface items. 6-label classification. |
| **Phase 7 Consensus session doc** | Likely not required — Phase 2 co-decision converged in 1 round. Defer decision to Phase 6 CP reviews (if deadlock surfaces, trigger then). |
| **Phase 8 Summary** | `docs/superpowers/specs/2026-04-24-nebula-resource-redesign-summary.md` — orchestrator consolidated deliverable |

---

## 7. Budget forecast (Phases 3-8)

Per architect Option B estimate + remaining time in envelope:

| Phase | Estimated agent-effort |
|---|---|
| Phase 3 Strategy (3 CPs × 2-round review) | ~4-6 hours |
| Phase 4 Spike (2 iterations max) | ~4-6 hours |
| Phase 5 ADR(s) | ~1-2 hours |
| Phase 6 Tech Spec (4-5 CPs × review cascade) | ~8-12 hours |
| Phase 7 Register | ~1 hour |
| Phase 8 Summary | ~1 hour |
| **Remainder of 5-day envelope** | **~20 hours** agent-effort |

**Headroom: comfortable.** Phases 0-2 consumed ~70 minutes agent-effort; ~20 hours remain in the 5-day envelope.

---

## 8. Open items flagged for downstream phases

Not blockers — items that Phase 3+ must answer:

1. **Daemon/EventSource extraction target** (Strategy §4) — engine/scheduler fold vs sibling crate
2. **Revoke semantics** (Strategy §3) — extension of §3.6 OR cross-ref to §3.7+ if already covered
3. **`AcquireOptions::intent/.tags` treatment** (Strategy §5 post-validation roadmap) — deprecate? remove? no-op retain?
4. **Manager file-split cut points** (Tech Spec §5) — which helpers go to which submodule
5. **Convenience method symmetry under `NoCredential`** (Tech Spec §5) — do `register_pooled` and friends keep their `Credential = NoCredential` shortcut?
6. **Consumer migration PR wave** (Tech Spec §13) — one atomic PR across 5 consumers, or 5 coordinated PRs with feature flag?

---

## 9. Verdict

**Scope LOCKED. Phase 3 may proceed.**

- Unanimous co-decision on Option B + merged amendments
- No round-2 required
- Clear handoff to Phase 3 Strategy draft (architect-led, CP1-CP2-CP3 cadence)
- Budget headroom comfortable

**Next phase:** Phase 3 — Strategy Document draft (architect-led, following credential Strategy pattern at `docs/superpowers/specs/2026-04-24-credential-redesign-strategy.md`).
