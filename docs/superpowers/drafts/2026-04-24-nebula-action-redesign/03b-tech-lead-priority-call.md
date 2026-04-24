# Phase 2 — Tech-lead priority call (scope option ranking)

**Date:** 2026-04-24
**Author:** tech-lead
**Mode:** Co-decision participant — priority decider on scope; security-lead has VETO; architect proposes options in parallel.
**Inputs read:**
- `02-pain-enumeration.md` (Phase 1 consolidated)
- `02d-architectural-coherence.md` (my own Phase 1 architectural coherence report)
- Memory: `decision_action_phase_sequencing`, `decision_terminate_gating`, `decision_controlaction_seal`, `project_action_redesign_phase1`, `project_cp6_vocab_unimplemented`
- Re-grep on credential + action `src/` (verification, see §0 below)

**Note on architect's input:** `03a-architect-scope-options.md` not yet written when this call was issued. Per cascade prompt fallback instruction, this call ranks the Phase 1 A'/B'/C' framing. If architect's options diverge materially from A'/B'/C', orchestrator should re-dispatch round 2 for re-ranking.

---

## 0. Starting position (Phase 1 reaffirmed, with one delta)

**Phase 1 position:** lean A', fallback C', NOT B' (rationale: B' requires permanent bridge layer at engine; `feedback_boundary_erosion` violation).

**Reaffirmed.** Re-grep on `crates/credential/src/` today shows partial CP6 progress: `AnyCredential` is now implemented (`crates/credential/src/contract/any.rs`, blanket impl over `Credential`). However the **phantom-rewriting / HRTB / RAII core** of CP6 — `CredentialRef<C>`, `SchemeGuard<'a, C>`, `SlotBinding`, `resolve_as_bearer`, `RefreshDispatcher` — remains spec-only. The 8 hits in `crates/action/src/error.rs` are all false positives on `CredentialRefreshFailed` (substring of `CredentialRef`). Action crate has **zero CP6 vocabulary**.

**Delta vs Phase 1 grep:** `AnyCredential` landing reduces Option A's lift modestly (the easy object-safe piece is done). Does NOT change the position — the load-bearing scope is the phantom + RAII + HRTB layer, which is still untouched. Position holds: lean A', fallback C', not B'.

**Cost framing for ranking (agent-days; my estimate as priority decider, not as architect):**
- A' (co-landed action + credential CP6 vocabulary): **8–12 agent-days**. Exceeds 5-day autonomous budget. User authorization required.
- B' (action-scoped bug-fix + hardening, defer CP6): **4–5 agent-days**. Fits budget.
- C' (escalate credential spec revision): **3 + N days**. The 3 covers tech-lead/architect drafting the revision proposal; N is unknown user-call latency to ratify the spec amendment, then ~5 more days for the revised-shape implementation.

---

## 1. Option ranking

### 1. **Option A' — Co-landed cascade (action + credential CP6)** — picked

**Rationale:** Spec-correct outcome. Single source of truth for credential shapes. Phantom-safety eliminates the cross-plugin shadow-attack class (security CR3) at compile time, not runtime. Zeroization RAII via `SchemeGuard` makes secret lifetime auditable as a type invariant. Avoids dual-vocabulary debt: action would not ship a `CredentialGuard`-shaped surface that's known-superseded the day it lands. Aligns with `feedback_active_dev_mode` (more-ideal over more-expedient) and `feedback_hard_breaking_changes` (we have license to do the right thing in one cut). Plugin-cascade-3C as one coordinated cut per `decision_action_phase_sequencing` is still tractable at this scope.

**Cost:** 8–12 agent-days. Exceeds 5-day budget. **User authorization required.** Escalation should name the budget overrun explicitly and ask user whether to (a) extend budget, (b) split into action cascade + credential cascade siblings (still A' in spirit, just two PRs in lockstep), or (c) fall back to C'.

**Dependencies:** Credential crate owner must confirm CP6 phantom/HRTB/RAII layer is on their roadmap and they will co-land. If credential owner is not signed up for the co-land, A' is not realistically deliverable and the realistic path collapses to C'.

### 2. **Option C' — Escalate credential spec revision** — second preference

**Rationale:** If A' is rejected on cost or credential-owner availability, C' is the principled fallback. C' unfreezes the credential Tech Spec to permit a **CP6-minus** vocabulary that maps 1:1 to what credential will actually ship — keeps the typed-key + scheme-typed-handle wins (eliminates CR3 shadow attack class) without requiring phantom-field-rewriting or HRTB resolve_fn ergonomics. Both crates land a coherent shape; CP7 picks up phantom-safety + RAII when authorship is ready. **Risk:** "defer to CP7" is the exact `feedback_incomplete_work` pattern unless CP7 is scheduled, owned, scoped — not "eventually."

**Cost:** 3 + N agent-days. N depends on user ratification latency. If user is willing to ratify the spec amendment fast, total is plausibly 6–8 days.

**Why not first:** A' is still architecturally cleaner if cost is unblocked. C' is the "principled retreat" — preferred over B' but second to A'.

### 3. **Option B' — Action-scoped bug-fix + hardening, defer CP6** — least preferred (veto-ish stance)

**Rationale for ranking last:** B' is local-optimal, not global-optimal. It fixes the CR-tier bugs (CR2 macro emission, CR3 type-name key, CR4 JSON depth, CR5/CR6/CR7 credential method surface, CR8/CR9 macro hygiene, CR10 Zeroize). It does NOT fix the underlying integration-shape question. Shipping a `CredentialGuard`/`ctx.credential::<C>(key)` shape — known to be superseded by CP6 within a wave or two — bakes in the **two-vocabulary problem** I flagged in §3 of `02d-architectural-coherence.md`: action speaks one credential vocabulary, credential internals speak CP6, engine becomes a permanent bridge translation layer.

**`feedback_boundary_erosion` violation:** "engine bridges" means engine has to emulate phantom-safety at runtime — typed-key lookups that fail at runtime instead of compile time. That's a strictly larger error surface, and the bridge becomes the kind of "small helper in the wrong crate that compounds" the feedback memory warns about. **`feedback_active_dev_mode` violation:** B' is the textbook gate-and-defer / cosmetic-quick-win pattern.

**`feedback_hard_breaking_changes` lens:** we have license to break things in a single coordinated cut; B' deliberately leaves the harder break for a future cascade. That's borrowing from future-us at a known interest rate.

**Veto-ish stance, not absolute veto:** if user authorization for A' is denied AND C' escalation is not available, B' becomes the realistic fallback — but I will explicitly flag in cascade output that B' ships with a known sunset, and the post-cascade tracker must include "credential CP6 adoption" as a real issue, not a TODO comment.

---

## 2. Load-bearing rationale (3 paragraphs)

**Cost vs correctness trade-off.** The 5-day autonomous budget was sized for an action-scoped cascade — fixing macro bugs, sealing ControlAction, wiring Terminate, hardening JSON depth, fixing the credential key heuristic. Phase 1 reframed the problem: the credential idiom is broken on both sides of the boundary (CP6 vocabulary unimplemented in credential crate itself), so any scope that "fixes credential integration" by definition touches both crates. A' acknowledges this honestly — 8–12 agent-days for the right shape. B' pretends the budget can absorb the symptom and defer the cause. The pretend cost is the dual-vocabulary debt: every plugin written against B' will need to be rewritten when CP6 lands; every line of "engine bridges" code is a maintenance liability we own indefinitely. Per `feedback_active_dev_mode`, settling for "green tests / cosmetic / quick win / deferred" is the mode I'm specifically told to refuse. The active-dev license is to think ahead, finish partial work, prefer more-ideal over more-expedient — that's A' verbatim.

**User authorization requirement (explicit).** Picking A' triggers escalation rule 10 (budget overrun) per cascade protocol. The escalation message orchestrator writes should name three sub-options for the user: (a) **extend budget to ~12 agent-days for one coordinated A' cascade**; (b) **split into two sibling cascades** — credential CP6 implementation cascade + action redesign cascade running in lockstep with hard sequencing (credential leaf-first, action consumer-second), each fitting a normal budget; (c) **fall back to C'** with the user ratifying the credential Tech Spec amendment. (b) is operationally similar to A' but lets each cascade fit a 5-day window; my preference among the three is (b) if credential-crate owner has bandwidth, else (a). C' is the principled retreat if neither (a) nor (b) is feasible. **B' should not be the user's first-offered option** — it's a fallback, not a peer.

**Migration path realism + boundary erosion.** `decision_action_phase_sequencing` already lays out 3A engine-internal → 3B action-internal-settled → 3C plugin-breaking as one coordinated cut. A' keeps that sequencing intact: 3A and 3B ship without plugin impact (independent of which option wins); 3C is where the credential vocabulary actually lands. The plugin-blast-radius (7 reverse-deps × 69 files × 63 public items × ~40 through `sdk::prelude`) is tractable for one coordinated migration, per `feedback_hard_breaking_changes`. If we pick B' instead, 3C ships a known-superseded surface, then a future credential-CP6 cascade comes back through the same plugin ecosystem with another 3C-shaped breaking change — same blast radius, twice. That's the "10x next month" cost the new-month test is designed to catch. **Boundary erosion check:** A' keeps the credential vocabulary inside the credential crate (where it belongs). B' creates an engine-side bridge layer that emulates phantom-safety at runtime — a permanent cross-crate responsibility smear. C' is clean (both crates land a coherent shape, just a more conservative one). A' and C' both pass the boundary check; B' fails it.

---

## 3. Trade-offs accepted by picking A'

**What A' costs us, named explicitly:**

1. **Coordination risk with credential cascade.** A' requires either (a) one large coordinated cascade or (b) two sibling cascades in lockstep. If credential-crate owner slips, action cascade slips with it. Phase 1 §3 of `02d-architectural-coherence.md` already named this risk; I am accepting it.
2. **Budget overrun, user authorization required.** A' cannot ship within the 5-day autonomous budget. Cascade pauses on user decision — escalation cost is real (latency + the user has to think about it).
3. **Macro discoverability cost.** CP6 vocabulary requires an attribute macro (`#[action(...)]` rewriting field types), not the current `#[derive(Action)]`. Per §2 of `02d-architectural-coherence.md`, attribute macros that silently rewrite field types hurt LSP/grep/goto-def — we mitigate by constraining the rewriting to a documented narrow zone (e.g., `#[action(credentials(slack: SlackToken))]` rather than rewriting arbitrary fields). The mitigation is real but adds DX nuance.
4. **Plugin migration day cost.** Phase 3C is one coordinated breaking PR touching 7 reverse-deps. Migration codemod + docs are mandatory. Plugin authors lose a day reviewing.
5. **Schedule pressure on credential-crate owner.** A' assumes credential-crate owner has bandwidth to land the CP6 phantom/HRTB/RAII layer in this cascade window. If they don't, A' isn't deliverable as A'.

**What I'm explicitly NOT giving up:**
- Macro test harness (trybuild + macrotest) lands in 3A regardless of option.
- ControlAction seal + DX-tier ratification (canon §3.5) lands in 3A regardless of option.
- `ActionResult::Terminate` feature-gate + scheduler wiring lands in 3A regardless of option.
- Lefthook parity fix, deny.toml layer rule, zeroize workspace pin land in 3A regardless of option.
- Security CR3 (cross-plugin shadow attack) is FIXED by A' at compile time. CR4 (JSON depth bomb) is fixed at adapter boundary regardless of option.

---

## 4. Conditions for reconsideration

**If security-lead vetoes A':** I would expect the veto to come on the form of "phantom-rewriting macro is too opaque for security audit" or "co-landing two cascades multiplies the review surface beyond what we can audit safely." In either case, fallback is **C'**, not B'. C' lands a typed-key + scheme-typed-handle vocabulary that still eliminates the shadow-attack class without phantom-rewriting; security-lead's audit surface is smaller. If security-lead vetoes BOTH A' and C', I will surface that to orchestrator as a deadlock and hand back to user.

**If user authorization for A' is denied (budget):** Fallback ordering: (b) sibling-cascades A' if credential owner has bandwidth → (c) C' escalation → (B') as last resort with explicit sunset flag.

**If credential-crate owner cannot co-land:** A' is not deliverable. Fallback to C'.

**If consensus locks on B' despite my position:** I will not block ratification, but I will flag the sunset risk in cascade output. Specifically, the post-cascade tracker MUST include a real issue titled "credential CP6 vocabulary adoption (action + credential)" with a defined target wave, owned, scoped — not a TODO comment, not "tracked for Phase 3 of the ControlAction plan" prose. Per `feedback_incomplete_work`, "deferred" only counts when the deferral is real.

**If architect's `03a-architect-scope-options.md` lands with options that diverge materially from A'/B'/C':** orchestrator should re-dispatch round 2 with architect's options as the input frame; this priority call is on the Phase 1 framing.

---

## 5. Handoff posture

- **Primary handoff:** orchestrator dispatches `03a-architect-scope-options.md` (architect) and `03c-security-lead-veto-review.md` (security-lead) in parallel; consensus convergence happens at `03d-phase2-scope-locked.md` (orchestrator-owned).
- **If security-lead veto surfaces on A':** orchestrator re-dispatches round 2 with the veto rationale as input; my round-2 output will rank C' first, B' second.
- **If user authorization needed (A' budget overrun):** orchestrator writes `ESCALATION.md` naming the three sub-options ((a) extend budget, (b) sibling cascades, (c) fall back to C') and pauses cascade pending user decision. I do not pre-empt the user call.
- **If consensus locks on B' despite my preference:** I will not block. I will flag the sunset risk and require the post-cascade tracker include a real CP6-adoption issue per §4 condition.
- **If architect's options diverge from A'/B'/C':** request orchestrator re-dispatch round 2 with architect's frame.

---

## Summary (≤150 words)

**Option picked: A'** (co-landed action + credential CP6 vocabulary).
**Cost:** 8–12 agent-days. Exceeds 5-day budget — **user authorization required**.
**Top reason:** B' bakes in two-vocabulary debt + permanent engine-side bridge layer (`feedback_boundary_erosion` violation) + textbook gate-and-defer (`feedback_active_dev_mode` violation). A' is the spec-correct outcome; eliminates security CR3 (cross-plugin shadow attack) at compile time, not runtime; aligns with `feedback_hard_breaking_changes` license to break in one coordinated cut.
**Escalation trigger:** user authorization for budget overrun. Orchestrator writes `ESCALATION.md` with three sub-options: (a) extend budget, (b) sibling cascades A' (credential leaf-first + action consumer-second in lockstep), (c) fall back to C' (escalate credential spec revision). B' is fallback only — not user's first-offered option. If security-lead vetoes A', fallback is C', not B'.

---

*End of tech-lead priority call. Routes to orchestrator for security-lead veto review and consensus convergence.*

---

## Round 2 re-rank — with B'+ hybrid included

**Date:** 2026-04-24 (same day, round 2)
**Trigger:** Architect's `03a-architect-scope-options.md` introduced Option B'+ (action-scoped + structural CP6 surface prep) which materially diverges from the Phase 1 A'/B'/C' frame I ranked in round 1. Per my round 1 §4 condition + §5 handoff posture, orchestrator re-dispatched. This is a re-rank, not a re-review — round 1 rationale stands; this section adjusts for the new option and the cascade-scope clarification.

### Context shift

Two material changes since round 1:

1. **Cascade scope clarified as DESIGN-ONLY.** Orchestrator confirmed: cascade budget covers Strategy + Spike + ADR + Tech Spec + summary. Implementation (the 8–22 agent-days I framed in round 1) is post-cascade work, gated separately by user. This collapses the budget axis materially:
   - A' design-effort: ~3–4d architect-time (Tech Spec drafting + 1 spike).
   - B'+ design-effort: ~2d architect-time (Tech Spec drafting + 1 spike).
   - B' design-effort: ~1–1.5d architect-time.
   - C' design-effort: ~2–2.5d architect-time + spec-amendment cycle.
   - **All four options fit cascade design budget.** My round 1 framing of "A' triggers escalation rule 10 via budget overrun" was based on conflating implementation cost with cascade cost. That conflation is dropped.
2. **Architect proposed B'+** (Option B + structural CP6 surface prep). The new option is materially distinct from B' because the API-shape commitment lands now (in this cascade's Tech Spec); only internals are deferred. Tech Spec freezes the public CP6 surface; credential-cascade follow-up swaps internals without touching the surface plugin authors see.

### Revised ranking (all 4 options)

#### 1. **Option A' — Co-landed cascade** — picked (still)

**Why first:** Budget-axis collapse removes the round 1 escalation-via-budget concern. A' was my round 1 first preference because it produces the spec-correct outcome in one coordinated cut — that rationale was never about budget, it was about avoiding two-vocabulary debt and engine bridges. With cascade-scope clarified as design-only, A's design effort fits autonomous budget cleanly. Implementation cost (8–22d) is a post-cascade question for the user — but the design closure work is what we're sizing. A' Tech Spec produces the artifact that allows implementation to be one coordinated cut later, with no surface-vs-implementation gap from day one. Honors `feedback_hard_breaking_changes` and `feedback_active_dev_mode` simultaneously, eliminates security CR3 by construction (phantom typing), zero boundary-erosion risk.

#### 2. **Option B'+ — Action-scoped + CP6 surface prep** — second (NEW; promoted above C')

**Why second:** B'+ commits the same public API shape as A' in this cascade's Tech Spec, deferring only internals. From the plugin author's perspective the result is indistinguishable from A' — they declare `CredentialRef<C>` fields, use `ctx.credential::<S>(key)`, get `SchemeGuard<'a, C>` RAII handles. The surface-vs-implementation gap is internal to the credential crate and resolved when credential-cascade lands CP6 internals. **Architect's claim is correct** that this is not a bridge layer in engine: see boundary-erosion assessment below. Beats C' on spec-compliance shape (CP6 letter, not CP6.1) and on `feedback_hard_breaking_changes` (one cut to spec-correct surface, plugins do not re-migrate).

**Why not first:** B'+ depends on credential-cascade landing CP6 internals on a reasonable timeline. If credential-cascade slips beyond ~1 quarter, the surface-vs-implementation gap widens and we accumulate a window where the contract held but isn't enforced by construction. A' closes that window now. Architect's framing — "the optimistic path; depends on credential cascade landing within reasonable timeframe; otherwise B'+ degrades to B' silently" — is honest. I am willing to take that bet only if:
- The credential CP6 implementation cascade has a committed slot before this Phase 2 closes (per `feedback_active_dev_mode` "before saying 'defer X', confirm the follow-up has a home").
- `CredentialRef<C>` / `SlotBinding` / `SchemeGuard` land in **nebula-credential** (architect's "land in credential as the final home from day one" preference), NOT in nebula-action's `credential` module. If they land in action with internals delegating to credential, that's a different question — see boundary-erosion section.

#### 3. **Option C' — Escalate credential spec revision (CP6 → CP6.1)** — third (DEMOTED from second in round 1)

**Why third now:** B'+ dominates C' on spec-compliance shape (full CP6 surface vs CP6.1 partial), on plugin author experience (no re-migration when CP7 lands; CP6.1 makes phantom-safety/RAII a future plugin-breaking change), and on cascade flow (no spec-revision authorization needed). Round 1 ranked C' second because it was the "principled retreat" if A' was infeasible on cost. With cascade scope clarified as design-only, the cost-axis collapse benefits A' and B'+ more than C', because C's spec-amendment cycle is a process cost (latency + co-decision rounds) that doesn't compress with the design-only scope. C' remains acceptable — it's principled and has zero surface-vs-implementation gap — but its primary virtue (smaller implementation surface) is a post-cascade concern, not a cascade concern.

**Why C' over B':** C' is internally consistent (CP6.1 spec defines the surface honestly); B' is the highest erosion-risk option per round 1.

#### 4. **Option B' — Action-scoped only, no CP6 surface prep** — last (still)

**Why last:** Round 1 rationale stands. B' bakes in two-vocabulary debt + boundary-erosion risk. The cascade-scope clarification doesn't rescue B' — design-only budget for B' (~1–1.5d) is the cheapest option but the deferred work has the largest blast radius (plugin re-migration when CP6 lands later). The "10x next month" cost the next-month-test catches.

**Round 1 veto-ish stance preserved:** B' should not be the user's first-offered option. With B'+ now on the table, B' has a strictly-better hybrid sibling — there is no scenario where I prefer B' over B'+ if `CredentialRef<C>` lands in nebula-credential.

### Does B'+ resolve your boundary-erosion concern?

**Architect's argument** (`03a` §1 B'+ scope, §3.7, §5):
> The API-shape commitment lives in **nebula-action's public surface** (where it semantically belongs — action authors declare credential dependencies), **not in engine's runtime translation**. The "internal swap" pattern is not a bridge layer; it is a deliberate forward-compatibility move sanctioned by the architect at design time.

**My assessment: YES — with one structural condition.**

The boundary-erosion concern in round 1 was not "any kind of forward-compatibility commitment is bridge-layer behavior." It was specifically: **if engine has to translate between action's typed-key vocabulary and credential's phantom-typed vocabulary at runtime, that's the engine accumulating a permanent cross-crate translation responsibility that doesn't belong there.** Engine is the consumer; engine should not be where the impedance mismatch gets reconciled.

B'+ relocates the impedance mismatch from "engine bridges between two crate vocabularies" to "credential crate's internals delegate to its own existing types until CP6 internals land." That's a fundamentally different architecture:

- **B' (the rejected option):** action ships pre-CP6 vocab → engine must translate to/from CP6 internals when credential lands them. Engine accumulates translation code. Translation code lives in the wrong crate (engine doesn't own credential semantics). **Boundary erosion.**
- **B'+ (the new option, *if* CredentialRef etc. land in nebula-credential):** action and credential both speak CP6 vocab as their public surface. Credential's `CredentialRef<C>` initially delegates to current credential internals (the existing typed-key path); when CP6 phantom/HRTB/RAII layer lands, credential swaps its own internals. **Engine never sees two vocabularies.** No translation. **Not boundary erosion.**
- **B'+ (variant where CredentialRef etc. land in nebula-action's `credential` module):** This IS boundary erosion — credential vocabulary living in action crate, with action delegating up to credential. Per `feedback_boundary_erosion` "which crate owns this concept?" — credential vocabulary belongs in credential. This variant is rejected.

**Structural condition for my pick:** B'+ is acceptable as a second-choice **only if** `CredentialRef<C>` / `SlotBinding` / `SchemeGuard<'a, C>` / `AnyCredential` land in `nebula-credential`'s public surface in this cascade's Tech Spec, not in nebula-action's `credential` module. Architect's stated "leaning preference is land in credential to avoid future re-home" matches. Tech Spec must lock this — not as a footnote, but as the §7 Interface section's primary placement decision.

**Secondary check (separately load-bearing):** the "internal swap" needs to be the credential crate swapping its own internals — not an external mechanism. If credential's `CredentialRef<C>` initial implementation requires action-side code to know which path it's on, the swap is observable across the boundary, which is itself erosion in disguise. Tech Spec needs to assert: action-side code is identical pre-and-post internal swap. Architect's "plugin authors do not re-migrate" claim implies this; Tech Spec must commit to it.

With those two conditions satisfied: **B'+ does escape boundary erosion.** Architect's argument is sound on the merits.

### Updated escalation posture

**Cascade-scope = design-only changes the escalation picture materially:**

| Option | Round 1 escalation trigger | Round 2 escalation trigger |
|---|---|---|
| A' | budget overrun (rule 10) → user authorization | **none for cascade**; user authorization required separately for post-cascade implementation effort |
| B'+ | not surfaced in round 1 (option didn't exist) | **none for cascade**; CP6 cascade slot must be committed before Phase 2 closes (active-dev-mode requirement) |
| B' | none for cascade; high-risk follow-up | **none for cascade**; high-risk follow-up |
| C' | spec revision (rule 10) → user authorization | **spec revision (rule 10) → user authorization** (unchanged — process cost doesn't compress) |

**Concretely:**

- **Does A' still need user authorization?** **For the cascade itself, no.** Design closure (Tech Spec + spike + ADR) fits autonomous budget. **For post-cascade implementation, yes** — 18–22d implementation effort is a separate user-gated decision. The cascade should produce the Tech Spec and ESCALATION.md should ask user whether to schedule A' implementation as one coordinated effort, or split into sibling cascades (credential-leaf + action-consumer in lockstep). My round 1 sub-options (a)/(b)/(c) for the user remain valid, just relocated to post-cascade.
- **Does B'+ need user authorization?** **For the cascade itself, no.** Design closure fits autonomous budget. **For post-cascade implementation, yes** — but B'+ implementation is meaningfully smaller (4–5d action-side; credential CP6 internals follow in their own cascade). Lower-friction user authorization than A'. **The mandatory addition:** user must commit to a credential CP6 implementation cascade slot — not a TODO, not "tracked for next phase," a real cascade slot — before Phase 2 closes. If user doesn't commit, B'+ degrades to B' silently and we land the boundary-erosion-risk option without the safeguard.
- **Does C' still trigger escalation rule 10?** **Yes — unchanged.** C' unfreezes credential Tech Spec CP6 (frozen across recent commits 65443cdb / 33eb3f01 / 883ccfbf in last 2 weeks). Spec amendment requires user authorization regardless of cascade-scope clarification. This is a process trigger, not a budget trigger.

### Conditions for reconsideration

Round 1 conditions all stand; updated for round 2:

- **If security-lead vetoes A':** Round 1 fallback was C' (not B'). With B'+ on the table, fallback is now **B'+ (with structural condition above) → C' → B' as last resort.** B'+ slots in cleanly between A' and C' on the post-veto rank.
- **If user denies post-cascade A' implementation effort:** Fallback ordering — sibling-cascades A' (credential leaf + action consumer in lockstep) → B'+ → C' → B'.
- **If credential-crate owner cannot co-land:** A' is not deliverable in one coordinated effort. Realistic path collapses to either sibling-cascades A' (if owner can co-land in next cascade) or **B'+ as the bridge** (action ships forward-compatible surface; credential-cascade lands CP6 internals when owner has bandwidth). **B'+ becomes my preferred fallback in this scenario** — better than C' because plugins do not re-migrate.
- **If user does not commit a credential CP6 implementation cascade slot:** B'+ is not selectable. Either A' (sized to land everything in one effort) or C' (honest spec revision). B'+ without the follow-up commitment IS the forbidden quick-win-trap mindset per `feedback_active_dev_mode`.
- **If consensus locks on B' despite my position:** Round 1 stance preserved — I do not block, but I flag sunset risk and require a real CP6-adoption issue in post-cascade tracker. This condition is now strictly weaker because B'+ exists as a strictly-better hybrid; I cannot imagine a consensus path to B' that doesn't first reject B'+.

---

## Round 2 summary (≤150 words)

**Updated pick: A' first, B'+ second, C' third, B' last.** Cascade-scope clarified as design-only collapses the budget axis — all four options fit autonomous cascade budget; only C' triggers escalation (rule 10, spec revision). A' remains my first preference for the same round 1 reason: spec-correct outcome, single source of truth, zero boundary-erosion risk. **B'+ does escape boundary erosion** — but only if `CredentialRef<C>` / `SlotBinding` / `SchemeGuard` land in nebula-credential (not in nebula-action's `credential` module), and only if user commits a credential CP6 implementation cascade slot before Phase 2 closes. With both conditions met, B'+ relocates the impedance mismatch to credential-internal swaps; engine never sees two vocabularies, which is what `feedback_boundary_erosion` actually guards against. B'+ now slots above C' on post-veto and post-budget-denial fallback paths. B' demoted strictly — no scenario where I prefer it over B'+ if the structural condition holds.

---

*End of round 2 re-rank. Routes to orchestrator for consensus convergence with security-lead's round 2 position.*
