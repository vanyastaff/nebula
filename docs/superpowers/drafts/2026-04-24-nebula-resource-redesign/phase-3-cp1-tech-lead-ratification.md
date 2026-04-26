# Strategy CP1 — Tech-Lead Ratification

**Date:** 2026-04-24
**Reviewer:** tech-lead (subagent dispatch)
**Document:** `docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md`
**Checkpoint:** CP1 (§0-§3)

---

## Ratification verdict

**RATIFY_WITH_EDITS** — advance to CP2 with the small edits in the "Required edits" section. The Strategy faithfully captures the LOCKED Phase 2 decision and the merged amendments. No structural misalignment, no Phase 2 re-litigation, no missing-from-evidence claims. Edits are clarification + one explicit encoding gap (Amendment 1 location).

Confidence: high. CP1 is a credible historical record and frames CP2-CP3 + Phase 4 spike + Phase 6 Tech Spec to land on the right shape.

---

## §3 Phase 2 faithfulness

### §3.1 Option A — BLOCKED

**Match: faithful.**

- Strategy §3.1 (line 199-203) cites security-lead BLOCK on 🔴-1 silent revocation drop being "unfixable by deferral" — matches `phase-2-security-lead-review.md:25-46` ("A is unfixable without becoming B") and `02-pain-enumeration.md:229-231` ("atomic landing, standalone PR not viable").
- Tech-lead independent dismissal cited correctly (`feedback_incomplete_work.md` — writing docs against superseded `Auth`-shaped trait), aligns with `phase-2-tech-lead-review.md:22-24`.
- Specific evidence (`crates/resource/src/manager.rs:1360-1401`) is correctly traced through both reviewer paths.

### §3.2 Option B — chosen

**Match: faithful, with one editorial nit on convergence framing.**

- Strategy §3.2 (line 205-209) names tech-lead's 2 amendments correctly: "lock §3.6 shape, no sub-trait fallback during spike; make rotation observability explicit DoD" — exact match to `phase-2-tech-lead-review.md:80-94`.
- Names security-lead's 3 amendments correctly: "isolation invariant on concurrent dispatch, revocation extension of §3.6, warmup footgun resolution" — exact match to `phase-2-security-lead-review.md:60-82`.
- Acknowledges "Locked in round 1 of the max-3-rounds co-decision protocol" — matches `03-scope-decision.md:6` ("converged in round 1").
- "The atomic landing, §3.6 blue-green pattern, and observability-as-DoD were already baked into B.2 🔴-1 treatment — security-lead's amendments tightened rather than redirected" — accurate; matches `phase-2-security-lead-review.md:51-56`.

**Editorial nit (see ambiguity #3 below):** "convergent endorsement" + "unanimously picked Option B in round 1" (line 197) is technically correct (3 reviewers picked B in round 1) but the "unanimous" framing elides that the choice came with 5 merged amendments (2 tech-lead + 3 security-lead). My preference: keep "unanimous" since that's the protocol-level fact, but anchor it explicitly to "Option B as scope" rather than "Option B as drafted". See ambiguity answer below.

### §3.3 Option C — rejected

**Match: faithful and correctly framed.**

- Strategy §3.3 (line 215-219) explicitly cites "evidence the Phase 1 evidence did not force" — not generic scope-too-big.
- Three specific items called out:
  - **Runtime/Lease collapse** — "orthogonal to credential driver" (matches `phase-2-tech-lead-review.md:14`: "Runtime/Lease friction is real… but orthogonal to the credential seam").
  - **`AcquireOptions::intent/.tags`** — "blocked on engine-side design (#391)" + security-lead C.8a remove preference (matches `phase-2-security-lead-review.md:96`).
  - **Service/Transport merge** — "defensible but thin" + `feedback_boundary_erosion.md` rationale (matches `phase-2-tech-lead-review.md:69-70`).
- Schedule risk framed as secondary, not primary (line 221) — correct ordering. The primary rejection reason is "evidence didn't force," not "no time."
- "Not now, not no" framing (line 221) preserves the deferred-to-future-cascade option — matches `03-scope-decision.md:55` (Runtime/Lease "future cascade") and §2 out-of-scope budget.

---

## §1 problem framing for downstream phases

All 6 subsections frame the problem at the right level for downstream consumption. Specific verifications below.

### §1.1 Credential rotation surface

**Frames Phase 6 Tech Spec §3 correctly without pre-committing implementation.**

- Strategy §1.1 (line 64-76) sets up "Resource trait with `type Credential: Credential`" by citing credential Tech Spec §3.6 (lines 928-996) as the structural target — the future-shape is named in *constraint-source* terms (line 73), not as a pre-decided trait signature. Phase 6 Tech Spec §3 will compile-able-Rust this; CP1 names the shape but doesn't lock it at the type-level. **Correct level.**
- The "advertised-but-unverifiable" framing (line 76) — "100% `()` usage is a canon §4.5 false-capability signal at the trait level" — sets up Phase 6 to apply the canon §4.5 lens to every assoc-type / public field decision. Good.

### §1.2 Daemon/EventSource orphan

**Frames Strategy §4 (CP2) decision between engine/scheduler fold vs sibling crate.**

- §1.2 (line 80-90) lists evidence (canon §3.5, INTEGRATION_MODEL.md:89-91) but does **not** pre-commit to a landing site. Line 90 frames the consequence ("anything useful about Daemon/EventSource must be rebuilt either inside `nebula-resource` or outside it") as the decision Strategy §4 must make.
- This matches `03-scope-decision.md §4.6` (line 126-131) which explicitly defers the (a) engine/scheduler fold vs (b) sibling crate decision to "Strategy §4 decides." **Correct level.**

### §1.3 Docs rot

**Frames docs rewrite as Phase 6 deliverable, not Phase 3.**

- §1.3 (line 104) explicitly: "Docs must be rewritten atomically with the trait reshape, not standalone." Cites `feedback_incomplete_work.md`. Matches my Phase 2 priority-call (`phase-2-tech-lead-review.md:24`: "writing the doc rewrite against an `Auth`-shaped trait known to be superseded by credential Tech Spec §3.6 is 'don't write the docs twice' at the architecture level").
- Aligns with `03-scope-decision.md:35` ("Rewrite docs AFTER trait shape locks. Docs are a Phase 6 deliverable, not Phase 3"). **Correct deferral.**

### §1.4 Manager orchestration

**Preserves "split the file, NOT the type" position.**

- §1.4 (line 117): "Splitting the file (but keeping the `Manager` type) aligns the surface with the internal reality: one coordinator, many submodules of helpers." Direct match to `02-pain-enumeration.md:82` (tech-lead Phase 1 preview) and `03-scope-decision.md:39` (🟠-7 treatment: "Split the file, NOT the type").
- §1.4 evidence trace is solid: 2101-line measurement, asymmetric topology ergonomics (5/7 conveniences), `_with` builder anti-pattern, `Auth = ()` bound on conveniences. **Faithful.**

### §1.5 Drain-abort

**Keeps SF-2 absorbed into Option B, not reverted to standalone.**

- §1.5 (line 119-129) treats drain-abort as an in-cascade pillar (one of the 6 convergent drivers). No "standalone PR candidate" framing.
- Matches `03-scope-decision.md:36` (🔴-4 treatment: "Absorbed into Option B atomically (tech-lead did not amend to 'defer to standalone PR')") and `03-scope-decision.md:80` ("SF-2 … now absorbed into Option B").
- The §12.6 observability-honesty framing (line 129) — "the phase is the advertised capability for 'is the resource healthy?' and it lies on the Abort path" — sets up the Phase 6 Tech Spec to treat phase consistency as a hot-path invariant. Good. **Faithful.**

### §1.6 Observability

**Sets up the Phase 6 CP-review gate I amended in.**

- §1.6 (line 141): "`feedback_observability_as_completion.md` applies: observability is Definition of Done for a hot path, not a follow-up." Direct match to my Phase 2 Amendment 2.
- Tied through to §2.4 line 184 ("trace span + counter + event = DoD for the rotation path. Phase 6 Tech Spec CP review has an explicit observability gate").
- This double-anchoring (problem statement + constraint) is exactly the framing I want for the Phase 6 CP gate. **Faithful.**

---

## §2 constraints encoding of Phase 2 amendments

### Amendment 1 — Lock §3.6 shape, sub-trait fallback REMOVED from spike exit criteria

**Encoded — but only in §3.2 (line 209), not in §2.**

- §3.2 (line 209) names: "tech-lead priority-called Option B with two bounded amendments (lock §3.6 shape, no sub-trait fallback during spike; …)".
- §2.4 (line 184) does not separately encode the spike-exit-criteria constraint.
- §2.3 (cross-crate contracts) cites credential Tech Spec §3.6 verbatim (line 165-169) — that's the *target* the shape is locked to, but not the *constraint that the spike doesn't have an escape valve*.

**Status:** Amendment 1 is captured in the historical-record §3.2 sentence. It is **not** captured as a forward-binding §2 constraint that Phase 4 spike will grep for.

**Required edit (E1, see below):** add a one-line constraint in §2.4 (or §2.3) explicitly: "Phase 4 spike exit criteria does NOT include sub-trait fallback. §3.6 shape failure escalates to Phase 2 round 2 per `phase-2-tech-lead-review.md` Amendment 1, not a mid-flight shape change."

### Amendment 2 — Observability as CP-review gate

**Encoded correctly in §2.4 line 184.**

- "`feedback_observability_as_completion.md` — trace span + counter + event = DoD for the rotation path. Phase 6 Tech Spec CP review has an explicit observability gate (`03-scope-decision.md §4.4`)."
- This is exactly the gate I asked for. **Faithful.**

---

## Answers to architect's 3 self-flagged ambiguities

### Ambiguity 1 — §2.3 revocation extension framing (problem-only vs decision-requested)

**My call: problem-framing is correct for CP1; CP2 §4 makes the decision. Add a one-word clarifier.**

- §2.3 (line 170) currently reads: "Strategy must extend §3.6 with revoke semantics. Candidate approaches (CP2 to pick): (a) `on_credential_refresh` carries both semantics (resource decides how to tear down when the scheme is revoked); (b) separate `on_credential_revoke` method."
- Framing the *candidates* in CP1 and the *decision* in CP2 is correct — CP1 is constraints + history, CP2 is decisions. The candidates need to be on the table by CP1 so spec-auditor can verify the credential-side spec extension boundary.

**Required edit (E2, see below):** change "Strategy must extend §3.6 with revoke semantics" to "**CP2 §4 must extend §3.6 with revoke semantics**" — one word, locks the decision-checkpoint boundary explicit. Currently reads as if the extension is the Strategy's CP1 problem (it's not).

### Ambiguity 2 — §2.1 ADR-0035 disclaimer positioning

**My call: positioning is correct. No new ADR amendment needed in Phase 5. Open-item §2.1 (line 232) framing is right.**

- §2.1 (line 149): "*For reference, not binding on this redesign.*" — correct. ADR-0035 is credential-side phantom-shim; resource-side `TopologyTag` is concrete enum (`01-current-state.md:88`'s "Brief was wrong. Corrected." note). Citing it here prevents Phase 4 spike or Phase 6 Tech Spec from accidentally invoking phantom-shim framing for topology dispatch — that's a real risk, the disclaimer pre-empts it.
- The open-item (line 232) "no ADR currently exists recording the TopologyTag-is-not-phantom-tag correction from Phase 0. Worth a one-line ADR amendment to ADR-0035 noting the scope boundary, or a Phase 5 ADR clarifying. CP2 to decide" — correctly defers the meta-question to CP2.

**My ratification position:** No new ADR is needed. ADR-0035 is about the credential phantom-shim pattern; resource crate doesn't use it. The "TopologyTag is not phantom-tag" correction is a Phase 0 brief-correction, not a design decision worth its own ADR. CP2 should resolve the open-item by **closing it** rather than creating ADR work. CP3 §6 can cite the open-item-resolution if archival proof is wanted.

**No required edit on E-this** (it's an open-item and architect's framing is fine; just stating my position so CP2 doesn't accidentally over-architect).

### Ambiguity 3 — §3.2 convergence framing (unanimous vs amendment-count)

**My call: keep "unanimous". Do NOT change to "convergent with 5 amendments" or similar.**

**Reasoning (one sentence as requested for return summary):** "Unanimous in round 1" describes the **protocol-level fact** (all 3 reviewers picked B in round 1 of max-3); the 5 merged amendments tightened scope but did not split the body, so changing to "convergent with N amendments" would conflate protocol convergence with content convergence and weaken the historical record.

**Optional editorial tightening (E3, see below):** add one parenthetical to §3.2 line 197 to disambiguate: "co-decision body (architect + tech-lead + security-lead) **unanimously picked Option B in round 1** of the max-3 protocol, with 2 tech-lead amendments and 3 security-lead amendments tightening the in-scope envelope (per the per-review pointers in this section)." This preserves "unanimous" as the headline fact and makes the amendment count visible without conflation. Architect's call whether to apply.

---

## Any missing constraints I want added

### MC-1: cascade invariants from Phase 2 not in §2

**One memory cross-ref I expected to see and didn't.**

- `feedback_active_dev_mode.md` is in `MEMORY.md` and applies here ("Active dev ≠ prod release. Never settle for 'green tests / cosmetic / quick win / deferred'"). The Strategy invokes `feedback_no_shims.md`, `feedback_hard_breaking_changes.md`, `feedback_observability_as_completion.md`, `feedback_boundary_erosion.md` — all correct. But `feedback_active_dev_mode.md` is the umbrella feedback that justifies why we're doing the trait reshape *now* rather than deferring to a hypothetical "v0.2 stability boundary" — and §2 doesn't cite it.

**Required edit (E4, see below):** add to §2.4 a one-line reference: "`feedback_active_dev_mode.md` — `frontier` maturity + active dev posture means breaking changes ship now, not 'after stability'. Aligns with §2.5 maturity invariant."

### MC-2: nothing else missing

I went through `phase-2-tech-lead-review.md` line-by-line for amendments / scope / coordination items and the LOCKED scope decision §4 (`03-scope-decision.md` §4.1-§4.7) for ratified design decisions. Everything load-bearing is encoded somewhere in §1, §2, §3, or referenced via the open-items list. The 4.6 (Daemon/EventSource extraction target) deferral to Strategy §4 / CP2 is explicitly acknowledged. The 4.5 warmup semantics deferral to Tech Spec §5 / Phase 6 is explicitly acknowledged. The 4.3 revoke semantics deferral covered above (Ambiguity 1). The 4.7 migration discipline (no shims, atomic 5-consumer migration) is in §2.4 (line 182) and §2.5 (line 193). **Comprehensive.**

---

## Required edits (RATIFY_WITH_EDITS)

| ID | Edit | Location | Priority |
|----|------|----------|----------|
| **E1** | Add one-line spike-exit-criteria constraint to §2.4: "Phase 4 spike exit criteria does NOT include sub-trait fallback. §3.6 shape failure escalates to Phase 2 round 2 per `phase-2-tech-lead-review.md` Amendment 1, not a mid-flight shape change." | §2.4 (insert after current `feedback_observability_as_completion.md` line) | **HIGH** — Amendment 1 currently only in §3.2 historical narrative; needs forward-binding constraint or Phase 4 spike could miss it |
| **E2** | Change "Strategy must extend §3.6 with revoke semantics" to "**CP2 §4 must extend §3.6 with revoke semantics**" | §2.3 line 170 | **MEDIUM** — locks the decision-checkpoint boundary; one-word clarifier |
| **E3** | Add parenthetical to §3.2 line 197: "…unanimously picked Option B in round 1 of the max-3 protocol, with 2 tech-lead amendments and 3 security-lead amendments tightening the in-scope envelope (per the per-review pointers in this section)." | §3.2 line 197 | **LOW** — editorial only; my position on Ambiguity #3 is "keep 'unanimous'" so this is optional polish |
| **E4** | Add `feedback_active_dev_mode.md` reference to §2.4 | §2.4 (after `feedback_boundary_erosion.md` line) | **MEDIUM** — closes a missing memory cross-ref that justifies the breaking-change posture |

E1 is the only edit that has substantive forward-binding effect. E2/E3/E4 are clarification + completeness.

---

## Convergence on CP1 (lock / need iteration)

**Lock without iteration after applying E1, E2, E4.** E3 is editorial; architect's call.

Reasoning:
- Verdict is RATIFY_WITH_EDITS, not ITERATE — no structural misalignment, no Phase 2 re-litigation.
- E1 is one-line addition (Amendment 1 forward-binding). E2 is one-word change. E4 is one-line addition (memory cross-ref). All three together are ~5 lines of edit; architect can apply directly without a new co-decision cycle.
- No questions for security-lead. No new constraints surfaced from re-reading Phase 2. The Strategy §0-§3 reads as a credible design record.
- spec-auditor doing parallel structural-consistency check; if they surface something, that's a separate iteration trigger, not me.

**CP1 should advance to CP2 once E1 + E2 + E4 land.** Estimated architect effort: 5-10 minutes of editing. No new round needed.
