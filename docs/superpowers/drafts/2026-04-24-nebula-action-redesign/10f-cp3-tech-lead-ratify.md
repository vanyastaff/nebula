---
ratifier: tech-lead (solo decider)
date: 2026-04-24
target: docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md (DRAFT CP3, post-iteration 2026-04-24)
inputs:
  - 10a spec-auditor (PASS-WITH-NITS) — 0 BLOCKER, 2 HIGH, 6 MEDIUM
  - 10b rust-senior (RATIFY-WITH-NITS)
  - 10c security-lead (ACCEPT, no edits)
  - 10d dx-tester (RATIFY-WITH-NITS)
  - 10e devops (RATIFY-WITH-NITS) — 2 critical iter items: nebula-redact + deny.toml syntax
  - architect single-pass iteration (10 edits) consolidated into Tech Spec CHANGELOG-CP3 lines 2276-2288
mode: solo decider — final ratify call on CP3 §9-§13 + post-iteration edits
---

## Ratification verdict (RATIFY / RE-ITERATE / ESCALATE)

**RATIFY (no round-2 iteration).** Commit-ready: **yes**. Escalation flag: **no**.

CP3 §9-§13 post-iteration absorbed all 10 architect edits cleanly: the two devops critical items (nebula-redact §13.4.4 + deny.toml wrappers-list extension §13.4.3) close compile-fail and duplicate-rule blockers, the spec-auditor 🟠 HIGHs (T6 MIXED normalization + §12.3 lib.rs:14→:4) are mechanically resolved, and security-lead 10c ACCEPT preserves §9.5.1 verbatim "MUST NOT propagate" with §9.5.5 VETO-trigger language unchanged. CHANGELOG-CP3 iteration entry (lines 2276-2288) records each fix with reviewer attribution; status header `DRAFT CP3 (iterated 2026-04-24)` correct.

## §13.4 nebula-redact integration check

§13.4.4 absorbs `nebula-redact` workspace integration as cascade-scope-preliminary with four atomic edits: new `crates/redact/{Cargo.toml,src/lib.rs}` + root `Cargo.toml [workspace] members` + `[workspace.dependencies]` + no new `deny.toml` ban (leaf utility). Verified: `crates/redact/` is absent in current workspace tree — the disposition is grounded. The "atomic with cascade PR per `feedback_active_dev_mode.md`" framing closes the CP2 09e gap that CP3 single-pass had silently dropped. §13.4.5 disposition table extended with fourth row for `nebula-redact` consistent with the body. No silent shim shape — substantive `redacted_display` body lands atomic with §11.3.2 / §6.3.1-A call sites per `feedback_no_shims.md`. **Critical compile-fail blocker closed.**

## §13.4.3 deny.toml form check

Verified against current `deny.toml`: existing `nebula-engine` rule lives at `deny.toml:59-66` with active `wrappers = [...]` list. Edit 1 (wrappers-list extension adding `"nebula-action-macros"` to existing list with inline rationale comment) is the syntactically correct cargo-deny shape — a parallel `{ crate = "nebula-engine", ... }` entry would have been a duplicate-rule conflict (architect's pre-iteration shape). Edit 2 (symmetric positive ban for `nebula-action` runtime layer with full reverse-deps wrapper list — engine/sandbox/api/sdk/plugin/cli/macros) closes Phase 0 §11 row 9 T9 full intent and matches the existing api/engine/sandbox/storage/sdk/plugin-sdk pattern at `deny.toml:54-81`. Architecturally correct: nebula-action is the business-trait layer, upward layers must not be runtime deps of it. **deny.toml syntax fix closed.**

## §9.5 cross-tenant Terminate VETO-strength preservation

§9.5.1 line 1609 quotes 08c §Gap 5 line 111 verbatim ("MUST NOT propagate"); §9.5.5 line 1639 retains the implementation-time VETO trigger language with §0.2 invariant 3 escalation path. Security-lead 10c ACCEPT (no edits) confirms RFC 2119 strength preserved — no softening to "should not" or "by default does not". The architect single-pass did not touch §9.5 (correct: zero security-lead edits required). Engine-side enforcement contract (§9.5.2: tenant scope check at scheduler dispatch path before fanning Terminate to siblings; cross-tenant skip silent + observable via `tracing::warn!` + counter; structural errors Fatal) preserves the active-dev-coherent shape: cross-tenant is policy-boundary observable, structural failure is fail-closed. **VETO-strength wording intact.**

## §10 T1-T6 AUTO/MANUAL consistency

T6 row at §10.2 (line 1679) now reads **MIXED** explicitly (AUTO default for trivial pass-through; MANUAL marker on edge-case detection per ADR-0040 §Negative item 4); §10.2.1 "MIXED mode (T6)" enumerates the two-stage behavior (AUTO attempt, MANUAL fallback on edge-case detection: custom Continue/Skip/Retry reasons, Terminate interaction, test fixtures); §10.5 line 1735 third bucket "Mixed" maps T6 with the same reasoning. Three-site internal contradiction (spec-auditor 10a 🟠 HIGH) closed. T1/T3/T4 = AUTO consistent across §10.2 / §10.2.1 / §10.5. T2/T5 = MANUAL-REVIEW consistent. §10.3 per-consumer step counts use Phase 0 §10 line 346-356 (off-by-one fix landed). `control_flow` attribute syntax unified to flag form across §10.2 / §10.4 / §12.2 per dx-tester 10d R1. §10.4 step 1.5 `semver` Cargo.toml dep instruction landed per dx-tester R2 / Phase 1 CC1 carry-forward. **AUTO/MANUAL/MIXED split self-consistent end-to-end.**

## CP4 forward-track readiness

10-item CP4 forward-track (a)-(j) preserved at lines 2253-2263; engine cascade handoff for §9.5.5 SchedulerIntegrationHook is correctly framed as engine-cascade scope (not Tech Spec scope) — line 2257 forward-track item (d) names `nebula-engine::registry` surface as engine-cascade-handoff and §15.5/§9.5.5 explicitly mark "engine-side scope per §7.4 cross-ref". ADR-0039 §1 amendment-in-place trigger preserved as item (g) line 2260: "Per §0.2 invariant 2, must land before Tech Spec ratification" — this is the §0.2 freeze gate, not a CP3 ratify gate, so does not block this ratification. Phase 8 enacts inline ADR edit + CHANGELOG entry. **CP4 cascade handoff queue clean; ADR-0039 amendment-in-place trigger correctly deferred to Phase 8 freeze gate.**

## Required edits (if any)

**None.** All 10 architect single-pass edits absorbed; remaining spec-auditor 🟡 MEDIUMs are CP4 §14 cross-section housekeeping (glossary entries; bookkeeping nits) per the standard CP cadence and do not block CP3 ratification.

## Summary

CP3 §9-§13 + post-iteration is **commit-ready**. Two devops critical items (nebula-redact workspace integration §13.4.4 + deny.toml wrappers-list extension §13.4.3) close cascade-landing-PR compile-fail blockers per `feedback_active_dev_mode.md` ("finish partial work in sibling crates"). Security-lead 10c ACCEPT preserves §9.5 VETO-strength end-to-end. T6 MIXED normalization + §10 transform mapping disambiguation close spec-auditor 🟠 HIGHs without re-opening Strategy. CP4 forward-track 10-item queue is internally consistent; ADR-0039 §1 amendment-in-place trigger correctly framed as Phase 8 freeze gate, not CP3 ratify gate. No round-2 iteration required; no escalation. Orchestrator commits.

**Handoff: orchestrator** for CP3 commit + CP4 §14-§16 cross-section pass kickoff.
