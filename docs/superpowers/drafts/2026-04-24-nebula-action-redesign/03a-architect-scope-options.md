# Phase 2 — Scope options (architect proposer position)

**Date:** 2026-04-24
**Author:** architect (sub-agent, proposer-not-decider)
**Mode:** Co-decision protocol — architect proposes; tech-lead picks (priority call); security-lead has VETO on options that drop CR3 / CR4.
**Inputs:** Phase 1 consolidated (`02-pain-enumeration.md`), tech-lead Phase 1 position (`02d-architectural-coherence.md`), security-lead Phase 1 threat model (`02b-security-threat-model.md`), dx-tester Phase 1 authoring report (`02a-dx-authoring-report.md`), rust-senior Phase 1 idiomatic review (`02c-idiomatic-review.md`), Phase 0 consolidated (`01-current-state.md`).

---

## 0. Proposer posture

I am explicitly **not** picking a winning option. My role here is to:

1. Lay out the option universe with honest cost estimates and trade-offs.
2. Ensure every option that survives the table addresses CR3 (cross-plugin shadow attack) and CR4 (JSON depth bomb) — security-lead's VETO triggers.
3. Surface the genuine load-bearing trade-off lines so tech-lead can apply his priority weighting and security-lead can apply their hardening weighting without reading 10 reports.
4. Stand ready to draft the Strategy Document (Phase 3) under whichever option is selected.

**Tech-lead's Phase 1 position is on record:** lean A', fallback C', not B'. My options must give him a path to keep this preference cleanly OR present compelling evidence to reconsider — they should not paint him into a corner. I therefore extend the universe with two refinements (A'-phased, B'+) that shift the trade-offs without requiring him to abandon his stated lean.

**Cost units are agent-days.** 1d = a focused day of agent work; 5d is the cascade autonomous budget hard stop. Estimates are order-of-magnitude (1d, 3d, 5d, 10d, 20d), not promises.

---

## 1. Option inventory

### Option A' — Co-landed cascade (action + credential CP6 implementation)

- **Scope summary (what's IN):**
  - Credential crate implements CP6 vocabulary leaf-first: `CredentialRef<C>`, `AnyCredential` object-safe supertrait, `SlotBinding` with HRTB `for<'ctx> fn(...) -> BoxFuture<'ctx, _>` resolve_fn, `SchemeGuard<'a, C>` RAII (`!Clone`, `ZeroizeOnDrop`, `Deref`), `SchemeFactory<C>` re-acquisition, `RefreshDispatcher::refresh_fn` HRTB.
  - Action crate adopts `#[action]` attribute macro (replacing `#[derive(Action)]`) with **narrow declarative rewriting contract**: rewriting confined to attribute-tagged credential/resource zones (per tech-lead §2 input — not arbitrary fields). `CredentialRef<C>` field support; `ActionSlots` impl emission; new `ActionContext` methods matching CP6 spec (`ctx.credential::<S>(key)` / `credential_opt::<S>(key)` per v2 spec §3).
  - Engine wires `resolve_as_<capability><C>` helpers, registers slot bindings, honors HRTB fn-pointers; depth cap added to all adapter JSON boundaries.
  - All 11 🔴 CR findings addressed end-to-end. Macro test harness added (trybuild + macrotest). HRTB `*Handler` boilerplate modernized to single-`'a` + type alias (rust-senior §6). `ControlAction` sealed (tech-lead §5 ratified solo call). `ActionResult::Terminate` feature-gated + scheduler-wired (tech-lead §5 ratified solo call).
  - Plugin ecosystem migration: 7 reverse-deps, 69 source files, 63 public items, ~40 cascaded through `sdk::prelude` — coordinated single-cut migration in Phase 3C per tech-lead §6 sequencing.
- **Deferred items (what's OUT):**
  - Cluster-mode coordination (leader election, reconnect orchestration) — engine layer concern; surfaces 3 hooks (`IdempotencyKey`, `on_leader_*` lifecycle, dedup window) on `TriggerAction` per tech-lead §4. Engine implementation in separate cascade.
  - DataTag hierarchical registry (Phase 0 S5) — net-new surface; deferred to a port-system sub-cascade.
  - `Provide` port kind (Phase 0 S5) — same.
- **Cost estimate:** **18-22 agent-days.** Rationale:
  - Credential CP6 implementation (leaf): 8-10d. Tech Spec CP6 §§2.7/3.4/7.1/15.7 specifies the shapes; rust-senior §5 confirms HRTB fn-pointer is "the smallest thing that works"; SchemeGuard RAII + SchemeFactory + RefreshDispatcher are non-trivial (zeroization invariant + cancellation drop-order tests required).
  - Action attribute macro (replacing derive) with narrow rewriting contract: 4-5d. New parser, expansion strategy, and trybuild + macrotest harness from scratch (T1 gap).
  - Engine wiring (resolve_as_*, slot registration, depth cap, idempotency-key plumbing, Terminate scheduler integration): 3-4d.
  - Plugin migration (codemod for 7 reverse-deps × 69 files; sdk::prelude alignment; doc updates): 2-3d.
  - **This exceeds the 5d autonomous budget by 3.5-4×.** Escalation almost certain.
- **Blast radius:**
  - **Crates touched:** `nebula-action`, `nebula-action-macros`, `nebula-credential`, `nebula-engine`, `nebula-sdk`, `nebula-api`, `nebula-sandbox`, `nebula-plugin`, `nebula-cli`. Plus indirect: `nebula-resource` (`on_credential_refresh` hook to consume CP6 RefreshDispatcher) — possibly 10 crates total.
  - **Public API surfaces changed:** `Action` trait family (HRTB modernization is non-breaking re-shape but visible in rustdoc); `CredentialContextExt` removed entirely; `#[derive(Action)]` removed in favor of `#[action]` attribute macro; `CredentialGuard` superseded by `SchemeGuard<'a, C>`; new `CredentialRef<C>`/`SlotBinding`/`AnyCredential` surfaces in nebula-credential public API; `sdk::prelude` re-export block fully reshuffled.
- **Critical findings addressed:** CR1 ✅ (CP6 vocabulary lands in both crates), CR2 ✅ (derive removed; new attribute macro has correct emission verified by macrotest), CR3 ✅ (explicit-key vocabulary eliminates type-name heuristic structurally), CR4 ✅ (depth cap added), CR5 ✅ (string-form removed), CR6 ✅ (`CredentialLike` superseded by `CredentialRef<C>` with concrete impls in credential crate), CR7 ✅ (spec methods land), CR8 ✅ (semver path qualified by macro emission audit), CR9 ✅ (`HasSchema` bound documented + auto-derived on `()` / `Value`), CR10 ✅ (no-key variant removed), CR11 ✅ (trybuild + macrotest landed).
- **Risks:**
  - **Coordination risk** between credential CP6 implementation and action redesign — slip in either delays both. Two in-flight specs on the critical path.
  - **Spec-reality drift during implementation** — CP6 spec shapes may need amendment when implementation reveals constraints; per `feedback_adr_revisable`, that's allowed but each amendment is a co-decision cycle that costs 0.5-1d.
  - **Plugin ecosystem disruption** — 7 reverse-deps × 69 files in one cut; per `feedback_hard_breaking_changes` this is licensed but creates a single high-stakes review event.
  - **Macro silent rewriting DX risk** — even with narrow declarative rewriting, goto-def on a credential field may land inside macro expansion. Mitigated by tech-lead §2 recommendation to constrain rewriting to attribute-tagged zones (e.g., `#[action(credentials(slack: SlackToken))]` emits phantoms rather than rewriting `slack: CredentialRef<SlackToken>`).
- **Migration story:**
  - Plugin authors see hard break: `#[derive(Action)]` removed; `ctx.credential_by_id(...)` removed; new `#[action]` attribute + `CredentialRef<C>` declarations + `ctx.credential::<C>(key)` access pattern.
  - Deprecation window: **none** (per `feedback_hard_breaking_changes` — no shims, no aliases). Pre-1.0 alpha; semver-checks advisory-only (Phase 0 T7).
  - Codemod ships alongside redesign: shell script that mechanically rewrites `#[derive(Action)]` → `#[action]` + flags credential access call sites for manual review (semantic mapping to typed key cannot be automated).
  - Migration guide as part of Tech Spec §9 (handoff section).
- **Tech Spec writing effort:**
  - Sections required: Goal & non-goals; Lifecycle (action authoring lifecycle); Storage schema (CredentialRef phantom layout, SlotBinding registry); Security (CR3/CR4 closure, SchemeGuard RAII invariants, HRTB fn-pointer phantom-safety argument); Operational (observability for resolve failures, refresh telemetry); Testing (trybuild + macrotest + cancellation-drop integration tests); Interface (full public API surface for action + credential additions); Open items / accepted gaps; Migration / handoff (codemod + plugin author migration guide).
  - **Spike requirements: 1 spike confirmed (HRTB fn-pointer + SchemeGuard cancellation drop-order)** — rust-senior §5 frames the shape as known-correct; spike validates compile + cancellation behavior end-to-end before Tech Spec freezes.
  - Estimated drafting: ~4 checkpoints at architect cadence (§1-3 → §4-6 → §7-9 → final cross-section pass). Spans ~3-4d of architect-time absorbed inside the 18-22d total.

### Option B' — Action-scoped bug-fix + hardening (defer CP6 to separate credential cascade)

- **Scope summary (what's IN):**
  - Fix CR2/CR5/CR6/CR8/CR9/CR10 — all macro emission bugs and currently-unusable credential surfaces.
  - Fix CR3 (cross-plugin shadow attack) by **deprecating `credential<S>()` no-key variant entirely** + requiring explicit key in `credential_typed::<S>(key)` + sanitizing error-channel type-name leak (S-C3). The deprecation is hard removal — no `#[deprecated]` shim — per `feedback_no_shims`.
  - Fix CR4 (JSON depth bomb) by adding depth cap at all adapter boundaries (`StatelessActionAdapter`, `StatefulActionAdapter`, plus per-trigger pre-checks where serde_json::Value reconstruction occurs).
  - Add macro test harness (trybuild + macrotest) — CR11. Includes regression coverage for every emission path that survives the redesign.
  - Seal `ControlAction` (tech-lead §5 ratified solo call) — close the public-non-sealed-trait door without canon revision needed for new variant.
  - Feature-gate + scheduler-wire `ActionResult::Terminate` (tech-lead §5 ratified solo call) — match `Retry` discipline; finish partial work per `feedback_active_dev_mode`.
  - Modernize `*Handler` HRTB boilerplate to single-`'a` + type alias (rust-senior §6) — cosmetic but removes the `'life0`/`'life1` historical artifact, reduces HRTB surface ~30-40% LOC.
  - Add Webhook S-W2 mitigation: constrain `SignaturePolicy::Custom` to a closed set of named verifier kinds (no anonymous closures), or attach attestation metadata to the `Custom` variant. Security-lead 🟠 finding addressed even though not 🔴.
  - Workspace hygiene: zeroize workspace-pin (T4), dead `nebula-runtime` reference fix (T3), lefthook parity (T5), add deny.toml layer rule for nebula-action (T9), retire unstable-retry-scheduler dead feature (T2).
- **Deferred items (what's OUT):**
  - **CP6 vocabulary entirely** — `CredentialRef<C>`, `SlotBinding`, `SchemeGuard<'a, C>`, `SchemeFactory<C>`, `RefreshDispatcher`, `AnyCredential`. Action retains a simpler typed access surface (`ctx.credential_typed::<S>(key)`) without phantom rewriting. Future credential-only cascade implements CP6 wholesale.
  - **Cluster-mode hooks** (`IdempotencyKey`, lifecycle callbacks, dedup window) — even though tech-lead §4 frames these as small, they couple to engine work that's not in this cascade.
  - **DataTag hierarchical registry** (Phase 0 S5) — same as A'.
  - **`Provide` port kind** (Phase 0 S5) — same as A'.
  - **`Resource::on_credential_refresh` hook** on `ResourceAction` — depends on CP6 RefreshDispatcher landing first.
  - **Filed sub-spec pointer:** "credential CP6 implementation cascade" — `docs/superpowers/specs/<future>-credential-cp6-impl-tech-spec.md`. Per `feedback_active_dev_mode` rule "before saying 'defer X', confirm the follow-up has a home" — this requires the user / tech-lead to commit a follow-up cascade slot, not just a TODO.
- **Cost estimate:** **4-5 agent-days.** Rationale:
  - Macro fixes (CR2/CR5/CR6/CR8/CR9/CR10) + harness setup (CR11): 1.5-2d. Macro is 359 LOC; trybuild snapshots + 9+ rejection-rule fixtures are mechanical work.
  - CR3 deprecation + explicit-key + error-channel sanitization: 0.5d.
  - CR4 depth cap at all adapter boundaries (4 sites): 0.5d.
  - HRTB modernization across 5 handler traits + adapters: 0.5-1d. Mechanical replacement; existing `dyn_compatible` tests catch regressions.
  - ControlAction seal + Terminate scheduler wiring + S-W2 mitigation: 1d.
  - Workspace hygiene (T2/T3/T4/T5/T9): 0.5d.
  - Plugin migration (smaller blast radius — only the macro syntax + credential method names change): 0.5d.
  - **Fits the 5d autonomous budget.** Tightest at 5d; orchestrator should plan for 4d core work + 1d slack.
- **Blast radius:**
  - **Crates touched:** `nebula-action`, `nebula-action-macros`, `nebula-engine` (Terminate scheduler wiring + depth cap pass-through), `nebula-sdk` (prelude re-shuffle for renamed methods).
  - **Public API surfaces changed:** `#[derive(Action)]` macro emission corrected (all attributes now actually work); `CredentialContextExt` reshaped — three methods reduce to two (typed-with-key + by-id); `*Handler` HRTB cosmetic shape change (visible in rustdoc); `ActionResult::Terminate` becomes feature-gated; `SignaturePolicy::Custom` shape narrowed.
  - Smaller cascade than A': ~3-4 crates touched; ~25-30 source files. `sdk::prelude` impact narrower (only credential method renames).
- **Critical findings addressed:** CR1 ❌ (CP6 vocabulary deferred), CR2 ✅, CR3 ✅ (via deprecation+removal+explicit-key), CR4 ✅, CR5 ✅, CR6 ❌ → ✅ partial (CredentialLike removed entirely; replaced by direct typed-key access), CR7 ❌ (spec methods adopted but spec semantics — `CredentialRef<C>` phantom — deferred), CR8 ✅, CR9 ✅, CR10 ✅, CR11 ✅.
  - **CR1 + CR7 unaddressed by spec letter.** Action ships with a credential surface that does not match credential Tech Spec CP6's user-facing API (`ctx.credential::<C>(key)` exists in name but not in shape — no phantom safety, no SchemeGuard).
  - This is what tech-lead §3 calls "two vocabularies long-term."
- **Risks:**
  - **Permanent bridge layer risk (tech-lead §3):** If credential cascade later implements CP6 vocabulary, the engine acquires a translation layer between action's typed-key access and credential's phantom-typed access. Per `feedback_boundary_erosion` this is a boundary erosion in waiting.
  - **Two-vocabulary window:** while credential cascade is in flight, action speaks a different shape than credential's spec — drift accumulates per `feedback_incomplete_work`.
  - **Plugin churn re-cost:** plugins migrate twice — once for B' (rename methods, fix macro syntax), once again when credential cascade eventually lands CP6 (replace typed-key access with `CredentialRef<C>` declarations). User has license to do this per `feedback_hard_breaking_changes`, but it doubles plugin-author burden.
  - **Follow-up rot risk:** "deferred to credential cascade" rates as "follow-up that needs a home." If user does not commit a credential cascade slot before Phase 2 lands, this is a quick-win trap (`feedback_active_dev_mode` explicit forbidden mindset).
- **Migration story:**
  - Plugin authors see one moderate break: `#[derive(Action)]` syntax fixes (now the documented attributes actually work); credential method calls renamed (`credential<S>()` → `credential_typed::<S>(key)`); `ResourceAction` resource access typed (per spec §4).
  - Deprecation window: **none** (per `feedback_no_shims`). Pre-1.0 alpha.
  - Codemod for the macro syntax fixes is automatable; credential call site rewrites need manual review (which type? which key?).
- **Tech Spec writing effort:**
  - Sections: Goal & non-goals; Lifecycle (action authoring lifecycle, abbreviated); Storage schema (depth-cap layout); Security (CR3/CR4 closure + S-W2 mitigation); Operational (Terminate scheduler integration semantics); Testing (trybuild + macrotest harness); Interface (revised public API for `CredentialContextExt`, `ActionResult` gating, `*Handler` HRTB shape, `SignaturePolicy::Custom`); Open items / accepted gaps (**explicit gap: CP6 vocabulary deferred to future credential cascade — pointer required**); Migration / handoff (codemod + plugin migration notes).
  - **Spike requirements: 0 confirmed.** All shapes are bug-fixes or known-correct alternatives.
  - Estimated drafting: ~3 checkpoints at architect cadence (§1-3 → §4-6 → §7-9). Spans ~1-1.5d of architect-time absorbed inside the 4-5d total.

### Option B'+ — Action-scoped + structural CP6 prep (recommended hybrid)

This is a refinement of B' that addresses tech-lead's "permanent bridge layer" concern by laying the architectural groundwork for CP6 in this cascade without requiring credential implementation now. Distinct option; lives between B' and A'.

- **Scope summary (what's IN):**
  - Everything in B' (CR2-CR11 except CR1 and CR7-spec-shape).
  - **Plus structural prep:**
    - Introduce `nebula-action::credential` module with traits/types that **mirror the CP6 surface contract** (`CredentialRef<C>` as a type alias / phantom wrapper, `SlotBinding` skeleton, `SchemeGuard<'a, C>` RAII placeholder) but with implementations that delegate to the current `CredentialContextExt` typed-key access. Action ships with the **API shape** of CP6, fed by current credential internals.
    - When credential cascade lands CP6 vocabulary, action's traits/types switch their internal implementation; the **public API authored by plugin authors does not break.**
    - This is NOT a shim or bridge layer — it is the public contract; the credential crate's internals get swapped in beneath without action's API changing. Per tech-lead §3 — bridge layers in engine are forbidden; this is a deliberate API-shape commitment in action.
  - Add the 3 cluster-mode hooks (`IdempotencyKey` trait method, `on_leader_*` default-no-op lifecycle hooks, `dedup_window` metadata field) per tech-lead §4 — engine cluster coordination remains out of scope, but action exposes the contract surface.
- **Deferred items (what's OUT):**
  - Same as B' but the deferred work is materially smaller in scope: credential crate now needs only to implement vocabulary internals to feed the action-side public surface. The user-facing action API is stable.
  - DataTag, `Provide` port — same as B'.
- **Cost estimate:** **6-8 agent-days.** Rationale:
  - Everything in B' (4-5d).
  - Plus structural-prep types in nebula-action (CredentialRef alias, SlotBinding skeleton, SchemeGuard delegating wrapper): 1.5-2d.
  - Plus 3 cluster-mode hooks on `TriggerAction`: 0.5d.
  - **Slightly exceeds 5d autonomous budget.** May need 1-2d extension authorization, far less than A's 13-17d shortfall.
- **Blast radius:**
  - Same crate-touch list as B' but adds nebula-credential public surface (CredentialRef, SlotBinding etc. can live in nebula-action's `credential` module first, with intent to migrate to nebula-credential when CP6 lands; or land in nebula-credential as the final home from day one — architect-leaning preference is "land in credential to avoid future re-home").
  - Net: 4-5 crates touched; ~30-35 source files.
- **Critical findings addressed:** CR1 ✅ (vocabulary surface lands, even if internals delegate), CR2-CR11 same as B'. **CR7 ✅** because the spec API methods (`ctx.credential::<C>(key)`) land with the documented signatures even if internal implementation is current-engine.
- **Risks:**
  - **Surface-vs-implementation gap:** plugin authors see CP6 API; engine internals are pre-CP6. The contract held only if credential cascade actually lands and swaps internals before any constraint forces a divergence between contract and implementation. If credential cascade slips beyond a quarter, the divergence becomes the architecture (`feedback_incomplete_work` trap).
  - **Risk of committing to a wrong shape:** if credential implementation reveals that CP6 §7.1 `SlotBinding` HRTB shape doesn't compile cleanly (rust-senior §5 says it should, but no spike has run), action's pre-committed surface needs revision. This is a 0.5d risk per shape-defect-discovered.
  - Plugin migration cost identical to B' (one cut, manageable blast radius).
  - **The "we're committing to the spec without implementing it" stance is the optimistic path.** It depends on credential cascade landing within reasonable timeframe; otherwise B'+ degrades to B' silently.
- **Migration story:**
  - Identical to B' in plugin-author experience.
  - When credential cascade later lands CP6 internals, plugins do **not** re-migrate (their declared `CredentialRef<C>` etc. continue to work; behavior tightens to phantom-safety enforcement).
- **Tech Spec writing effort:**
  - Sections: same as B' but Interface section is larger (full CP6 public API surface). Storage schema covers the future-CP6 internal layout target as well as the present-day delegation.
  - **Spike requirements: 1 spike — confirm `CredentialRef<C>` + `SlotBinding` skeleton compiles + LSP-resolves correctly with delegating internals.** Spike fits inside the 6-8d total.
  - Estimated drafting: ~3-4 checkpoints. Spans ~2d of architect-time absorbed inside total.

### Option C' — Escalate credential spec revision (CP6 → CP6.1)

- **Scope summary (what's IN):**
  - Request credential Tech Spec CP6 §§2.7/3.4/7.1/15.7 revision: defer phantom rewriting + `SlotBinding` HRTB + `SchemeGuard` RAII to a future CP7.
  - CP6.1 ships with: stable `AuthScheme`; explicit-key typed access (`CredentialRef<C>` becomes a typed wrapper without phantom rewriting); Drop+zeroize on credential guard (similar to current `CredentialGuard<S>` with explicit-key constructor). Phantom-safety + RAII are CP7 work.
  - Action redesign adopts the **CP6.1 vocabulary** — reduced surface that maps 1:1 to what credential ships in CP6.1.
  - Everything else from B' (macro fixes, depth cap, HRTB modernization, ControlAction seal, Terminate gating + wiring, S-W2 mitigation, workspace hygiene).
- **Deferred items (what's OUT):**
  - Phantom-rewriting attribute macro (CP7).
  - `SlotBinding` HRTB fn-pointer registry (CP7).
  - `SchemeGuard<'a, C>` lifetime-tied RAII (CP7) — current `CredentialGuard<S>` Drop+zeroize remains.
  - Filed sub-spec pointer: "credential CP7 — phantom-safety + RAII tightening" — a real cascade slot, not a TODO. Per `feedback_active_dev_mode`, must have a home.
- **Cost estimate:** **6-9 agent-days** (variable).
  - Spec amendment cycle (CP6 → CP6.1 freeze update): 1-2d. Triggers escalation rule 10 — needs user authorization. Architect drafts revision, tech-lead + security-lead review, user ratifies. Cost depends on how many co-decision rounds the amendment requires.
  - Action implementation against CP6.1: ~5-6d.
  - Net: B' work + 1-2d for the spec revision cycle + ~1-2d for the simpler typed-wrapper implementation that exceeds B's CR3 deprecation.
- **Blast radius:**
  - Same as B'+ but the deferred CP7 work is more honest (a documented future spec, not an "internal swap" optimism).
  - Crates touched: same set as B'+.
- **Critical findings addressed:** CR1 partial (CP6.1 vocabulary lands; phantom + SlotBinding deferred — partial spec adherence by definition), CR2-CR11 same as B'. **CR7 ✅** with CP6.1 semantics (less safe than CP6 but explicit and documented).
- **Risks:**
  - **Escalation rule 10:** unfreezes a recently-frozen spec (CP6 froze across commits 65443cdb / 33eb3f01 / 883ccfbf in last 2 weeks). User authorization required.
  - **CP7 follow-up rot:** same trap as B' — only valid if CP7 has a scheduled cascade slot.
  - **Lower-quality long-term outcome:** phantom-safety and RAII are documented-as-deferred rather than landed. Per `feedback_active_dev_mode` "prefer more-ideal over more-expedient," C' chooses expedient.
  - Counter-argument: spec revision is an honest engineering response when implementation reveals constraints — it's not a workaround, it's a re-decision. Per `feedback_adr_revisable`, frozen specs are revisable when context shifts.
- **Migration story:**
  - Plugin authors see one cut, similar to B'. CP6.1 vocabulary is publicly stable. When CP7 lands, phantom-safety + RAII tightening may require a follow-up plugin migration depending on how CP7 chooses to retrofit.
- **Tech Spec writing effort:**
  - Spec amendment doc (CP6 → CP6.1 delta): ~0.5-1d of architect time on top of cascade Tech Spec.
  - Action Tech Spec: similar to B' but Interface section reflects CP6.1 surface.
  - **Spike requirements: 1 spike confirmed — typed-key explicit access pattern under cancellation drop-order test.** Smaller than A's spike.
  - Estimated drafting: ~3-4 checkpoints. Spans ~2-2.5d of architect-time.

---

## 2. Option comparison matrix

| Option | Cost (agent-days) | Blast radius (crates × files) | 🔴 addressed | Risk level | Escalation probability | Budget fit |
|---|---|---|---|---|---|---|
| **A'** Co-landed cascade | **18-22d** | 9-10 × ~80-100 | 11/11 | High (coordination + plugin disruption) | **High — exceeds 5d budget by 3.5-4×** | ❌ requires user budget extension |
| **B'** Action-scoped only | **4-5d** | 3-4 × ~25-30 | 8.5/11 (CR1 + CR7-shape deferred) | Medium (two-vocab window risk; bridge erosion risk) | **Low — fits budget** | ✅ if user accepts B' as honest scope |
| **B'+** Action-scoped + CP6 surface prep | **6-8d** | 4-5 × ~30-35 | 11/11 (surface) / 8.5/11 (semantics) | Medium-low (honest contract; surface-vs-impl divergence risk) | **Low — 1-2d budget extension** | ✅ small extension |
| **C'** Escalate spec revision | **6-9d** | 4-5 × ~30-35 | 9.5/11 (CP6.1 spec semantics) | Medium (CP7 follow-up rot; spec unfreeze precedent) | **High — escalation rule 10 trigger** | requires user spec-revision authorization |

Notes on the scoring:
- **🔴 addressed** — half-credit when a finding is structurally addressed but not by the spec's letter (e.g., B'+ exposes the CP6 API but its semantics are CP6.1-grade until credential cascade lands). Tech-lead/security-lead may weight surface-vs-semantics differently.
- **Risk level** — qualitative; reflects boundary-erosion exposure (`feedback_boundary_erosion`), follow-up rot exposure (`feedback_active_dev_mode`), and breaking-change disruption.
- **Escalation probability** — probability of triggering the user-authorization escalation envelope. A' triggers via budget; C' triggers via spec unfreeze; B' / B'+ are inside autonomous authority.

---

## 3. Trade-off lines (where options genuinely differ)

These are the load-bearing decisions; tech-lead applies priority weighting and security-lead applies VETO check against each.

### 3.1 If you want CR3 cross-plugin shadow attack **structurally eliminated** (no key heuristic exists at all in the surface, even at runtime), only **A'** and **B'+** qualify. **B'** and **C'** rely on key explicitness as a discipline (mandatory-key on `credential_typed::<S>(key)`); a future drift could re-introduce a heuristic. **Security-lead VETO check.** All four options at minimum deprecate the type-name heuristic, so all four pass security-lead's CR3-must-be-fixed VETO. The differentiator is whether the heuristic class is impossible-by-construction (A'/B'+ via phantom typing of CredentialRef<C>) or impossible-by-discipline (B'/C').

### 3.2 If you want CR4 JSON depth bomb fixed, **all four options qualify.** Depth cap is a small mechanical change (~0.5d) at adapter boundaries; it lives in every option.

### 3.3 If you want plugin authors to stop seeing **unusable `credential = Type` syntax today**, all four qualify (CR2 fix is in every option). Differentiator: A' replaces the derive entirely; B'/B'+/C' keep the derive and fix its emission.

### 3.4 If you want **the action × credential coupling to be done correctly once and not re-touched** (per `feedback_hard_breaking_changes` — "expert-level not junior patches"), only **A'** qualifies. B', B'+, and C' all leave a future re-touch (B' has CP6 cascade slot; B'+ has internal-swap-when-credential-lands; C' has CP7).

### 3.5 If you want **the cascade to fit autonomous budget without escalation**, only **B'** qualifies. B'+ requires 1-2d extension (small, low-friction). A' requires 13-17d extension (large, almost certainly user-blocked). C' requires spec-revision authorization (process-blocked, possibly small day-count-blocked too).

### 3.6 If you want **`feedback_active_dev_mode` "prefer more-ideal over more-expedient" honored**, **A'** and **B'+** are most-ideal; **C'** is middle (honest deferral with a real CP7 home); **B'** is most-expedient (deferred-without-home is forbidden mindset; with-home is acceptable but lower-quality outcome).

### 3.7 If you want **`feedback_boundary_erosion` honored** (no permanent bridge layer in engine), **A'** is best, **B'+** is acceptable (the API-shape commitment is in action, not engine), **C'** is acceptable (CP6.1 spec defines the surface honestly), **B'** is the highest erosion-risk option (engine likely accumulates translation layer when credential CP6 lands later).

### 3.8 If you want **two-vocabulary window minimized**, **A'** wins (single landing); **B'+** is next (action's API is forward-compatible); **C'** is good (CP6.1 spec is internally consistent); **B'** is worst (action speaks pre-CP6 vocabulary indefinitely).

### 3.9 If you want **security-lead's S-W2 (`SignaturePolicy::Custom` audit defeat) fixed in this cascade**, all four options can include it (mechanical change ~0.5d). Recommend including in all options regardless of which is picked.

### 3.10 If you want **rust-senior's HRTB modernization landed cheaply**, all four options can include it (mechanical change ~0.5-1d). Recommend including in all.

### Summary of trade-off line implications

| Priority axis | A' | B' | B'+ | C' |
|---|---|---|---|---|
| Done-once-and-not-retouched | ✅ best | ❌ worst | 🟡 partial (depends on credential cascade) | 🟡 partial (CP7 promise) |
| Budget fit | ❌ +13-17d | ✅ fits | 🟡 +1-2d | ❌ +1-4d + escalation |
| Active-dev-mode lens | ✅ most ideal | ❌ expedient (only OK with home) | ✅ ideal-with-honest-bet | 🟡 honest middle |
| Boundary erosion risk | ✅ none | ❌ engine bridge incoming | ✅ none | ✅ none |
| Two-vocab window | ✅ none | ❌ multi-quarter | 🟡 surface-stable | 🟡 single rev |
| Plugin migration churn | one big cut | one moderate cut | one moderate cut | one moderate cut |
| Spec compliance | ✅ CP6 letter | ❌ pre-CP6 | 🟡 CP6 surface / CP6.1 semantics | 🟡 CP6.1 letter |

---

## 4. Open questions for tech-lead / security-lead / user

These are questions that legitimately need their input, not architect judgment.

1. **(user)** Does A's 18-22d cost estimate justify asking to extend the cascade autonomous budget? If so, by how much — full extension to ~20d, or partial extension with B'+ fallback if the spike (HRTB + SchemeGuard + cancellation drop) reveals constraints?
2. **(user, tech-lead)** Is C' (CP6 → CP6.1 unfreeze) acceptable given the credential Tech Spec CP6 freeze is ~2 weeks old? Recent commits (65443cdb, 33eb3f01, 883ccfbf) ratified Gate 1, Gate 2, and CP6 §15.4/§15.7 refinements — unfreezing now revisits very recent work.
3. **(tech-lead)** Does B'+ adequately address tech-lead's §3 "engine bridges = boundary erosion" concern? My read: yes, because the API-shape commitment is in action's public surface (not engine's runtime), and the swap-internals-when-credential-lands path keeps engine clean. Confirm or push back.
4. **(security-lead)** Does B' / C' deprecation+removal of `credential<S>()` no-key variant + mandatory explicit key on `credential_typed::<S>(key)` adequately structurally address CR3, or does VETO require A'/B'+ phantom typing? My read: VETO is satisfied by removal-of-heuristic-method; phantom typing is defense-in-depth. Confirm or VETO.
5. **(security-lead)** S-W2 (`SignaturePolicy::Custom` defeating `OptionalAcceptUnsigned` audit-trail design) — should the in-this-cascade fix replace `Custom(Arc<dyn Fn>)` with a closed enum of named verifier kinds, or attach attestation metadata (closure source-name / sha) to the `Custom` variant? Both are mechanically ~0.5d but the design differs.
6. **(tech-lead)** B'+ commits action's public API shape to CP6 vocabulary while landing pre-CP6 internals. If credential cascade slips and the surface-vs-implementation gap widens for >1 quarter, does that count as `feedback_incomplete_work` (forbidden mindset — "deferred to next wave") or as `feedback_active_dev_mode` "honest contract held longer than planned"?
7. **(user, orchestrator)** If A' is selected and exceeds budget, does the cascade pause for user authorization, or does the cascade narrow to A'-phased (Phase 3A/3B/3C from tech-lead §6) where 3A+3B fit budget and 3C lands in a follow-up cascade with explicit pre-commitment?

---

## 5. Architect draft position (NOT a decision)

**Architect lean: B'+ (Action-scoped + CP6 surface prep).**

This is not a decision — tech-lead picks; security-lead may VETO. But proposer must stake a position so reviewers know where I'd land if asked.

**Reasoning:**

1. **B'+ honors `feedback_hard_breaking_changes` and `feedback_active_dev_mode` simultaneously.** The plugin migration is a single cut to spec-correct surface; plugins do not re-migrate when credential cascade lands CP6 internals.
2. **B'+ honors `feedback_boundary_erosion`** because the API-shape commitment lives in nebula-action's public surface (where it semantically belongs — action authors declare credential dependencies), not in engine's runtime translation. The "internal swap" pattern is not a bridge layer; it is a deliberate forward-compatibility move sanctioned by the architect at design time.
3. **B'+ keeps the budget extension small** (1-2d vs A's 13-17d). Lower escalation probability; more likely to land cleanly.
4. **B'+ matches tech-lead's stated "lean A', fallback C', not B'" preference** by NOT being B' — it is closer to A' in spec-compliance shape than to B' in coupling assumptions.
5. **B'+ covers both security-lead VETO triggers** structurally (CR3 via phantom typing of CredentialRef<C>, CR4 via depth cap).

**Where B'+ is weaker than A':** if the user is willing to extend to ~20 agent-days, A' is the more durable outcome. The B'+ surface-vs-implementation gap is honest engineering but it's a window. A' closes the window now.

**Where B'+ is weaker than C':** if the user prefers a single internally-consistent spec revision over an optimistic surface bet, C' is more honest about what's deferred. B'+ requires faith that credential cascade lands CP6 internals on a reasonable timeline.

**My request to tech-lead and security-lead:**

- Tech-lead to apply priority weighting on §3 trade-off lines and pick an option (A' / B' / B'+ / C'). I will draft Strategy Document under whichever is selected.
- Security-lead to confirm CR3/CR4 VETO satisfaction across all four options (or VETO specific ones).
- If A' selected and budget extension authorized, architect drafts Strategy Document + 1 spike (HRTB + SchemeGuard + cancellation drop).
- If B'+ selected, architect drafts Strategy Document + 1 spike (CredentialRef<C> + SlotBinding skeleton compile + LSP-resolve check).
- If C' selected, architect drafts Strategy Document + spec amendment doc (CP6 → CP6.1) for credential Tech Spec, plus 1 spike (typed-key cancellation drop).
- If B' selected, architect drafts Strategy Document + records "credential CP6 cascade slot" as a follow-up that must have a committed home before Phase 2 closes (or the option degrades to forbidden quick-win-trap).

**Architect stands ready to draft Strategy Document under whichever option is selected.**

---

*End of Phase 2 architect proposer position. Routing to orchestrator for tech-lead pick + security-lead VETO check.*
