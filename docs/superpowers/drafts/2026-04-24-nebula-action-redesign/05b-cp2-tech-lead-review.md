---
reviewer: tech-lead
mode: solo decider (§4 sub-decisions ratification + §5 spike plan adequacy)
date: 2026-04-24
target: docs/superpowers/specs/2026-04-24-action-redesign-strategy.md (DRAFT CP2, §4-§5)
parallel: spec-auditor (structural audit)
---

## Review verdict (RATIFY / RATIFY-WITH-NITS / REVISE)

**RATIFY-WITH-NITS.** Two targeted edits required for CP2 final lock; no re-draft. §4.1 locked decision + §4.4 security floor verbatim adequate; §4.3.1 / §4.3.2 / §4.3.3 sub-decisions match my Phase 1 solo calls and CP1 review hints faithfully. §4.2 implementation path framing (a/b/c) is honest and structured for user pick at Phase 8 without pre-picking. §5 spike plan iter-1/iter-2 scope discharges the right HRTB + cancellation-drop dependencies before Tech Spec §7 writing. Two nits below are operational discipline, not framing defects.

## §4 sub-decisions ratification

### §4.1 — locked decision: A'

**RATIFY.** Verbatim alignment with `03-scope-decision.md` §0 ("Chosen option: A' — Co-landed cascade") and §1 ("Tech Spec covers design for [eight components]"). Cascade-scope-as-DESIGN-closure-only reframe is preserved. Reference to "post-cascade implementation is user-gated" is the right framing — Strategy doesn't pre-empt the user's path-choice authority. The "eight A' components (§3.1) are unchanged and inherit from the scope decision" line is the right delegation — §4 doesn't re-derive, it locks load-bearing sub-decisions inside the components. Good.

### §4.2 — implementation path framing (a/b/c)

**RATIFY.** The three-path table is honest and structured for user pick at Phase 8:

- **Path (a)** — single coordinated PR with the 18-22d / 8-12d gap explained as codemod+plugin migration counted vs not. Trade-off (single cut, high coordination cost, large reviewer load) is accurate.
- **Path (b)** — sibling cascades (credential leaf-first + action consumer-second in lockstep). Matches my round 2 preference per `03b-tech-lead-priority-call.md`. Trade-off (sequencing friction during the gap, owner bandwidth dependency) is accurate.
- **Path (c)** — phased rollout with B'+ surface commitment. Activation precondition (line 205) cites both §3.2 conditions verbatim: (1) `CredentialRef<C>` / `SlotBinding` / `SchemeGuard<'a, C>` placement in `nebula-credential` (NOT action), (2) committed credential CP6 implementation cascade slot before activation. Silent-degradation guard preserved. Good — this is the §3.2 nit from CP1 review carried forward correctly.

"Strategy does not pre-pick (a)/(b)/(c); orchestrator surfaces the choice to user when cascade summary lands (Phase 8)" is the correct framing per scope decision §6. No pre-emption of user authority. Good.

**Minor framing observation (not blocking):** path (a) cites the 18-22d / 8-12d gap; path (b) doesn't enumerate per-cascade agent-day estimates ("Each fits normal autonomous budget"). This is fine for Strategy-level framing; CP3 §6 / Tech Spec §9 should re-baseline path (b) per-cascade budgets when the credential CP6 implementation cascade slot lands. Not a CP2 blocker.

### §4.3.1 — `*Handler` HRTB modernization IN this cascade

**RATIFY.** Confirms my Phase 1 02c §6 position (rust-senior 02c §6 line 386 LOC payoff finding) and my CP1 review pre-load hint (line 113-114: "§4 must lock the §186 open item: `*Handler` HRTB modernization is in-scope OR scoped-out, not 'optional but recommended.'"). Verdict: in-scope, not optional, not deferred. Reasoning chain is accurate:

- ~30-40% LOC reduction across `stateless.rs:313-322`, `stateful.rs:461-472`, `trigger.rs:328-381`, `resource.rs:83-106` plus mirrored adapter sites — single `'a` lifetime + `BoxFut<'a, T>` type alias replacing pre-1.85 HRTB shape. Matches 02c §6 finding verbatim.
- Mechanically low-risk + architecturally coherent with A' (the same files are being rewritten anyway for `CredentialRef<C>` / `SlotBinding`). Deferring to a separate hygiene PR forces a second pass — that's the "more-expedient over more-ideal" trap from `feedback_active_dev_mode`.
- Correct citation of `feedback_idiom_currency.md` per my CP1 review line 39-42 ("mandatory if CP2 §4 picks up HRTB modernization as in-scope"). §2.11 amendment-pending in CP2 CHANGELOG line 375 covers this.

**Out-of-scope framing.** `#[trait_variant::make(Handler: Send)]` adoption (rust-senior 02c §6 line 362-380) correctly scoped out as a separate Phase 3 redesign decision — would collapse the RPITIT/HRTB split into a single source generating both, breaking existing public `*Handler` trait surface. CP3 should add an explicit §3.4 OUT row for this (CHANGELOG line 371 says "to be added in CP3 if not pre-empted"). Acceptable.

### §4.3.2 — `Retry`+`Terminate` symmetric gating

**RATIFY.** Confirms my Phase 1 solo-decided call on `ActionResult::Terminate` per `decision_terminate_gating` (gate AND wire end-to-end, not gate-only) extended to `Retry` consistently. The two-path framing is correct:

1. **Wire both end-to-end** — scheduler infrastructure lands in cascade scope; both feature flags come down; canon §11.2 graduates "planned" → "shipped."
2. **Retire `unstable-retry-scheduler`** OR keep it as gated-with-wired-stub (return-typed-error-not-implemented at engine boundary). `feedback_active_dev_mode.md` violation if Retry wired and Terminate gate-only — that's the asymmetric gate-only-and-defer trap.

The "Strategy locks the principle, Tech Spec §9 picks the path" framing is the right division of authority — Strategy can't resolve scheduler design depth without the design pass. Good.

**Confirmation of my Phase 1 solo call:** `decision_terminate_gating.md` reads "feature-gate Terminate AND wire end-to-end in redesign cascade; don't gate-only (active-dev-mode)." §4.3.2 preserves this and extends symmetric discipline to Retry. CP3 §6 / Tech Spec §9 must record the chosen path explicitly when scheduler depth is decided.

### §4.3.3 — codemod scope (5 transforms)

**RATIFY.** Five mechanical transforms enumerated; sufficient minimal complete set for plugin migration:

1. `#[derive(Action)]` → `#[action]` with attribute-args migration. `parameters = Type` (broken in current macro per 02c §2 line 134-141) "drops or rewrites depending on Tech Spec §7 ActionContext API" — defers correctly to Tech Spec.
2. `ctx.credential_by_id(...)` / `ctx.credential_typed(...)` / `ctx.credential::<S>(key)` (3 variants) → `ctx.credential::<S>(key)` / `ctx.credential_opt::<S>(key)`. Pinned exact API location to open item §5.1.1 (was CP1 §1(a)). Correct.
3. `CredentialContextExt::credential<S>()` no-key heuristic (S-C2 / CR3) — **hard removal**, no `#[deprecated]` shim. Cites `feedback_no_shims.md` + security-lead 03c §1 VETO. Codemod must error on remaining sites with crisp diagnostic. Matches must-have floor §3 item 2 verbatim.
4. `crates/action/macros/Cargo.toml` `[dev-dependencies]` block — `trybuild`, `macrotest` per Phase 0 T1 finding. Codemod adds the block; new harness lands alongside macro rewrite. Correct — this discharges the structural defect class (proc-macros without trybuild).
5. `nebula-sdk::prelude` re-export reshuffle — 40+ re-exports per scope decision §1.6; codemod identifies removed names; reverse-dep migration guide lists added/removed/renamed pairs.

The "design only" scope split — codemod **execution** (running on 7 reverse-deps) is post-cascade per §3.4 OUT row; codemod **design** (script shape, transform list, dry-run output format) is Tech Spec §9 — is the right boundary. Tech Spec §9 may add transforms during design without re-opening Strategy. Good.

**Sufficient for plugin migration story?** Yes. The five transforms cover macro rewrite + ActionContext API surface + S-C2/CR3 hard removal + macros harness landing + sdk::prelude reshuffle. The 7 reverse-deps will exercise transforms 1-3 + 5 in real usage; transform 4 is internal. No silent gaps.

### §4.4 — security must-have floor verbatim

**RATIFY.** Four-item floor from `03-scope-decision.md` §3 cited verbatim with line ranges intact:

1. **CR4 / S-J1 JSON depth bomb** — depth cap (128) at adapter JSON boundaries. Mandated by v2 design spec post-conference amendment B3. Verbatim match.
2. **CR3 / S-C2 cross-plugin shadow attack** — replace type-name heuristic with explicit keyed dispatch at method-signature level. **Hard removal**, not `#[deprecated]` shim. Cites `feedback_no_shims.md` + security-lead 03c §1 VETO. Verbatim match — language is preserved character-for-character.
3. **`ActionError` Display sanitization** — `redacted_display()` helper in `tracing::error!` call sites to preempt S-C3 / S-O4 leak class. Helper hosting open item §5.1.2 properly cross-referenced. Verbatim match.
4. **Cancellation-zeroize test** — closes S-C5; pure test addition. Verbatim match.

**`feedback_observability_as_completion.md` integration (line 253):** correctly cites items 2-3 as needing trace spans + invariant checks shipped with the security hardening, not as follow-up. This was CP1 review optional discretionary item line 102; promoted to load-bearing in CP2 §4.4. Good — DoD framing is the right operational discipline. §2.11 amendment-pending in CHANGELOG line 375 covers the citation roll-up to CP3.

**Security-lead VETO check at implementation time** is preserved per §4.4 framing. No drift.

## §5 spike plan adequacy

### §5.1 tracked open items

**RATIFY.** All five carried-forward items have named owners + deadlines + resolution scopes:

- §5.1.1 ActionContext API location (was CP1 §1(a)) — owner architect + credential Tech Spec author; deadline before Tech Spec §7 drafting. Correct.
- §5.1.2 `redacted_display()` helper crate location (was CP1 §2.6) — owner architect + security-lead; deadline Tech Spec §4 (Security section). Correct — likely `nebula-log` or new `nebula-redact` per credential Strategy §6.5 queue.
- §5.1.3 Credential Tech Spec §7.1 precise line numbers (was CP1 §2.8) — owner architect; deadline CP3 draft. Correct.
- §5.1.4 B'+ contingency activation criteria detail — my CP1 hint line 116 carried forward; CP3 §6 must record signal triggers + rollback path + sunset commit. Correct.
- §5.1.5 Engine cluster-mode hooks final shape — Tech Spec §7 scope; Strategy locks "surface contract only" per §3.4. Correct.

### §5.2 spike plan

**RATIFY-WITH-NITS** (one minor; see Required changes #1).

**§5.2.1 spike target** — HRTB fn-pointer + `SchemeGuard<'a, C>` cancellation drop-order. Two questions discharge the right dependencies:

1. Does `SlotBinding::resolve_fn: for<'ctx> fn(...) -> BoxFuture<'ctx, _>` (per credential Tech Spec §3.4 line 869) compile against engine wiring + `#[action]` macro emission contract? This is exactly the dependency-discharge spike I flagged in CP1 review line 115.
2. Does `SchemeGuard<'a, C>` (`!Clone`, `ZeroizeOnDrop`, `Deref`, lifetime parameter per credential Tech Spec §15.7 line 3394-3429) honor zeroize-on-drop across cancellation boundary (drop guard mid-`.await` under `tokio::select!`)? This validates the security floor item 4 (cancellation-zeroize test) compositionally.

Both questions gate Tech Spec §7 Interface confidence. Right scope.

**§5.2.2 iter-1 — minimum compile shape.** Three compile-fail probes:

- Probe 1: `ResourceAction` without `Resource` binding fails to compile — validates ActionSlots impl emission. Right scope.
- Probe 2: `TriggerAction` without trigger source fails to compile — validates trait contract enforcement. Right scope.
- Probe 3: Credential field rewriting confined to attribute-tagged zones — bare `CredentialRef<C>` outside `credentials(slot: Type)` zone fails or warns. Validates §3.1 component 2 narrow declarative rewriting contract per my Phase 2 §2 architectural coherence constraint. Right scope.

Iter-1 DONE: all 3 probes compile-fail as expected; minimal skeleton compiles clean; `cargo check --workspace` clean. Concrete, observable, no hedging. Good.

**§5.2.3 iter-2 — composition + cancellation + perf sanity.** Three actions covering different variants:

- Action A: `StatelessAction` + `Bearer` (Pattern 1 pass-through).
- Action B: `StatefulAction` + `OAuth2` with refresh (Pattern 2 dispatch + `RefreshDispatcher::refresh_fn` HRTB).
- Action C: `ResourceAction` + `CredentialRef<C>` + `Postgres` (resource binding + credential composition + `Resource::on_credential_refresh` interaction surface).

Plus dispatch ergonomics check + cancellation-drop-order test + macro expansion perf sanity (within 2x of current `#[derive(Action)]` baseline). Right composition coverage.

**Iter-2 DONE criteria:** all 3 actions compile + dispatch-shape compiles clean + cancellation-drop test passes + expansion perf within 2x. Concrete + measurable + falsifiable. Good.

**Aggregate DONE (§5.2.4):** all probes pass + composition compiles + cancellation-drop test passes + expansion perf within 2x. Spike worktree isolated; scratch only; no commit to main — pattern follows credential Strategy §6.1 spike iter-1/2/3 worktree pattern. Correct.

**Budget framing (§5.2.4):** max 2 iterations per cascade prompt; failures may trigger Strategy revision (CP3 amendment) but are NOT cascade-blocking — spike failure narrows §4.2 path choice (e.g., HRTB compose failure broadens path (c) B'+ activation criteria) or surfaces Tech Spec §7 redesign requirement, not cascade-blocking escalation. Honest framing — failure-mode planning matches credential Strategy §6.1 spike pattern.

**Spike → Tech Spec interface lock plan (§5.2.5):** three artefacts (`NOTES.md`, `final_shape_v2.rs`, test artefacts) become input to Tech Spec §7 Interface section in Phase 6. Tech Spec §7 cites artefacts directly; if §7 deviates, deviation is CP3 amendment with rationale. Right plan — preserves spike authority while allowing principled deviation.

**DONE criteria align with what would convince me to ratify Phase 6 Tech Spec §7 Interface?** Yes, with one caveat (Required changes #1 below).

## Required changes for CP2 final (if any)

Two targeted edits. Single iteration; no re-draft.

1. **§5.2 — add spike fail-state narrowing to path (c) explicit linkage.** Spike §5.2.4 already says "spike failure narrows §4.2 path choice (e.g., if HRTB shape doesn't compose cleanly, path (c) B'+ activation criteria broaden)." This is the right linkage but stops short of recording what happens to the §3.2 conditions if iter-1 probe 3 fails (bare `CredentialRef<C>` outside attribute-tagged zone compiles cleanly, contradicting narrow declarative rewriting). One additional sentence: "If iter-1 probe 3 fails (narrow declarative rewriting cannot be enforced compile-time), §3.2 condition (1) for B'+ activation may shift — `CredentialRef<C>` placement constraint becomes runtime-enforced rather than compile-time, requiring CP3 §6 amendment to B'+ activation criteria." This pre-empts the silent-degradation trap if probe 3 fails. Small addition; Strategy-level not Tech Spec-level.

2. **§4.3.1 — add forward-pointer to §3.4 OUT row for `#[trait_variant::make]`.** CHANGELOG line 371 says "Out-of-scope row added to §3.4 in CP2 CHANGELOG" — but §3.4 (line 168-181) does not actually contain this row. Either:
   - Add the §3.4 row in CP2 final (preferred): "`#[trait_variant::make(Handler: Send)]` adoption" → "Separate Phase 3 redesign decision; breaks existing public `*Handler` trait surface" → "Out per §4.3.1 framing."
   - Or update §4.3.1 line 219 from "Out-of-scope row added to §3.4 in CP2 CHANGELOG" to "Out-of-scope row to be added to §3.4 in CP3 if not pre-empted" (matches the §4.3.1 actual state — row not yet present).

Picking the first resolution; row is small and belongs in §3.4 to keep OUT-tracking consolidated. Architect can pick either; both close the discrepancy.

**Optional / discretionary** (architect decides, not blocking CP2 lock):

- §4.2 path (b) per-cascade budget — CP3 §6 / Tech Spec §9 should re-baseline path (b) agent-day estimates per-cascade when the credential CP6 implementation cascade slot lands. Not a CP2 blocker; CP3 hint.
- §5.1.5 (cluster-mode hooks final shape) — Tech Spec §7 should explicitly delineate which hooks have default bodies vs require implementation. Not gating; CP3 / Tech Spec hint.

Neither blocks CP2 ratification.

## CP3 §6 hints (post-validation roadmap content)

Forward-pointers for the architect drafting CP3 §6 (post-validation roadmap):

1. **B'+ contingency activation criteria explicit.** Per §5.1.4 + my CP1 review line 116: CP3 §6 must record (a) what signal triggers B'+ over A' — credential-crate owner bandwidth gap surfaces during Tech Spec §3.4 design; cluster-mode hook design surfaces unbudgeted constraint; reviewer load on single-PR path proves untenable; iter-1 probe 3 fail surfaces compile-time-enforcement gap; (b) rollback path B'+ → A' upgrade when credential CP6 implementation cascade lands; (c) sunset commit on action's internal bridge layer (e.g., `resolve_as_<capability><C>` thunk deletion when credential CP6 internals land).

2. **§4.3.2 retry-scheduler chosen path recorded.** CP3 §6 records whether Tech Spec §9 picked wire-end-to-end vs gated-with-wired-stub. If wire-end-to-end: feature flags come down, canon §11.2 graduates "planned" → "shipped" — cite the canon edit. If gated-with-wired-stub: feature flags persist with documented sunset trigger.

3. **Spike output → Tech Spec §7 traceability.** CP3 §6 should reference the three spike artefacts (`NOTES.md`, `final_shape_v2.rs`, test artefacts) by location (likely `docs/superpowers/drafts/2026-04-24-nebula-action-redesign/spike/`) so Tech Spec §7 cites them directly. Pattern follows credential Strategy §6.1 / Tech Spec §15.12.3.

4. **Engine cluster-mode coordination cascade scheduling.** Per §3.4 row 3 (line 172), tech-lead schedules post-action-cascade close, queued behind credential CP6 implementation cascade. CP3 §6 should pin the queue position and surface it in cascade summary at Phase 8 so user sees the cluster-mode follow-up explicitly.

5. **§4.3.1 `#[trait_variant::make]` separate redesign — when?** Out-of-scope this cascade, but CP3 §6 should record a forward-flag: separate Phase 3 redesign decision opens after action cascade closes, gates on user authorization (breaks public `*Handler` trait surface).

6. **Path (b) budget re-baseline.** When user picks path (b) at Phase 8, the per-cascade budgets must be re-baselined (currently aggregate-budget framing). CP3 §6 / Tech Spec §9 records when this happens.

## Summary for orchestrator

**Verdict: RATIFY-WITH-NITS.** Two targeted edits before CP2 final lock — not a re-draft. §4.1 locked decision (A') verbatim with `03-scope-decision.md`. §4.2 path framing (a/b/c) honest, structured for user pick at Phase 8. §4.3.1 (HRTB modernization in-scope) confirms my Phase 1 02c §6 position. §4.3.2 (Retry+Terminate symmetric gating) confirms my Phase 1 `decision_terminate_gating.md` solo call. §4.3.3 codemod scope (5 transforms) sufficient for plugin migration story. §4.4 security floor verbatim with `03-scope-decision.md` §3.

§5 spike plan iter-1 (3 compile-fail probes) + iter-2 (3 actions composition + cancellation-drop + perf sanity) discharge the HRTB + cancellation dependencies before Tech Spec §7 writing. DONE criteria concrete, falsifiable, measurable.

**Required changes:** (1) §5.2 spike fail-state linkage to path (c) — one sentence covering iter-1 probe 3 fail → §3.2 condition (1) shift; (2) §4.3.1 forward-pointer to §3.4 OUT row for `#[trait_variant::make]` — row is missing from §3.4 despite CHANGELOG claim. Both small edits; architect iterates once.

**Top 2 CP3 §6 hints:** (1) B'+ contingency activation criteria — signal triggers + rollback path + sunset commit, explicit per §5.1.4; (2) §4.3.2 retry-scheduler chosen path recorded with canon §11.2 edit if wire-end-to-end picked.

**Iterate: yes (single pass, two targeted edits, no re-draft).**
