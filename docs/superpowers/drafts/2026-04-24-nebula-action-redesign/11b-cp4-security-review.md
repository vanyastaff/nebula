# CP4 security review — Tech Spec §16.3 DoD must-have floor + §15 §6.x closure (nebula-action redesign)

**Reviewer:** security-lead
**Date:** 2026-04-24
**Document reviewed:** [`docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md`](../../specs/2026-04-24-nebula-action-tech-spec.md) — §16.3 DoD (lines 2401-2417) + §15 §6.x closure rows (lines 2236-2253) — focused review per orchestrator brief
**Mode:** confirmation review (no new security surface introduced at CP4 §14-§16; floor authority already binding via CP2 [`09c-cp2-security-review.md`](09c-cp2-security-review.md) + Phase 2 [`03c-security-lead-veto-check.md`](03c-security-lead-veto-check.md) §2)

---

## Verdict

**ACCEPT.** §16.3 DoD item 2 enumerates all four must-have floor items as cascade-landing-PR obligations with verbatim hard-removal language for CR3 fix; §15.1 closure rows resolve every §6.x carry-forward to either CLOSED or FLAGGED-NOT-ENACTED (with named owner + trigger + sunset window for the two cross-crate amendments). No silent deferral. **VETO authority on shim-form drift retained intact** — three independent anchors (§0.2 invariant 3, §6.2.3, §16.3 item 2) reinforce one another. **No freeze-blocker.**

---

## §16.3 DoD must-have floor verification

§16.3 item 2 (lines 2406-2410) maps 1:1 to my CP2 §6 sign-off (09c lines 125-133):

| Must-have floor item | CP2 §6 spec | §16.3 DoD bullet | Status |
|---|---|---|---|
| **CR4 / S-J1 — JSON depth cap** | §6.1 (depth cap 128 + `ValidationReason::DepthExceeded { observed, cap }`) | "depth cap 128 at every adapter JSON boundary per §6.1; typed `ValidationReason::DepthExceeded { observed, cap }` per §6.1.3" | **VERIFIED — cascade-landing PR obligation** |
| **CR3 / S-C2 — hard removal of no-key `credential<S>()`** | §6.2.2 Option (a) hard delete; §6.2.3 verbatim VETO quote | "hard removal of `CredentialContextExt::credential<S>()` per §6.2 (NOT `#[deprecated]` shim — `feedback_no_shims.md` + security-lead 03c §1 VETO retained)" | **VERIFIED — cascade-landing PR obligation; shim-form rejection explicit** |
| **`ActionError` Display sanitization** | §6.3 (`redacted_display()` pre-`format!` wrap at error-emit sites) | "`redacted_display(&e)` wrap at every error-emit site per §6.3" | **VERIFIED — cascade-landing PR obligation** |
| **Cancellation-zeroize test** | §6.4 (three sub-tests + per-test `Arc<AtomicUsize>` probe) | "three sub-tests per §6.4.1; per-test `Arc<AtomicUsize>` probe per §6.4.2" | **VERIFIED — cascade-landing PR obligation** |

**None deferred to follow-up.** §16.3 framing ("the cascade-landing PR is DONE when ALL of the following land in cascade scope") plus the explicit `feedback_active_dev_mode.md` invocation at §16.3 line 2403 ("never settle for green tests / cosmetic / quick win / deferred") makes follow-up deferral structurally precluded.

**Cross-checked DoD item 6** (`nebula-redact` workspace integration) lands atomic with cascade PR — the redact crate is the host for `redacted_display()` per §6.3.2, so item 3 (sanitization) cannot ship without item 6. Atomicity preserved.

**Cross-checked DoD item 1** (all 11 🔴 closed in code) is the umbrella; CR3 and CR4 are within those 11. Item 2 is the *security-test-verification* layer on top of item 1, not a substitute. No double-counting hazard.

---

## §15 §6.x closure check (no silent deferral)

§15.1 CP2 carry-forward table (lines 2236-2253) closes every §6.x open item:

| §6.x item | §15.1 closure status | Silent-deferral risk |
|---|---|---|
| §6.1.2 — byte-pre-scan vs `Value`-walking | **CLOSED CP2 §6.1.2** (byte-pre-scan path) | None — committed mechanism |
| §6.2-1 — `credential_typed::<S>(key)` retention | **CLOSED CP3 §9.3.1** (REMOVE alongside `credential<S>()`) | None — stronger than CP2 (CP2 left open) |
| §6.3-1 — full `redacted_display()` rule set | **CLOSED CP3 §9 design + CP2 §6.3.1-A** (pre-`format!` wrap-form is the single rule; substring patterns evolve in `nebula-redact` post-cascade) | None — single audit-point preserved |
| §6.4-1 — `tokio::time::pause()` vs real-clock | **CLOSED CP3 §9** (recommendation `pause()` for deterministic timing) | None |
| §6.4 cross-crate amendment к credential §15.7 | **FLAGGED NOT ENACTED — see §15.4** (named owner: credential Tech Spec author; trigger: CP4 cross-section pass; sunset ≤1 release cycle) | None — explicit owner + trigger + sunset per `feedback_active_dev_mode.md` |
| §6.5 — cross-tenant `Terminate` boundary | **CLOSED CP3 §9.5** (engine-side enforcement; verbatim "MUST NOT propagate" invariant; security-lead implementation-time VETO retained) | None — closure documented in 10c-cp3-section95-review |

**§15.4 cross-crate amendment** is the only §6.x item still in flight. It is **flagged-not-enacted with full triage metadata** (owner / trigger / sunset window) per `feedback_active_dev_mode.md` "no silent deferral" floor — this is the documented cross-cascade amendment shape, not a silent skip. The amendment is for the test-only constructor variant on `SchemeGuard<'a, C>`; the cancellation-zeroize test ships gated `#[cfg(feature = "test-helpers")]` until the credential-side feature lands. This is acceptable: the test surface ships in cascade scope (§16.3 item 2 fourth bullet), and the gating mechanism preserves DoD verification at integration time.

**§6.2-1 closure** (CP3 §9.3.1 REMOVE `credential_typed::<S>(key)` alongside `credential<S>()`) is **stronger than CP2 framing**. CP2 §15 carry-forward (line 2161) listed retention vs removal as open; CP3 picked REMOVE. Tightens the API surface — fewer methods to misuse, fewer audit-points. Endorsed.

**No §6.x item silently deferred.** Every row has a closure point or a §15.4 explicit cross-crate handoff with sunset window.

---

## VETO retention check (shim-form drift)

Three independent anchors reinforce the hard-removal stance — drift to `#[deprecated]` would have to clear all three independently:

1. **§0.2 freeze invariant 3** (line 43): "Security floor change. Any of the four invariant items in §4 (per Strategy §2.12 + §4.4) is relaxed, deferred, or has its enforcement form softened (e.g., 'hard removal' → 'deprecated shim' — `feedback_no_shims.md` violation)." Triggers freeze invalidation requiring ADR-supersede.

2. **§6.2.3** (line 1233 verified): "This Tech Spec commits to hard-removal. Any implementation-time deviation toward `#[deprecated]` shim form invalidates the freeze per §0.2 item 3 ('hard removal' → 'deprecated shim' — `feedback_no_shims.md` violation') AND triggers security-lead implementation VETO per 03c §1." Cross-references §0.2 invariant + Phase 2 03c §1 VETO threshold.

3. **§16.3 DoD item 2 second bullet** (line 2408): "hard removal of `CredentialContextExt::credential<S>()` per §6.2 (NOT `#[deprecated]` shim — `feedback_no_shims.md` + security-lead 03c §1 VETO retained)." Inline-cited at the DoD verification surface.

**Cross-section consistency check:**
- §1 G3 (line 67) restates the hard-removal commitment in Goals.
- §14.4 closure traceability table (line 2168 — CR3 / S-C2 row) restates the verbatim VETO trigger language citation.
- §15.5.1 (CP4 §15 ENACTED amendment-in-place) does NOT touch §6.2 — the §6.2 hard-removal commitment is unchanged from CP2 ratification through CP4 freeze.

**One §1985-line caveat noted, NOT a regression.** §14.5 Negative-space confirmation discusses a **deprecation cycle** for *future* removals (not the CR3 fix): "Deprecation cycle of one minor release, NOT shim form per `feedback_no_shims.md`. Deprecation is `#[deprecated(since = "X.Y.0", note = "...")]` on the type / method, with a clear migration target named in the note." This is the **post-cascade evolution policy** for *unrelated future deprecations*, not a softening of the CR3 hard-removal commitment. The §14.5 text refers to "Deprecate one minor (X.Y.0); remove the next (X.Y+1.0)" as a general post-cascade policy — which is correct discipline for unrelated post-1.0 evolution. **Does not regress §6.2 hard-removal commitment**: §6.2 is a cascade-landing-PR obligation (current breakage), not a post-1.0 deprecation flow.

**VETO authority retained verbatim.** No regression detected. Implementation-time VETO authority on shim-form drift retained per 03c §1 + §0.2 invariant 3 + §6.2.3 + §16.3 item 2.

---

## Required edits (if any)

**None.** §16.3 item 2 enumerates all four must-have floor items as cascade-landing-PR obligations; §15.1 closes every §6.x item without silent deferral; VETO retention is reinforced across three independent anchors. No mechanical edits, no clarification edits, no scope edits required.

---

## Summary

ACCEPT for §16.3 DoD must-have floor + §15 §6.x closure. §16.3 item 2 verifies all 4 must-have floor items (CR4/S-J1 JSON depth cap, CR3/S-C2 hard removal of `credential<S>()`, `ActionError` Display sanitization via `redacted_display()`, cancellation-zeroize test) as cascade-landing-PR DoD obligations. §15.1 closes every §6.x carry-forward to CLOSED or FLAGGED-NOT-ENACTED with full triage metadata (owner/trigger/sunset). No silent deferral. **Hard-removal commitment for CR3 fix is reinforced across §0.2 invariant 3 + §6.2.3 + §16.3 item 2** — three independent anchors. **VETO authority on shim-form drift retained verbatim** per `feedback_no_shims.md` + 03c §1.

**Verdict: ACCEPT. VETO: no (not triggered). Freeze-blocker: no.**

*End of CP4 security review (§16.3 + §15 §6.x focused).*
