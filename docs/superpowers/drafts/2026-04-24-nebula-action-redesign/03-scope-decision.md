# Phase 2 — Scope decision (locked)

**Date:** 2026-04-24
**Orchestrator:** claude/upbeat-mendel-b30f89
**Protocol:** Co-decision (architect proposer + tech-lead priority decider + security-lead VETO check). 2 rounds required — round 1 produced divergent framing; round 2 resolved with architect's B'+ hybrid evaluated explicitly.
**Inputs:**
- [`03a-architect-scope-options.md`](./03a-architect-scope-options.md) — 4 options proposed (A'/B'/B'+/C')
- [`03b-tech-lead-priority-call.md`](./03b-tech-lead-priority-call.md) — round 1 picked A'; round 2 re-rank with B'+ included: **A' 1st / B'+ 2nd / C' 3rd / B' 4th**
- [`03c-security-lead-veto-check.md`](./03c-security-lead-veto-check.md) — ACCEPT all three (A'/B'/C') with must-have floor; B'+ by implication ACCEPT (phantom typing structurally eliminates S-C2)

---

## 0. Gate decision

**Gate status:** ✅ Phase 2 locked. **Chosen option: A' — Co-landed cascade (action + credential CP6 implementation, design scope only).**

Cascade proceeds to Phase 3 (Strategy Document draft, architect-led).

**No escalation triggered at Phase 2.** Key reframe that resolved the budget concern:

> **Cascade scope = DESIGN closure only.** Per cascade prompt non-goals: "Final implementation (Tech Spec ends at design closure, П1 implementation gated separately per user)." Agent-day estimates in architect's 03a / tech-lead's 03b that exceeded 5-day budget were **implementation costs** (A' ~18-22d full impl, B'+ ~6-8d, B' ~4-5d). Tech Spec **writing** efforts are ~2-4d, all within cascade budget.

Escalation rule 10 (cross-crate freeze) not triggered: A' implements the frozen credential CP6 spec rather than revising it. A' Tech Spec will describe action + credential CP6 design together; post-cascade implementation plan is user's call.

---

## 1. Scope summary — what's locked

### Chosen shape: Option A' — Co-landed action + credential CP6 design

**Tech Spec covers design for:**

1. **Credential crate CP6 vocabulary implementation design** — `CredentialRef<C>` typed handle, `AnyCredential` object-safe supertrait (partially landed; noted in tech-lead's round 2 as `crates/credential/src/contract/any.rs`), `SlotBinding` with HRTB `for<'ctx> fn(...) -> BoxFuture<'ctx, _>` `resolve_fn`, `SchemeGuard<'a, C>` RAII (`!Clone`, `ZeroizeOnDrop`, `Deref`), `SchemeFactory<C>` re-acquisition arc, `RefreshDispatcher::refresh_fn` HRTB.

2. **Action crate CP6 adoption design** — new `#[action]` attribute macro (replacing `#[derive(Action)]`) with **narrow declarative rewriting contract** (rewriting confined to `credentials(slot: Type)` / `resources(slot: Type)` attribute-tagged zones; NOT arbitrary field rewriting — per tech-lead §2 architectural coherence constraint). `CredentialRef<C>` field support; `ActionSlots` impl emission; new `ActionContext` methods matching CP6 spec per v2 spec §3 (`ctx.credential::<S>(key)` / `credential_opt::<S>(key)`).

3. **Engine wiring design** — `resolve_as_<capability><C>` helpers, slot binding registration at registry time, HRTB fn-pointer dispatch at runtime, depth-cap at all adapter JSON boundaries (must-have §5).

4. **Security hardening (security-lead must-have floor §5 — non-negotiable):**
   - JSON depth cap (128) at `StatelessActionAdapter::execute`, `StatefulActionAdapter::execute`, API webhook body deserialization
   - Replace S-C2 type-name heuristic with explicit keyed dispatch (method signature surgery, not `#[deprecated]`)
   - Sanitize `ActionError` Display path in `tracing::error!` call sites
   - Add cancellation-zeroize test (closes S-C5)

5. **Phase 1 tech-lead solo-decided calls (ratified in cascade scope):**
   - Seal `ControlAction` + canonize DX tier in canon §3.5 as "erases to primary"
   - Feature-gate **AND** wire `ActionResult::Terminate` (apply `Retry` discipline; `feedback_active_dev_mode` — no gate-only-and-defer)
   - Modernize `*Handler` HRTB boilerplate to single-`'a` + type alias (rust-senior §6; optional but recommended)

6. **Plugin ecosystem migration design** — codemod script for `#[derive(Action)]` → `#[action]`; migration guide for 7 reverse-deps; `sdk::prelude` re-export block reshuffle. Design only; execution gated by user post-cascade.

7. **Cluster-mode hooks design** — 3 hooks on `TriggerAction` (`IdempotencyKey`, `on_leader_*` lifecycle, `dedup_window` metadata) per tech-lead §4. Surface contract only; engine cluster coordination implementation out of scope.

8. **Workspace hygiene (absorbed as cascade hygiene bundle):**
   - `zeroize` workspace=true pin (T4)
   - Retire `unstable-retry-scheduler` dead feature (T2) OR convert to a gated variant with actual wiring (depends on Terminate scheduler design)
   - `deny.toml` layer-enforcement rule for `nebula-action` (T9)
   - Lefthook pre-push parity with CI required jobs (T5, per `feedback_lefthook_mirrors_ci.md`)

---

## 2. Out of scope (explicit, with sub-spec pointers)

The following are DEFERRED with sub-spec / separate-cascade pointers. Per `feedback_active_dev_mode`: "before saying 'defer X', confirm the follow-up has a home."

| Deferred item | Sub-spec pointer | Reason |
|---|---|---|
| DataTag hierarchical registry (58+ tags) | Future port-system sub-cascade | Net-new surface; orthogonal to action core |
| `Provide` port kind | Same sub-cascade as DataTag | Net-new; not cascade-gating |
| Engine cluster-mode coordination implementation (leader election, reconnect orchestration) | Engine cascade (TBD — tech-lead to schedule) | Engine-layer concern; action surfaces hooks only |
| `Resource::on_credential_refresh` full integration | Absorbed into resource cascade or co-landed with credential CP6 implementation | Depends on `SchemeFactory` availability |
| Post-cascade implementation of CP6 vocabulary | User decision: (a) single co-landed PR, (b) sibling cascades (credential leaf-first + action consumer-second), (c) phased rollout | Implementation not in cascade scope per prompt non-goals |
| **Adjacent finding T3** — dead `nebula-runtime` reference in `test-matrix.yml:66` + CODEOWNERS:52 | Separate PR (not action cascade's scope) | Filed at cascade end |

---

## 3. Security must-have floor (from 03c, load-bearing)

**Non-negotiable; in cascade scope regardless of all other decisions. Phase 3 Strategy + Phase 6 Tech Spec must cite these as not-deferrable.**

1. **CR4 / S-J1 JSON depth bomb** — depth cap (128) at every adapter JSON boundary.
2. **CR3 / S-C2 cross-plugin shadow attack** — replace type-name heuristic with explicit keyed dispatch at method signature level. **CR3 fix MUST be hard removal**, not `#[deprecated]` shim (per security-lead 03c §1 Option B' conditions; applies to A' by default because A' removes the method class entirely).
3. **ActionError Display sanitization** — route through `redacted_display()` helper in `tracing::error!` paths to preempt S-C3 / S-O4 leak class.
4. **Cancellation-zeroize test** — closes S-C5; pure test addition, no architectural cost.

Deferred (security 🟠 findings — acceptable with sunset commit):

- **S-W2** `SignaturePolicy::Custom(Arc<dyn Fn>)` — webhook hardening cascade within 2 release cycles
- **S-C4** detached spawn zeroize defeat — absorbed into credential CP6 landing (if A' implementation proceeds) OR standalone credential cascade
- **S-O1/S-O2/S-O3** output pipeline caps — output-pipeline hardening cascade
- **S-I2** `CapabilityGated` documented-false-capability — sandbox phase-1 cascade
- **S-W1 / S-W3 / S-F1 / S-I1 / S-U1 / S-C1** — minor defense-in-depth; cascade exit notes

---

## 4. Stakeholder positions (archived)

- **architect** (03a) — leans B'+ as draft position; ready to draft Strategy under whichever option selected. Will draft A' per chosen scope. Spike confirmed for Phase 4: HRTB fn-pointer + `SchemeGuard` cancellation drop-order.
- **tech-lead** (03b round 2) — picks A' 1st, B'+ 2nd, C' 3rd, B' 4th. Round 2 reframe accepted: cascade = design scope only; budget not the blocker. B'+ was acceptable fallback but A' preferred for spec-correctness.
- **security-lead** (03c) — ACCEPT all; must-have §5 floor non-negotiable. No VETO at scope time. **Implementation-time VETO retained** on `#[deprecated]`-instead-of-hard-removal for CR3 fix (flagged to Phase 6 Tech Spec and implementation phases).

---

## 5. Phase 3 handoff — architect Strategy Document

**Architect is now primary writer**, routing through checkpoint cadence per cascade prompt §Phase 3:

- CP1: §1 problem statement + §2 constraints (ADR-refs, PRODUCT_CANON refs, credential CP6 composition) + §3 options analysis → review
- CP2: §4 recommendation + §5 open items + spike plan → review
- CP3: §6 post-validation roadmap → freeze

Per-CP: architect draft → spec-auditor audit pass → tech-lead review (solo decider mode) → iterate once. Escalate if divergence after 2 rounds.

**Strategy Document must cite:**
- This scope decision (A' chosen; must-have §5 non-negotiable; OUT-of-scope markers from §2)
- ADR-0035 phantom-shim capability pattern (existing ratified ADR)
- Credential Tech Spec CP6 §§2.7 / 3.4 / 7.1 / 15.7 as authoritative shape source
- Canon §3.5 (trait family) + §0.2 (canon revision process) — A' triggers canon revision for §3.5 DX tier ratification
- Prior v2 design docs: `2026-04-06-action-v2-design.md`, `2026-04-06-resource-v2-design.md`

---

## 6. Implementation path (post-cascade — USER DECISION)

Explicitly NOT decided in this cascade. Cascade produces the design Tech Spec; user picks implementation path when ready:

- **(a)** Single coordinated PR landing both crates CP6 + engine wiring + plugin migration (15-22 agent-days per architect 03a §1 A' estimate; tech-lead round 1 §1 estimate 8-12d — discrepancy reflects whether codemod + plugin migration are counted)
- **(b)** Sibling cascades — credential CP6 implementation cascade lands leaf-first; action consumer cascade lands second in lockstep. Each fits normal autonomous budget.
- **(c)** Phased rollout with intermediate B'+ surface commitment — action ships CP6 API surface with delegating internals while credential cascade lands CP6 internals; plugins do not re-migrate.

Orchestrator flags (a/b/c) for user when cascade summary lands (Phase 8).

---

## 7. Cross-crate coordination flags

- **nebula-credential**: A' implementation requires CP6 §§2.7/3.4/7.1/15.7 phantom + HRTB + RAII layer landing. `AnyCredential` already landed (`crates/credential/src/contract/any.rs`); `CredentialRef<C>` / `SlotBinding` / `SchemeGuard` / `SchemeFactory` / `RefreshDispatcher` remain spec-only.
- **nebula-engine**: 27+ import sites; `resolve_as_<capability><C>` helpers, slot binding registry, HRTB fn-pointer dispatch, `Terminate` scheduler integration design.
- **nebula-sandbox**: dyn-handler ABI (process / in-process runners) — if `*Handler` HRTB modernization lands, ABI surface re-shapes (non-breaking for runtime; visible in rustdoc / cargo-semver-checks).
- **nebula-sdk**: `prelude.rs` re-export block fully reshuffled on macro / context method renames.
- **nebula-api**: webhook trait surface stable; `SignaturePolicy::Custom` design note (deferred to webhook hardening cascade).

---

## 8. Pointers

- Phase 0 ground truth: [`01-current-state.md`](./01-current-state.md)
- Phase 1 pain: [`02-pain-enumeration.md`](./02-pain-enumeration.md)
- Phase 2 co-decision inputs: [`03a-architect-scope-options.md`](./03a-architect-scope-options.md), [`03b-tech-lead-priority-call.md`](./03b-tech-lead-priority-call.md), [`03c-security-lead-veto-check.md`](./03c-security-lead-veto-check.md)
- Cascade log: [`CASCADE_LOG.md`](./CASCADE_LOG.md)

---

*End of Phase 2 scope decision. Cascade proceeds to Phase 3 (Strategy Document, architect-led, CP1-CP3 cadence).*
