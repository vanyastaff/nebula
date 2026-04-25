---
reviewer: tech-lead
mode: solo decider (§1 attribution + §3 ranking ratification + §2 constraints adequacy)
date: 2026-04-24
target: docs/superpowers/specs/2026-04-24-action-redesign-strategy.md (DRAFT CP1, §0-§3)
parallel: spec-auditor (structural audit)
---

## Review verdict

**RATIFY-WITH-NITS.** Two §3 nits are load-bearing for B'+ contingency correctness; one §3.4 row violates `feedback_active_dev_mode` by its own letter. §1 attribution and §2 constraints adequacy are clean. Architect must iterate once before CP1 lock; the iteration is targeted (3 spots), not a re-draft.

## §1 attribution check

Clean.

- **(a) Credential paradigm mismatch (CR1, CR5–CR10).** Phase 0 / Phase 1 attribution faithful. The "both crates are out-of-sync with a still-unimplemented spec" framing matches my round 2 grep + `project_cp6_vocab_unimplemented.md`. Re-verified today: `crates/credential/src/` still has zero `CredentialRef|SchemeGuard|SlotBinding` matches. Citation accurate.
- **(b) Macro emission bugs (CR2, CR8, CR9, CR11).** Three-agent convergence (dx-tester / security-lead / rust-senior) on the `parameters = Type` bug is correctly attributed; root-cause framing ("structural: no `[dev-dependencies]`") is honest, not overselling — proc-macros without trybuild is a known structural defect class, not "the team forgot."
- **(c) Canon §3.5 governance drift.** Distinction between **documentation drift** (10 re-exports) and **structural drift** (engine dispatch is still 4-variant `ActionHandler`) is technically correct and is the right framing. Honors my `decision_controlaction_seal` (seal as DX helper, do not extend canon §3.5 dispatch primaries). Not overselling.
- **(d) S-C2 / S-J1 attribution.** S-C2 "exploitable today, not hypothetical" is accurate — `type_name::<S>().rsplit("::").next()` is the actual line. S-J1 v2-spec amendment B3 attribution is correctly cited. No drift from Phase 0 / Phase 1 sources.

**Plugin-author experience paragraph.** The 12 / 8 / 32 minute figures are dx-tester worktree numbers; §1 cites them as canonical. §183 (open items) flags this for re-confirm — acceptable for CP1.

**Cross-crate impact.** "Symmetric deadlock" framing matches my Phase 1 finding and round 2 reaffirmation. Good.

## §2 constraints adequacy check

Complete enough for ratify. Two minor additions worth considering for CP2 (not blocking CP1):

- **§2.1 / §2.2 / §2.3 / §2.4 / §2.5 / §2.6 / §2.7** — canon citations all resolve to the right line ranges. §2.3 specifically calls out `Terminate` as a §4.5 violation today and binds A' to the `Retry` discipline (`unstable-retry-scheduler` pattern); that matches my `decision_terminate_gating` (gate AND wire, not gate-only).
- **§2.8 credential Tech Spec shapes.** Verbatim adoption of §2.7 / §3.4 / §7.1 / §15.7. Line ranges in §15.7 (3383-3517 including spike iter-3 lifetime-gap refinement at 3503-3516) match the credential Tech Spec citations. §7.1 line range is flagged in §183 open items as "needs precise line-number citation" — acceptable for CP1.
- **§2.9 ADR-0035 phantom-shim.** Line ranges 65-108 and 124-135 (Pattern 4, lifecycle sub-trait erasure) cited correctly. Composition obligation to `mod sealed_caps` is the right boundary call.
- **§2.10 layer invariants.** T9 (`deny.toml` rule for `nebula-action`) is correctly named. Boundary erosion language matches `feedback_boundary_erosion`.
- **§2.11 feedback memory.** Five memories cited; rationale per memory is honest, not boilerplate. `feedback_no_shims` framing of CR3 fix as "hard removal, not `#[deprecated]`" is right.
- **§2.12 must-have floor.** Four items, all from security-lead 03c. Non-negotiable framing matches the floor's role.

**Considered for §2 additions, not flagged as missing:**

- `feedback_idiom_currency.md` (1.95+ Rust idioms — `async-trait` / `Box<dyn Error>` / `Arc<Mutex>` defaults are anti-patterns now). Relevant for the `*Handler` HRTB modernization in §3.1 component 5; could be cited in §2.11 as load-bearing for that component. Optional CP1 add; mandatory if CP2 §4 picks up the HRTB modernization as in-scope.
- `feedback_observability_as_completion.md` (typed error + trace span + invariant check are DoD). Relevant for §3.1 component 4 security hardening (sanitization paths must ship with trace spans, not as follow-up). Optional CP1 add.

Neither is load-bearing enough to gate CP1 ratification. Architect can decide whether to roll into the CP1 iteration or carry to CP2.

## §3 ranking alignment with Phase 2

§3.1 (A' chosen), §3.2 (B'+ second / contingency), §3.3 (B' / C' rejected) match my round 2 ranking **A' 1st / B'+ 2nd / C' 3rd / B' 4th** correctly. Two §3.2 omissions are load-bearing — see Required Changes below.

**§3.1 alignment.** Components 1-8 mirror `03-scope-decision.md` §1.1-§1.8 without silent additions or drops. Component 5 (Phase 1 tech-lead solo-decided calls) correctly ratifies all three of my decisions: ControlAction seal, Terminate gate-AND-wire, *Handler HRTB modernization. Component 7 (cluster-mode hooks) correctly delineates "surface contract only" — engine cluster coordination is in §3.4 OUT row.

**§3.2 alignment with my round 2 framing — TWO LOAD-BEARING OMISSIONS.**

My round 2 promoted B'+ over C' specifically *because* B'+ escapes boundary erosion — but only under two conditions:

1. **Structural condition** — `CredentialRef<C>` / `SlotBinding` / `SchemeGuard<'a, C>` MUST land in `nebula-credential`'s public surface, NOT in `nebula-action::credential::*`. If they land in action with delegation up to credential, that IS the boundary-erosion variant (credential vocabulary in wrong crate). Architect's `03a` line 133 acknowledges both placement variants and states the "leaning preference is land in credential to avoid future re-home." Strategy §3.2 does not carry this forward — it describes B'+ at the abstract level and never names the placement constraint. **For B'+ contingency activation correctness, §3.2 must lock the placement decision to nebula-credential, not leave it open.**

2. **Mandatory addition** — User must commit a credential CP6 implementation cascade slot before Phase 2 closes. Without that commitment, B'+ degrades to B' silently (the forbidden quick-win-trap per `feedback_active_dev_mode`). Strategy §3.2 says only "B'+ stays available as a contingency... if §4 recommendation frames A' implementation path... and the user picks one that exposes scheduling friction." That's the right contingency *trigger* but doesn't cite the *precondition* — which is that any B'+ activation must be paired with a real (named, owned, scheduled) credential CP6 implementation cascade slot, not a TODO.

These two omissions are not §3.2 *misframings* — the rest of §3.2 is honest. They are *missing constraints* that decide whether B'+ is selectable when the contingency triggers. If §4 (CP2) frames the A' implementation choice and the user pivots to B'+, the orchestrator and architect need these constraints already locked in §3.2 to avoid re-running this analysis.

**§3.3 B' rejection rationale alignment.** Honest acknowledgment, not dismissive. The five sub-bullets (permanent engine bridge, superseded credential idiom, double migration cost, no canon revision, security-lead must-have at "deprecate not remove" form) all match my round 1 + round 2 rationale. The explicit ranking quote ("A' 1st / B'+ 2nd / C' 3rd / B' 4th") is cited verbatim. Good.

**§3.3 C' rejection rationale alignment.** Honest. The "spec-reality gap is implementation lag, not spec defect" framing matches my round 2 reasoning. Spike iter-1/2/3 commit hashes and "all 5 spike scope questions cleared with no fallback triggered" claim is accurate. C' as "serious option that lost on bar" framing honors the seriousness; not strawmanned. The escalation rule 10 framing (C' triggers it, A' sidesteps) matches my round 2 escalation table.

## §3.4 OUT-of-scope tracking check

Most rows resolve to real homes. **One row violates `feedback_active_dev_mode` by its own letter** — flag for fix.

Real homes (acceptable):

- DataTag / `Provide` port → port-system sub-cascade (named).
- `Resource::on_credential_refresh` → resource cascade or co-landed with credential CP6 implementation (named, conditional).
- Post-cascade implementation of CP6 vocabulary → user decision (a)/(b)/(c) per `03-scope-decision.md` §6 (named, references the decision document).
- T3 `nebula-runtime` reference → separate PR (acceptable; small).
- S-W2 → webhook hardening cascade, 2 release cycles (named, sunset-committed in §2.12).
- S-C4 → credential CP6 landing (named, conditional).
- S-O1/S-O2/S-O3 → output-pipeline hardening cascade (named).
- S-I2 → sandbox phase-1 cascade (named).
- S-W1/S-W3/S-F1/S-I1/S-U1/S-C1 → cascade exit notes per security-lead 03c sunset list (named, accept).
- Signed manifest infrastructure → credential Strategy §6.5 queue #7 (named).

**Flagged row (TBD violation):**

- **"Engine cluster-mode coordination implementation"** → `Engine cascade (TBD — tech-lead to schedule)`. The "TBD — tech-lead to schedule" is a literal `feedback_active_dev_mode` violation: "before saying 'defer X', confirm the follow-up has a home." TBD is not a home. Two acceptable resolutions:
  - Replace TBD with a concrete cascade name + my commitment to schedule it (e.g., "Engine cluster-mode coordination cascade — tech-lead will schedule post-action-cascade, target queue position behind credential CP6 implementation cascade").
  - If I cannot commit to scheduling at CP1 time, escalate to user as an open item in §3.4-pre-CP2 rather than ship as TBD.

I commit to the first resolution. Architect can replace TBD with the explicit owner + queue-position commitment.

## Required changes (if any) for CP1 final

Three targeted edits. Single iteration; no re-draft.

1. **§3.2 — add structural condition for B'+ contingency activation.** Insert a "Conditions for B'+ activation" sub-paragraph naming both: (a) `CredentialRef<C>` / `SlotBinding` / `SchemeGuard<'a, C>` placement in `nebula-credential` (NOT `nebula-action::credential::*`); (b) committed credential CP6 implementation cascade slot before any B'+ activation. Cite my round 2 §"Does B'+ resolve your boundary-erosion concern?" framing in `03b-tech-lead-priority-call.md`.

2. **§3.2 — add the silent-degradation safeguard.** One sentence: "Without a committed credential CP6 implementation cascade slot, B'+ degrades to B' silently — `feedback_active_dev_mode` violation. CP2 §4 / CP3 §6 must verify the slot is committed before ratifying any B'+ contingency activation." This is the operational form of the mandatory addition.

3. **§3.4 row 3 — replace TBD with concrete commitment.** Change the "Reason" column for "Engine cluster-mode coordination" from "Engine cascade (TBD — tech-lead to schedule)" to a concrete commitment: "Engine cluster-mode coordination cascade — tech-lead schedules post-action-cascade close, queued behind credential CP6 implementation cascade." If architect cannot make this commitment on my behalf, escalate to me directly via handoff and I will commit at CP1 lock.

**Optional / discretionary** (architect decides):

- §2.11 — consider citing `feedback_idiom_currency.md` as load-bearing for §3.1 component 5 (`*Handler` HRTB modernization). If CP2 §4 keeps this component "optional" per §186 open item, the citation is mandatory. If §4 promotes it to in-scope, the citation is highly recommended.
- §2.11 — consider citing `feedback_observability_as_completion.md` as load-bearing for §3.1 component 4 (security hardening must ship with trace spans, not as follow-up).

Neither is gating CP1 ratification.

## Summary for orchestrator

**Verdict: RATIFY-WITH-NITS.** Architect iterates once on three targeted edits before CP1 lock — not a re-draft. §1 attribution clean. §2 constraints complete. §3 ranking alignment with my round 2 (A'/B'+/C'/B') is faithful in framing but omits two load-bearing constraints from B'+ contingency activation — placement of `CredentialRef<C>` etc. in nebula-credential, and committed credential CP6 implementation cascade slot. §3.4 has one row violating `feedback_active_dev_mode` by its own letter (TBD owner) — small fix.

**§4-§6 hints to pre-load for CP2:**

- §4 (recommendation): must explicitly choose between (a) single coordinated A' implementation PR, (b) sibling cascades A' (credential leaf-first / action consumer-second in lockstep), (c) phased rollout with B'+ surface commitment as intermediate. My round 2 preference is (b) if credential-crate owner has bandwidth, else (a). Tie-break: ESCALATION.md surfaces the choice to user.
- §4 must lock the §186 open item: `*Handler` HRTB modernization is in-scope OR scoped-out, not "optional but recommended."
- §4 must lock the §186 open item: `unstable-retry-scheduler` retire OR convert-to-gated-with-wiring (depends on `Terminate` scheduler design).
- §5 (open items + spike plan, CP2): must include spike for `SlotBinding::resolve_fn` HRTB compilation against current credential internals. This is a dependency-discharge spike before Tech Spec writing begins (Phase 6).
- §6 (post-validation roadmap, CP3): must record the B'+ contingency activation criteria explicitly — what signal triggers B'+ over A', what the rollback path looks like, what the B'+ → A' upgrade path looks like.
