---
reviewer: tech-lead
mode: solo decider (§6 sub-decision ratification per architect CP3 handoff)
date: 2026-04-24
target: docs/superpowers/specs/2026-04-24-action-redesign-strategy.md (DRAFT CP3, §6)
parallel: spec-auditor (structural audit)
---

## Review verdict (RATIFY / RATIFY-WITH-NITS / REVISE)

**RATIFY-WITH-NITS.** One small edit; no re-draft. §6.2 ADR roadmap (3 required + 1 optional), §6.5 a/b/c decision tree, §6.6 slot commitment as path (c) viability gate, and §6.8 B'+ co-decision rule all confirm or honestly extend my Phase 1 / CP1 / CP2 positions. §6 also discharges the §4.3.2 retry-scheduler forward-ref via §6.3 CP3 row ("retry-scheduler chosen path") — Strategy locks principle, Tech Spec CP3 picks path; my `decision_terminate_gating.md` solo call is preserved. The single nit is a missing co-decision-route detail in §6.8 (orchestrator is the surface, not a co-decider) — see Required edits.

## §6 sub-decision ratifications (per focus area)

### 1. §6.2 ADR roadmap (3 required + 1 optional) — RATIFY

The three required ADRs match the Phase 5 expected scope:

- **ADR-00NN trait shape** (`#[action]` attribute macro + narrow-zone rewriting contract) — corresponds to §3.1 component 1 + §4.3 sub-decisions; load-bearing.
- **ADR-00NN+1 macro emission contract** (typed-only credential surface, `Input: HasSchema` documented bound, `parameters` emission corrected, `semver` dep declared) — discharges the §1(b) macro emission bug list (CR2/CR8/CR9/CR11) plus §5.2.3 expansion-perf gate; load-bearing.
- **ADR-00NN+2 ControlAction seal + canon §3.5 DX tier ratification** — confirms my Phase 1 `decision_controlaction_seal.md` solo call (seal ControlAction; keep as internal DX helper; do NOT extend canon §3.5 dispatch primaries). The §6.2 row makes this a canon-revision ADR per `PRODUCT_CANON.md` §0.2 line 27 ("explicit canon revision required to add another action trait"); CR3 fix lands as `feedback_no_shims.md`-compliant removal. The 5-trait DX tier vs 4-trait dispatch core distinction is preserved verbatim from my decision. Good.

The fourth (optional) ADR — TriggerAction cluster-mode hooks — correctly punts on full integration depth: §3.1 component 7 surfaces hooks only, full integration is engine-cluster-mode cascade scope. Optional framing is honest given §3.4 row 3 cluster-mode coordination cascade scheduling commitment.

**Drafting order is sound.** ADR-00NN before ADR-00NN+1 (trait shape grounds emission contract), ADR-00NN+1 before ADR-00NN+2 (emission contract grounds canon revision rationale). ADR-00NN+3 in parallel with ADR-00NN+2 once §3.4 row 3 is committed. This serializes the load-bearing dependencies without over-sequencing — each ADR's context is grounded in a frozen prior, not a draft. Phantom-shim citation (ADR-0035 line 378) is correctly load-bearing for ADR-00NN+1 + ADR-00NN+2.

**No missing ADRs.** §4.3.1 HRTB modernization does not warrant a separate ADR — it's a hygiene cut absorbed into ADR-00NN trait shape (single `'a` + `BoxFut<'a, T>` is mechanically a trait-shape choice, not a new architectural decision). §4.3.2 retry-scheduler is intentionally Tech Spec §9 + CP3 §6 territory, not ADR. §4.3.3 codemod transforms are Tech Spec §9 design, not Strategy ADR. The 3-required + 1-optional cut is the right bar.

### 2. §6.5 post-cascade implementation path criteria (a/b/c decision tree) — RATIFY

The decision tree honestly captures the trade-offs a user would actually weigh:

- **(a) appropriate when** "Credential cascade owner has bandwidth committed (§6.6) AND plugin authors can absorb single-PR review surface" — concrete preconditions + 18-22d aggregate budget. No false confidence; the AND is binary.
- **(b) appropriate when** "Bandwidth split needed across two owners AND tighter per-cascade review surface preferred; per-cascade budgets re-baselined at path selection" — explicitly cites my CP2 hint line 161 (currently aggregate-budget framing → re-baseline at path pick). Honest acknowledgment that path-(b) budgets are not yet enumerated.
- **(c) appropriate when** "Credential cascade slot commitment lapses (§6.6) — i.e., implementation slips beyond cascade close window AND user accepts B'+ contingency activation per §6.8" — correctly phrased as a fallback (not a co-equal path), gated on §6.8 viability rule.

The §6.5 viability gate ("Path (c) requires B'+ activation; B'+ activation is not user-pickable in isolation — see §6.8 co-decision rule") is the right safeguard. Strategy preserves user pick authority on (a)/(b) but doesn't let path (c) be picked on a hand-wave — `feedback_active_dev_mode.md` silent-degradation guard active. This honors my CP1 line 116 / CP2 line 88 silent-degradation guard verbatim. Good.

**No pre-emption of user authority.** Strategy explicitly does not pre-pick at §6.5 ("Strategy does not pre-pick"); orchestrator surfaces choice at Phase 8 cascade summary. CP2 framing preserved.

### 3. §6.6 cross-crate coordination tracking — RATIFY

**Three-field bar is correct.** Named owner + scheduled date + queue position is the right minimum bar — confirms my CP1 line 116 / CP2 line 88 silent-degradation guard. The reasoning chain is sound:

- **Named owner** — without this, "credential cascade owner has bandwidth" reduces to wishful thinking.
- **Scheduled date (absolute, not relative)** — this is the crucial bit. "Post-action-cascade close" is exactly the kind of vague pointer that historically rotted in Nebula (see my memory `feedback_active_dev_mode.md` precedent + `project_adr0013_unimplemented.md` "ADR-0013 accepted but no build.rs / mode-* feature exists in workspace yet"). Absolute date forces the calendar conversation.
- **Queue position** — relative-to-other-queued-cascades framing matches credential Strategy §6.5 queue convention, which already proved out at credential cascade landing.

**Tracking obligation, not gating** is the right framing. The slot doesn't gate Strategy freeze, Tech Spec writing, or cascade close — it gates path-(c) availability only. This is precisely the boundary I'd want: Strategy can lock without resolving cross-team scheduling, but path (c) can't be picked on cascade-queue.md absence.

**Failure mode is explicit and binary.** "If the slot is not committed before Phase 8 user pick, path (c) is NOT VIABLE — the user pick narrows to (a) or (b)." No silent degradation; the user sees the narrowed choice at Phase 8. This is the active-dev discipline applied to contingency surfaces — exactly the right `feedback_active_dev_mode.md` extension.

**Where the slot lands.** `docs/tracking/cascade-queue.md` (or equivalent, orchestrator picks at Phase 8). I verified the file does not yet exist — that's correct ("created at Phase 8 if path (c) is on the table"), matches the §6.4 concerns-register lifecycle pattern ("created at Phase 7 entry; absent if Phase 7 is skipped"). No premature scaffolding. Good.

### 4. §6.8 B'+ contingency activation criteria — RATIFY-WITH-NITS

**Activation signal (binary OR) — RATIFY.** Two independent failure modes correctly enumerated:
1. Spike iter-1 probe 3 fails → narrow declarative rewriting cannot be enforced compile-time. Detection at Phase 4 spike iter-1; signal is `trybuild` probe 3 negative result + spike `NOTES.md` failure record. This honors my CP2 required-edit #1 (spike fail-state path-c linkage) — extended to full B'+ activation, which is the correct generalization.
2. Credential cascade slot commitment lapses → §6.6 absence at Phase 8. Detection at Phase 8 cascade summary; signal is the absence of the slot row in `docs/tracking/cascade-queue.md`.

The "Signals are not OR-summed" caveat ("the second signal is a function of cascade scheduling, not spike outcome — they're independent failure modes") is the right disambiguation — either signal alone activates B'+, but they're not aggregated.

**Rollback path — RATIFY.** Two A' load-bearing decisions revert:
1. Remove `#[action]` attribute macro adoption (revert §4.3.1 sub-decision).
2. Reactivate `#[derive(Action)]` with macro emission bug fixes from ADR-00NN+1 (CR2/CR8/CR9/CR11 from §1(b)) — accept structural derive limitation.

The "B'+ activation re-opens CR1 + CR7 as known sunset items" with §6.7 rows + sunset window ≤4 release cycles is the correct active-dev discipline applied to contingency surfaces. CR1 (typed credential surface unrealized) and CR7 (canon §3.5 governance debt unresolved) are honestly recorded as 🔴 acknowledged-debt with longer sunset window than 🟠 deferred-tracking. Good — preserves the `feedback_active_dev_mode.md` "follow-up has a home" rule even when the contingency activates.

**Co-decision rule — architect + tech-lead — RATIFY-WITH-NIT.** B'+ activation requires architect + tech-lead co-decision, NOT solo orchestrator. Per `feedback_adr_revisable.md` (point-in-time decisions can be revised) + `feedback_hard_breaking_changes.md` (architecture-level decisions need expert framing). Solo orchestrator activation flagged as `feedback_active_dev_mode.md` silent-degradation violation — correct.

**Nit on co-decision routing.** §6.8 says "the orchestrator surfaces the activation signal, architect frames the rollback in CP3 amendment terms (per §0 amendment mechanics), tech-lead ratifies." This is the right sequence, but it doesn't explicitly say what happens if architect and tech-lead **disagree**. Per my consensus-mode operating rule (system prompt: "if you and another agent disagree, do not silently break the tie — surface both positions for orchestrator to escalate"), the right resolution is: orchestrator escalates to user as tie-break. §6.8 should say this explicitly so future-cascade reviewers don't assume orchestrator silently picks. One sentence; see Required edits.

**Sunset commit for B'+ surface — RATIFY.** ≤4 release cycles, follow-up cascade. Pre-empts B'+ becoming permanent B' degradation. Correct active-dev discipline.

### 5. §4.3.2 retry-scheduler path closure (carried CP2 hint) — RATIFY

§6 honestly closes this forward-ref via §6.3 CP3 row ("Migration, codemod, **retry-scheduler chosen path**, observability spans"). This is the correct authority division:

- Strategy §4.3.2 locks principle (Retry+Terminate symmetric gating; either both wire end-to-end or both stay gated-with-wired-stub; no asymmetric gate-only).
- Tech Spec §9 / CP3 picks path (wire-end-to-end vs gated-with-wired-stub) at design time.
- Strategy §6 holds the placeholder ("CP3 records chosen path" — open items line 466).

This preserves my Phase 1 `decision_terminate_gating.md` solo call ("feature-gate Terminate AND wire end-to-end in redesign cascade; don't gate-only") **as the principle Strategy locks**, while letting Tech Spec resolve the design depth that Strategy can't. Correct division of authority.

**Not a punt to Tech Spec — a deferral with named landing point.** Tech Spec §9 picks; CP3 §6 records the chosen path with canon §11.2 graduate-or-stay edit cited (per my CP2 hint line 153). This is the same pattern as `decision_action_phase_sequencing.md` (Phase 3A→3B→3C cut as single coordinated cascade) — Strategy frames the discipline, Tech Spec picks the cut. Honest closure.

## Required edits (if any)

**One small edit. Single iteration; no re-draft.**

1. **§6.8 — clarify orchestrator role in co-decision routing.** §6.8 says architect + tech-lead co-decision required (not solo orchestrator). The sequence ("orchestrator surfaces signal → architect frames rollback in CP3 amendment → tech-lead ratifies") is correct, but the disagreement path is implicit. Add one sentence at end of "Co-decision rule" paragraph:

> "If architect and tech-lead disagree on B'+ activation, orchestrator does NOT silently break the tie; orchestrator surfaces both positions to user as the tie-break authority. This honors the consensus-mode operating discipline (no silent override of co-decider authority)."

This pre-empts a future-cascade reviewer assuming orchestrator picks the activation outcome unilaterally when architect/tech-lead split. Strategy-level not Tech Spec-level — `feedback_active_dev_mode.md` discipline applied to contingency-decision routing too.

**Optional / discretionary** (architect decides, not blocking CP3 lock):

- §6.2 ADR-00NN+3 (TriggerAction cluster-mode hooks) optional/required threshold — Strategy locks "surface contract only" at §3.4. If Tech Spec §7 surfaces any hook with required-implementation (no default body), ADR-00NN+3 should be promoted to required. CP3 §6 hint to Tech Spec §7 author. Not a CP3 blocker.
- §6.7 sunset table — S-C4 / S-C5 row "absorbed into CP6 if path (a) or (b) selected" is correct, but if path (c) is selected and B'+ activates, the rows need re-target to "post-A'-failure follow-up cascade" per §6.8. This is implicit in §6.8 "B'+ activation re-opens CR1 + CR7 as known sunset items" but not cross-linked from §6.7 → §6.8. CP3 hint to spec-auditor or architect for cross-link. Not blocking.

Neither blocks CP3 ratification.

## Strategy freeze readiness

**Yes, freeze-ready after the §6.8 co-decision-routing nit lands.**

CP1 / CP2 / CP3 sub-decisions all ratified or RATIFY-WITH-NITS with required edits closing in single iterations:
- CP1 RATIFY-WITH-NITS — 3 edits closed (B'+ structural condition, silent-degradation safeguard, §3.4 TBD).
- CP2 RATIFY-WITH-NITS — 2 edits closed (spike fail-state path-c linkage, §3.4 trait_variant OUT row).
- CP3 RATIFY-WITH-NITS — 1 edit (§6.8 co-decision-routing clarification).

§6 discharges all 8 forward-promises mapped from CP1/CP2 open items + tech-lead CP2 hints (§5 line 474 inventory). The §4.3.2 retry-scheduler path closure goes through §6.3 CP3 row + open items line 466 — Tech Spec authority preserved, Strategy doesn't punt. Memory entries (`decision_controlaction_seal.md`, `decision_terminate_gating.md`, `decision_action_phase_sequencing.md`, `decision_action_redesign_phase2_pick.md`) all preserved verbatim or honestly extended.

**No load-bearing forward-refs dangling.** Spike → Tech Spec sequencing locked at §6.1; ADR drafting roadmap locked at §6.2; Tech Spec CP cadence locked at §6.3; concerns-register lifecycle conditional at §6.4; path criteria at §6.5; cross-crate slot commitment at §6.6; sunset commitments at §6.7; B'+ contingency at §6.8.

After the §6.8 edit, Strategy lock at CP3 freeze is honest and complete. Phase 4 (spike) can begin once spec-auditor structural audit closes.

## Summary for orchestrator

**Verdict: RATIFY-WITH-NITS.** One small edit before CP3 final lock — not a re-draft. §6.2 ADR roadmap (3 required + 1 optional, drafting order grounded in frozen priors); §6.5 a/b/c decision tree (honest preconditions, no false confidence); §6.6 slot commitment three-field bar (named owner + absolute scheduled date + queue position) preserves my silent-degradation guard verbatim; §6.8 B'+ contingency (binary OR signals, rollback path with CR1/CR7 re-opening, architect + tech-lead co-decision rule) all RATIFY. §4.3.2 retry-scheduler forward-ref closes via §6.3 CP3 row + open items line 466 — Strategy locks principle (Retry+Terminate symmetric gating), Tech Spec CP3 picks path. My `decision_terminate_gating.md` solo call is preserved.

**Required edit:** §6.8 — add one sentence clarifying that orchestrator does NOT silently break architect/tech-lead disagreement on B'+ activation; orchestrator surfaces both positions to user as tie-break. Pre-empts a future-cascade reviewer assuming unilateral orchestrator authority when co-deciders split. Strategy-level discipline; small.

**Strategy freeze readiness:** YES, freeze-ready after §6.8 nit lands. All 8 §5-line-474 forward-promises discharged; CP1/CP2/CP3 sub-decisions all ratified; no dangling load-bearing forward-refs. Phase 4 (spike) can begin once spec-auditor structural audit closes.

**Iterate: yes (single pass, one targeted edit, no re-draft).**
