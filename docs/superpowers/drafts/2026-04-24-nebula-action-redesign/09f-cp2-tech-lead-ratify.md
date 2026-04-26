# Tech Spec CP2 — tech-lead ratification (post-iteration)

**Reviewer:** tech-lead (solo decider on G6 / §2.7.1 / §6 co-decision items per Strategy §6.3 line 386-394 reviewer matrix; co-decider WITH security-lead on §6 floor implementation forms per Strategy §4.4 + 03c §1 VETO)
**Date:** 2026-04-24
**Document reviewed:** [`docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md`](../../specs/2026-04-24-nebula-action-tech-spec.md) — CP2 post-iteration (5-reviewer matrix + user §2.9 reconsideration)
**Inputs cross-checked:**
- [`09c-cp2-security-review.md`](09c-cp2-security-review.md) — co-decision sign-off YES on all 4 §6 items + 3 required edits (now applied per CHANGELOG line 1614 / 1615)
- [`09b-cp2-rust-senior-review.md`](09b-cp2-rust-senior-review.md) line 11, 51-67, 102, 107, 111 — §5.3-1 deny.toml wrappers amendment path committed
- [`09a-cp2-tech-spec-audit.md`](09a-cp2-tech-spec-audit.md), [`09d-cp2-dx-review.md`](09d-cp2-dx-review.md), [`09e-cp2-devops-review.md`](09e-cp2-devops-review.md) — closures verified via CHANGELOG entries
- ADR-0039 §1 line 50-54 (verified `SlotBinding { ..., capability: Capability::Bearer, ... }` shape divergence is real)
- Memory: `decision_terminate_gating`, `decision_action_techspec_cp1_ratify`, `decision_action_strategy_cp2_ratify` (load-bearing for ratification continuity)

---

## Ratification verdict (RATIFY / RE-ITERATE / ESCALATE)

**RATIFY — commit-ready, no round-2.**

CP2 iteration absorbed every reviewer required-edit class (security-lead 09c three required edits, rust-senior 09b three nits, dx-tester 09d two items, devops 09e two items, spec-auditor 09a three numbered items) plus the user §2.9 reconsideration. The CHANGELOG (lines 1606-1623) maps each closure to verified reviewer finding. §6 co-decision items lock concrete implementation forms with security-lead's three required edits applied verbatim. §2.9 refined REJECT preserves Phase 1 verdict with a tightened rationale that resolves the user's RSS/Kafka pushback principally rather than defensively. ADR-0039 §1 amendment-in-place trigger correctly deferred to Phase 8 with explicit re-pin fallback rejected by cross-crate authoritative-source rule.

No VETO trigger fired. Implementation-time security-lead VETO authority on §6.2 shim-form drift retained per 03c §1 + §1 G3 + §0.2 invariant 3.

---

## §2.9 refined REJECT ratification (user-raised reconsideration)

**RATIFY** — Configuration vs Runtime Input axis is the right principled distinction.

The user's RSS url+interval / Kafka channel example surfaced a real lifecycle artefact (per-instance configuration). Architect's response in §2.9.1a + §2.9.6 prelude is **structurally honest, not rhetorical**:

1. **Configuration carrier is `&self`.** Tech Spec §4.2 + `tests/execution_integration.rs:155` precedent (`NoOpTrigger { meta: ActionMetadata }`) is real — RSSTrigger composes identically with ordinary fields under `#[action]`'s "fields outside zones pass through unchanged" semantics.
2. **Configuration schema flows through `ActionMetadata::parameters` universally.** §4.6.1 binding `#[action(parameters = T)]` to `with_schema(<T as HasSchema>::schema())` per `crates/action/src/metadata.rs:292` is **not Trigger-specific** — works on all 4 variants (the existing `for_stateless`/`for_stateful`/`for_paginated`/`for_batch` helpers at metadata.rs:140-222 are convenience shortcuts that derive from `A::Input`, but `with_schema` is the universal mechanism). The absence of `for_trigger` is a discoverability gap (now tracked at open item §2.9-1 → CP3 §7), not a structural rebuttal of REJECT.
3. **Runtime Input divergence is the real axis.** `TriggerAction::handle`'s parameter is `<Self::Source as TriggerSource>::Event` projected from the source — fundamentally different lifecycle from "user-supplied per-dispatch parameter." Forcing `Action<I, O>` parameterization on Trigger requires lying (`type Input = ()`) or redundant projection — the more-ideal shape per `feedback_active_dev_mode.md` is to let each trait read as what it actually is.

**Sanity-check the principle.** This matches my `decision_terminate_gating` reasoning style — "the Phase 1 finding names the structural property; the spec resolves it principally rather than smoothing it over." Spike `final_shape_v2.rs:209-262` validates the shape end-to-end at `c8aef6a0`. Re-open triggers in §2.9.7 (fifth primary at four-of-five sharing the shape, OR concrete typed-reflection consumer) are correctly framed as future-ADR concerns, not current Tech Spec scope.

No edit required. The §2.9.5 verdict annotation + §2.9.6 prelude addition correctly retrofit the axis-naming without re-opening the verdict.

---

## §6 co-decision sign-off (4 floor items)

**RATIFY all 4 items — agreement with security-lead 09c verbatim.** No §6 disagreement.

| Item | Security-lead 09c | Tech-lead position | Agreement |
|---|---|---|---|
| §6.1 JSON depth cap (S-J1 / S-J2) | YES — pending §6.1-A + §6.1-B (now applied) | RATIFY — pre-scan via existing `check_json_depth` is the right mechanism choice; `pub(crate)` visibility commit + typed `DepthCheckError { observed, cap }` return-shape close the single-audit-point property + observability DoD per `feedback_observability_as_completion.md`. Three apply sites (stateless input, stateful input, stateful state) verified at line numbers. | **AGREE** |
| §6.2 Hard-remove no-key `credential<S>()` (S-C2 / CR3) | YES — VETO retained verbatim; Option (c) interim escape hatch removed; stronger than CP1 §Gap 1 framing | RATIFY — §6.2.2 commits Option (a) hard-delete; §6.2.3 quotes 03c §1.B + 03c §4 VETO trigger language verbatim; closing sentence re-asserts §0.2 invariant 3 freeze trigger AND security-lead implementation VETO. This is `feedback_no_shims.md` + `feedback_hard_breaking_changes.md` discipline at full strength. | **AGREE** |
| §6.3 `ActionError` Display sanitization (S-O4 / S-C3) | YES — pending §6.3-A pre-`format!` sanitization (now applied as §6.3.1-A) | RATIFY — `nebula-redact` NEW dedicated crate is correct call (single audit point + correct layering — redaction is content-rule orthogonal to logging facade; CODEOWNER alignment for security-critical surface). §6.3.1-A pre-`format!` wrap-form is the load-bearing fix — Display impl is the leak surface, not the outer string. CP3 §9 enumerates full apply-site list. | **AGREE** |
| §6.4 Cancellation-zeroize test (S-C5) | YES — closes 08c §Gap 4 cleanly | RATIFY — per-test `Arc<AtomicUsize>` instrumentation is the correct call (no cross-test contamination, no `serial_test::serial` slowdown). `engine_construct_with_probe` test-only constructor variant gated `#[cfg(any(test, feature = "test-helpers"))]` — production constructor unchanged; clean. Test location at `crates/action/tests/cancellation_zeroize.rs` (integration layer, not embedded in `testing.rs` public surface) honors `feedback_boundary_erosion.md`. | **AGREE** |

**No softening attempted, no security position requires escalation.** §6 is freeze-grade for tech-lead per Strategy §4.4 + §1 G3 + §0.2 invariant 3. Implementation-time VETO authority on §6.2 shim-form drift retained per 03c §1 — Tech Spec ratification does not consume that authority, it preserves it.

---

## R5 ADR-0039 amendment-in-place defer check

**RATIFY DEFERRAL — does not violate `decision_terminate_gating`-style "finish partial work" mindset.**

The trigger (Tech Spec §15 line 1587) is structurally different from the Terminate gate-and-defer pattern that `decision_terminate_gating` forbids:

1. **Terminate gate-and-defer was forbidden** because the public API surface (`ActionResult::Terminate`) shipped without engine-end honoring it — a canon §4.5 false-capability violation in the production crate.
2. **ADR-0039 amendment-in-place is a documentation reconciliation**, not a public-API shipping question. ADR-0039 is `proposed` (moves to `accepted` upon Tech Spec ratification per §0.2 line 38). The Tech Spec §3.1 has the correct shape (`SlotBinding { field_name, slot_type, resolve_fn }` per credential Tech Spec §9.4 authoritative source); ADR-0039 §1 example will be amended-in-place during Phase 8 cross-section pass.

**Phase 8 enactment commitment is real, not a TODO comment.** §15 line 1587 explicitly says: "*Phase 8 must enact OR this Tech Spec must re-pin §3.1 to ADR-0039's current shape (rejected — credential Tech Spec §9.4 wins per cross-crate authoritative-source rule).*" The fallback alternative is rejected by the cross-crate authoritative-source rule, which means Phase 8 enactment is the ONLY remaining path — this is `feedback_active_dev_mode.md` discipline ("before saying 'defer X', confirm the follow-up has a home"). The home is Phase 8.

**§0.2 invariant 2 trigger.** §15 line 1587 also notes this is a §0.2 invariant 2 trigger if not landed before Tech Spec ratification — meaning the freeze policy itself enforces enactment before CP4 → FROZEN CP4. Phase 8 cross-section pass is bounded by the freeze gate, not by an open-ended TODO.

No required edit. Defer is principled.

---

## §5.3-1 deny.toml wrappers ratification

**RATIFY — rust-senior 09b NIT 1 closed cleanly.**

Tech Spec §5.3-1 (lines 1023-1034 verified) commits both:
1. **Path 1 chosen verbatim per rust-senior 09b line 64 recommendation** — `nebula-engine` as dev-dep on `nebula-action-macros`, NOT stub helpers (which would lose the real-bound verification of Probe 6).
2. **`deny.toml` wrappers amendment shape committed** — `nebula-action-macros` added to the `nebula-engine` wrappers list with inline justification ("dev-only dependency on nebula-engine for compile-fail Probe 6 (real `resolve_as_bearer::<C>` HRTB coercion bound-mismatch verification). Stub-helper alternative loses real-bound verification — see Tech Spec §5.3-1.").

CP3 §9 lands the actual `deny.toml` edit + verifies via `cargo deny check` post-amendment per Tech Spec §5.3-1 closing paragraph. The dev-only direction preserves at runtime — `nebula-action-macros` builds without `nebula-engine` in its production dependency closure. This is `feedback_boundary_erosion.md`-compliant: the boundary erosion is acknowledged explicitly with an inline reason, not silently absorbed.

No required edit.

---

## CP3 §2.9-1 forward-track placement

**RATIFY — `ActionMetadata::for_trigger::<A>()` helper at CP3 §7 ActionMetadata field-set lock is correct placement.**

Three reasons:

1. **Topical fit.** §2.9-1 is a metadata-builder convenience-layer question (does the convenience helper API surface need a Trigger-shaped row?), not a trait-shape or runtime-model question. CP3 §7 is the ActionMetadata field-set lock — that's where the metadata-builder API is finalized per Strategy §5.1.1.
2. **Non-blocking.** The universal `with_schema` builder works for all 4 variants today (per `crates/action/src/metadata.rs:292`); a Trigger-shaped helper is discoverability sugar, not a structural gap. Deferring to CP3 §7 doesn't block CP2 lock or CP3 §6/§7/§8 drafting.
3. **Speculative-DX risk acknowledged.** §2.9.1a closing paragraph names the alternative ("a separate `type Config: HasSchema` associated type purely for the helper's discoverability — narrow speculative-DX risk per `feedback_active_dev_mode.md`"). CP3 §7 is the right place to make that call with the full ActionMetadata field-set in view.

No required edit. CP3 §7 placement aligns with Strategy §5.1.1 deadline framing for ActionContext API location resolution (same checkpoint).

---

## Required edits (if any)

**NONE.** All three security-lead 09c required edits (§6.1-A, §6.1-B, §6.3-A) are applied verbatim to the Tech Spec per CHANGELOG entries:
- §6.1-A → §6.1.2-A (`pub(crate)` visibility commit) at line 1150-1152
- §6.1-B → §6.1.2-B (`Result<(), DepthCheckError { observed, cap }>` return-shape) at line 1154-1169
- §6.3-A → §6.3.1-A (pre-`format!` sanitization wrap-form) at line 1263-1278

Rust-senior 09b NIT 1 (§5.3-1) is closed via the `deny.toml` wrappers amendment commitment at lines 1024-1034. Spec-auditor 09a items #1/#2/#3 closed via §7.2 dual-amendment paragraph (line 1416-1421), §6 header authority sourcing correction (line 1123-1126), and §6.1 cap=128 attribution correction (line 1134). Dx-tester 09d items closed via §4.1.3 cross-zone collision invariant + §5.4-companion regression-lock probe. Devops 09e items closed via macrotest 1.2 pin update across §5.1/§5.3/§5.5 + trybuild three-consumer rationale correction at §5.1.

CP2 is commit-ready as-is. **No round-2 needed.**

---

## Summary

**RATIFY — commit-ready, no round-2.** §6 co-decision items (4 floor items) lock concrete implementation forms with security-lead's three required edits applied verbatim; agreement with security-lead 09c is full and unanimous — no escalation. §2.9 refined REJECT preserves Phase 1 verdict with the user's RSS/Kafka pushback resolved principally via Configuration ≠ Runtime Input axis. ADR-0039 §1 amendment-in-place trigger correctly deferred to Phase 8 cross-section pass — fallback re-pin rejected by cross-crate authoritative-source rule, §0.2 invariant 2 enforces enactment before CP4 freeze. §5.3-1 `deny.toml` wrappers amendment commits Path 1 (rust-senior 09b NIT 1 closed). §2.9-1 `for_trigger::<A>()` helper at CP3 §7 ActionMetadata field-set lock is correct placement. Implementation-time security-lead VETO authority on §6.2 shim-form drift retained per 03c §1 + §1 G3 + §0.2 invariant 3. **Orchestrator commits.**

*End of CP2 tech-lead ratification.*
