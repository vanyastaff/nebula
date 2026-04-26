---
status: FROZEN CP3 2026-04-24
cascade: nebula-action redesign
owner: architect (drafting); tech-lead (solo decider at CP gates); security-lead (VETO authority on scope-critical findings); orchestrator (coordination)
created: 2026-04-24
target freeze: CP3 (estimated within 3-5 cascade days from creation)
related:
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/01-current-state.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md
  - docs/superpowers/specs/2026-04-24-credential-redesign-strategy.md
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md
  - docs/adr/0035-phantom-shim-capability-pattern.md
  - docs/PRODUCT_CANON.md
---

# Strategy — nebula-action redesign

## §0 Freeze policy

**Scope of this document.** Strategy-level decisions for the `nebula-action` redesign cascade. The chosen scope is **Option A' — co-landed action + credential CP6 design** (locked at Phase 2, see [`03-scope-decision.md`](../drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) §1). The Strategy frames *why this and not the alternatives*; the Tech Spec (Phase 6) translates the chosen direction into implementation-ready signatures.

**Status progression.** This document moves through four states:

| State | Sections frozen | Mutation mechanism |
|---|---|---|
| **DRAFT CP1** | §0–§3 only | Re-draft sections in place; track diffs in CHANGELOG below |
| **DRAFT CP2** (this revision) | §0–§5 (CP2 adds §4 recommendation + §5 open items + spike plan) | Same |
| **DRAFT CP3** | §0–§6 (CP3 adds §6 post-validation roadmap) | Same |
| **FROZEN CP3** | All sections | New ADR with inline forward-pointer at the amended paragraph |

**What changes invalidate the freeze.** Once frozen at CP3, only an ADR may amend §1–§6. Drafts may be re-revised in CP1/CP2/CP3 windows without ADR.

**Amendment vs supersession.** Amendments (typo, citation correction, follow-up clarification post-spike) land **inline** with an "*Amended by ADR-NNNN, YYYY-MM-DD*" prefix at the changed paragraph and do not retract the surrounding decision. Supersession applies when the underlying decision is reversed; the superseding ADR is referenced and the original paragraph is preserved with a "*Superseded by ADR-NNNN*" prefix. Pattern follows the credential Strategy precedent ([`2026-04-24-credential-redesign-strategy.md`](2026-04-24-credential-redesign-strategy.md) §0).

**Authority chain.** PRODUCT_CANON > ADRs (0028-0035 + the action-redesign ADRs that this cascade ratifies) > Strategy (this document) > Tech Spec (Phase 6) > implementation plans. Strategy directs the Tech Spec; Tech Spec is implementation-normative.

**Reading order.** §0 (this) → §1 (problem) → §2 (constraints) → §3 (options analysis with explicit pick) → §4 (recommendation, CP2) → §5 (open items + spike plan, CP2) → §6 (post-validation roadmap, CP3).

## §1 Problem statement

`nebula-action` is structurally sound at runtime — cancel safety, error taxonomy, dispatch semantics, and webhook crypto primitives are reference-quality (Phase 0 [`01-current-state.md`](../drafts/2026-04-24-nebula-action-redesign/01-current-state.md) §1, §3.S1, line 117 of [`PRODUCT_CANON.md`](../../PRODUCT_CANON.md) §4.2). The crate's **integration surface** — credential resolution path, macro emission contract, canon §3.5 trait family, and security-relevant adapter boundaries — is not. Phase 0 surfaced 4 🔴 critical findings; Phase 1 ([`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md)) escalated to 11 deduplicated 🔴 findings + 30+ 🟠 — well past the redesign-justified threshold.

Four distinct drift patterns converge:

**(a) Credential integration paradigm mismatch (CR1, CR5–CR10).** The 2026-04-24 credential Tech Spec (now at CP6, see [`2026-04-24-credential-tech-spec.md`](2026-04-24-credential-tech-spec.md)) specifies an API paradigm — `CredentialRef<C>` typed handle, `AnyCredential` object-safe supertrait, `SlotBinding` with HRTB `for<'ctx> fn(...) -> BoxFuture<'ctx, _>` `resolve_fn`, `SchemeGuard<'a, C>` RAII, `SchemeFactory<C>` re-acquisition arc, `RefreshDispatcher::refresh_fn` HRTB — that is **structurally absent** from `nebula-action`. Even more critically, tech-lead's grep ([`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md) §3 critical reframe) confirmed the same vocabulary is **also absent from `crates/credential/src/`** — `CredentialRef`, `SchemeGuard`, `SlotBinding`, `SchemeFactory` have zero matches. Action is not lagging a shipped credential API; both crates are out-of-sync with a still-unimplemented spec. dx-tester evidence: time-to-first-successful credential-bearing action is 32 minutes against a target <5; `#[action(credential = "key")]` string form silently drops to 0 deps; `#[action(credential = Type)]` typed form requires `CredentialLike` with zero workspace implementors; v2 spec's `ctx.credential::<S>(key)` / `credential_opt::<S>(key)` ActionContext API surface does not exist (3 alternative methods, none match). [Open item — CP2 to resolve: credential Tech Spec §2.7 covers `#[action]` macro rewriting, NOT the `ActionContext::credential` API contract; CP2 §4 must pin the actual ActionContext API location in the credential Tech Spec, see §5 open items.]

**(b) Macro emission bugs masked by missing test harness (CR2, CR8, CR9, CR11).** The 359-LOC `#[derive(Action)]` proc-macro has zero `trybuild` / `macrotest` coverage. Three independent agents (dx-tester, security-lead, rust-senior) each hit the same bug: `parameters = Type` emits `.with_parameters(<Type>::parameters())` against a method that does not exist on `ActionMetadata` ([`01-current-state.md`](../drafts/2026-04-24-nebula-action-redesign/01-current-state.md) §2 finding C2; `crates/action/macros/src/action_attrs.rs:129-134`). Adjacent macro defects: unqualified `::semver::Version` emission requires user to add an undocumented `semver` dep; `Input: HasSchema` bound undocumented; macro emits nothing for ports despite README promise. Root cause is structural: `crates/action/macros/Cargo.toml` has no `[dev-dependencies]` section.

**(c) Canon §3.5 governance drift via ControlAction + DX tier (S1, S2 from Phase 0; tech-lead §1 from Phase 1).** [`PRODUCT_CANON.md`](../../PRODUCT_CANON.md) §3.5 line 82 enumerates four action traits and requires explicit canon revision (§0.2, line 27) to add another. `crates/action/src/lib.rs:13-20` re-exports 10 trait surfaces, including `ControlAction` (public, non-sealed, fifth dispatch-time trait) plus 5 DX specialization traits (`PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction`, `ControlAction`). Engine dispatch is preserved as a clean 4-variant `ActionHandler` enum (verified by tech-lead grep), so this is documentation drift, not structural drift — but unfixed it is a literal canon-revision event by §0.2 wording. The library docstring at `crates/action/src/lib.rs:11` self-contradicts: it re-states the canon-revision rule while re-exporting 10 traits.

**(d) Exploitable security surfaces (CR3 / S-C2 cross-plugin shadow attack; CR4 / S-J1 JSON depth bomb).** S-C2: `CredentialContextExt::credential<S>()` ([`01-current-state.md`](../drafts/2026-04-24-nebula-action-redesign/01-current-state.md) §2 finding C3; `crates/action/src/context.rs:637-643`) computes the lookup key as `type_name::<S>().rsplit("::").next().unwrap_or(type_name).to_lowercase()`. Two plugins each registering `struct OAuthToken` collide silently — plugin B can resolve plugin A's token via type-name shadowing. **Exploitable today, not hypothetical.** S-J1: `StatelessActionAdapter::execute` (`crates/action/src/stateless.rs:356-383`) calls `serde_json::from_value` with no depth cap — attacker-supplied deeply-nested JSON via workflow input stack-overflows the worker. v2 design spec post-conference amendment B3 mandates 128-depth recursion limits at adapter boundaries; zero matches for `set_max_nesting` / `recursion_limit` exist in `crates/action/src/`.

**Plugin-author experience today.** dx-tester authored three action types in a scratch worktree ([`02a-dx-authoring-report.md`](../drafts/2026-04-24-nebula-action-redesign/02a-dx-authoring-report.md)): time-to-first-compile is 12 / 8 / 32 minutes (Stateless / Stateful / ResourceAction+Credential) against a target of <5. Eleven 🔴 authoring blockers surfaced. Choice paralysis at credential access (3 method variants, none matching the spec pair) compounds the friction.

**Cross-crate impact if nothing is done.** The deadlock is symmetric. Credential CP6 spec (frozen since 2026-04-24, [`2026-04-24-credential-redesign-strategy.md`](2026-04-24-credential-redesign-strategy.md) §6 Checkpoint 3) waits for a spec-compliant consumer to materialize — no consumer materializes because action's redesign waits for credential. Each crate's Tech Spec describes shapes the other expects to consume; neither lands without coordination.

**Why action-only fixes (Option B') is rejected.** B' would land in cascade budget (~4-5 agent-days) and clear CR2/CR4/CR5/CR8/CR9 (macro + JSON depth) plus deprecate CR3, but leaves the credential idiom permanently superseded. A bridge layer in the engine emulating phantom-safety at runtime is required — direct `feedback_boundary_erosion` violation per tech-lead round 2 analysis ([`03b-tech-lead-priority-call.md`](../drafts/2026-04-24-nebula-action-redesign/03b-tech-lead-priority-call.md)) — and plugin authors re-migrate when CP6 lands later, paying breaking-change cost twice. Two-vocabulary debt is permanent until the credential cascade lands; B' is local-optimal, not global-optimal.

**Why spec revision (Option C') is rejected.** C' would request credential Tech Spec CP6 §§2.7 / 3.4 / 7.1 / 15.7 revision to permit derive-based implementation (no attribute macro, no silent field-type rewriting, explicit user-visible types). The spec-reality gap is **implementation lag, not spec defect** — iter-1 / iter-2 / iter-3 spike validation ([`2026-04-24-credential-redesign-strategy.md`](2026-04-24-credential-redesign-strategy.md) §6.1) confirmed the chosen shape compiles, satisfies dyn-safety via ADR-0035 phantom-shim, and clears all 5 spike scope questions with no fallback triggered. Unfreezing a 2-week-old frozen spec that CP6 Gate 1/2/3 ratified — to accommodate a derive-only macro that cannot perform field-type rewriting — is disproportionate. Per `feedback_adr_revisable.md`, ADRs are point-in-time, but spec revision requires justification beyond "implementation has not started yet"; the spec was validated, not authored speculatively. C' was an option of last resort if A' implementation proved unworkable; A' implementation path is workable.

## §2 Constraints

Authoritative invariants any Strategy direction must honor. Numbered for cross-reference from §3 / §4.

**§2.1 — PRODUCT_CANON §3.5 trait family (line 82, [`PRODUCT_CANON.md`](../../PRODUCT_CANON.md)).** Canon enumerates four action traits (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`); adding another requires canon revision per §0.2 (line 27). A' triggers a §3.5 DX tier ratification — `ControlAction` sealed and DX traits canonized as "erases to primary" — and a §0.2 revision cycle to re-state the trait family enumeration. Tech-lead solo-decided this call at Phase 1 ([`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md) §7).

**§2.2 — PRODUCT_CANON §0.2 revision triggers (line 27).** "Capability lag" + "uncovered case" both fire on the current state: canon §3.5 freezes a 4-trait enumeration that predates the DX tier; `ControlAction` is an uncovered case the canon is silent on. A' lands the canon revision; B' would let the drift persist; C' is orthogonal. Strategy must produce the canon revision PR alongside the action redesign PR, not after.

**§2.3 — PRODUCT_CANON §4.5 false-capability rule (line 131).** "Public surface exists iff the engine honors it end-to-end." `ActionResult::Terminate` (`crates/action/src/result.rs:207-219`) is a public variant with documented "Phase 3 of the ControlAction plan and is not yet wired." This is a literal §4.5 violation today. Tech-lead solo-decided ([`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md) §7): feature-gate **AND** wire `Terminate` in cascade, not gate-only-and-defer (`feedback_active_dev_mode.md`). A' must apply the same discipline `ActionResult::Retry` already meets ([`PRODUCT_CANON.md`](../../PRODUCT_CANON.md) §11.2 line 286-298): the `unstable-retry-scheduler` feature flag pattern.

**§2.4 — PRODUCT_CANON §11.2 retry canon (line 286-298).** Engine-level node re-execution from `ActionResult::Retry` is `planned`; the canonical retry surface today is the `nebula-resilience` pipeline composed inside an action body. The `Retry` variant is feature-gated behind `unstable-retry-scheduler`; the gate must come down only when scheduler ships end-to-end. A' must not introduce a parallel retry surface or relax the gate; the redesign reaffirms the canon.

**§2.5 — PRODUCT_CANON §11.3 idempotency (line 300).** Idempotency at the action boundary is a §11.3 contract — deterministic per-attempt key, persisted in `idempotency_keys`, checked through `ExecutionRepo` before the side effect. A' must not introduce action-side idempotency surfaces that bypass `ExecutionRepo`. Cluster-mode hooks (per [`03-scope-decision.md`](../drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) §1.7: `IdempotencyKey`, `on_leader_*` lifecycle, `dedup_window` metadata on `TriggerAction`) are surface contract only; engine cluster coordination is out of scope per §3.4 OUT markers.

**§2.6 — PRODUCT_CANON §12.5 secrets and auth (line 386-391).** No secrets in logs / error strings / metrics labels. `Zeroize` / `ZeroizeOnDrop` on key material; redacted `Debug` on credential wrappers. Every new `tracing::*!` that takes a credential or token argument must use redacted forms. A' must route `ActionError` Display through a `redacted_display()` helper in `tracing::error!` paths (security must-have §3 of [`03-scope-decision.md`](../drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) §3) and ensure `SchemeGuard<'a, C>` zeroize-on-drop semantics are honored across the cancellation boundary (closes S-C5).

**§2.7 — PRODUCT_CANON §12.6 isolation honesty (line 393-397).** In-process sandbox is correctness-and-least-privilege, not a security boundary against malicious native code. `IsolationLevel::CapabilityGated` is acknowledged as a documented-false-capability ([`02b-security-threat-model.md`](../drafts/2026-04-24-nebula-action-redesign/02b-security-threat-model.md), 03c S-I2). A' must not strengthen `CapabilityGated` claims in user-visible documentation; sandbox phase-1 cascade is the sub-spec home (per [`03-scope-decision.md`](../drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) §3 deferred row).

**§2.8 — Credential Tech Spec CP6 shapes (authoritative).** A' adopts the following shapes verbatim from [`2026-04-24-credential-tech-spec.md`](2026-04-24-credential-tech-spec.md):

- **§2.7 line 486-528** — `#[action]` macro translation: `CredentialRef<dyn ServiceCapability>` in user syntax → `CredentialRef<dyn ServiceCapabilityPhantom>` in generated code (silent rewrite per Tech Spec §2.7 decision). Pattern 1 concrete `CredentialRef<ConcreteCredential>` pass-through unchanged.
- **§3.4 line 807-939** — Pattern 2 dispatch narrative: declaration-site phantom check + resolve-site `where C: Credential<Scheme = X>` enforcement; engine performs type reflection + `downcast_ref`; action body sees `&Scheme` directly, never `&dyn Phantom`.
- **§7.1 (refresh dispatcher)** — `RefreshDispatcher::refresh_fn` HRTB `for<'ctx> fn(...) -> BoxFuture<'ctx, _>` pattern; A' adopts the **same HRTB function-pointer pattern** for `SlotBinding::resolve_fn` per Tech Spec §3.4 line 869.
- **§15.7 line 3383-3517** — `SchemeGuard<'a, C>` RAII: `!Clone`, `ZeroizeOnDrop`, `Deref<Target = C::Scheme>`, lifetime parameter prevents storage in struct fields outliving the call. `SchemeFactory<C>` for re-acquisition by long-lived resources. Includes the spike iter-3 lifetime-gap refinement (line 3503-3516) — engine passes `SchemeGuard<'a, C>` alongside `&'a CredentialContext<'a>` sharing `'a` to prevent retention via `'static` inference.

**§2.9 — ADR-0035 phantom-shim canonical form ([`docs/adr/0035-phantom-shim-capability-pattern.md`](../../adr/0035-phantom-shim-capability-pattern.md)).** A' design must compose with ADR-0035's two-trait phantom-shim pattern (line 65-108): per-capability inner sealed traits in `mod sealed_caps`; "real" capability trait supertrait-chained for compile-time constraint; "phantom" capability trait `dyn`-safe for `dyn` positions. Pattern 4 (lifecycle sub-trait erasure, ADR-0035 §2 line 124-135 amendment 2026-04-24-C) extends the convention to `dyn RefreshablePhantom` / `InteractivePhantom` / `RevocablePhantom` / `TestablePhantom` / `DynamicPhantom` — relevant for any engine-side runtime registry that iterates over action's credential slots by lifecycle capability. The `#[action]` macro's translation rule (Tech Spec §2.7) is one half of the action-side ADR-0035 obligation; the other half is verifying the per-crate `mod sealed_caps` declaration in any action crate that defines its own service capability traits.

**§2.10 — Workspace layer invariants (`deny.toml`, [`docs/PRODUCT_CANON.md`](../../PRODUCT_CANON.md) §12.1 line 358).** Layer rules are positive in `deny.toml` for engine / storage / sandbox / sdk; `nebula-action` lacks an explicit positive layer rule. A' must add the rule (T9 from Phase 0). No upward dependencies between layers; `crates/action` does not embed engine internals; engine's 27+ import sites of action are downward, which is correct — but the shape stays one-way, and any helper that "wants to live in action because the engine is its only caller" must be evaluated as a boundary decision per `feedback_boundary_erosion.md`, not convenience.

**§2.11 — Feedback memory (load-bearing for option ranking).**

- `feedback_hard_breaking_changes.md` — license to break plugin surface in one cut. A' deletes `CredentialContextExt::credential<S>()`, replaces `#[derive(Action)]` with `#[action]`, reshuffles `nebula-sdk::prelude` 40+ re-exports, and migrates 7 reverse-deps in lockstep. This is in-scope per the feedback rule; semver-checks are advisory-only during alpha ([`01-current-state.md`](../drafts/2026-04-24-nebula-action-redesign/01-current-state.md) §8 line 222).
- `feedback_boundary_erosion.md` — no engine bridge layers. A' wins; B' loses. The bridge that B' would require to emulate phantom-safety at runtime is the canonical case the rule was written to prevent.
- `feedback_active_dev_mode.md` — more-ideal over more-expedient; no gate-only-and-defer. Applies to `ActionResult::Terminate` (gate **and** wire), to `ControlAction` sealing (don't half-seal), and to the credential CP6 vocabulary (adopt the spec, don't bridge around it). The 3-engineering-gate path the credential Tech Spec uses (§15.12, line 3621+) is the model: engineering-derived sequencing, not consumer-derived deferral.
- `feedback_adr_revisable.md` — ADRs are point-in-time; superseding requires justification. A' triggers an ADR-supersede on canon §3.5 (DX tier ratification) and likely a new ADR for the action-redesign-specific decisions (e.g., `#[action]` attribute macro vs derive). C' would supersede credential CP6 spec — the bar is "spec defect" not "implementation lag"; A' avoids the bar.
- `feedback_no_shims.md` — never propose adapters/bridges/shims that keep wrong behavior working. CR3 fix (cross-plugin shadow attack) **must be hard removal** of `CredentialContextExt::credential<S>()` no-key heuristic, not `#[deprecated]` keeping the heuristic compileable. Security-lead 03c §1 retains implementation-time VETO authority on this point.

**§2.12 — Security must-have floor (non-negotiable invariant, from [`03-scope-decision.md`](../drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) §3).** This floor is **invariant, not goal**; in cascade scope regardless of all other decisions; Phase 6 Tech Spec must cite each item as not-deferrable:

1. **CR4 / S-J1 JSON depth bomb** — depth cap (128) at every adapter JSON boundary (`StatelessActionAdapter::execute`, `StatefulActionAdapter::execute`, API webhook body deserialization). Mandated by v2 design spec post-conference amendment B3.
2. **CR3 / S-C2 cross-plugin shadow attack** — replace type-name heuristic with explicit keyed dispatch at method-signature level. **Hard removal**, not `#[deprecated]` shim (`feedback_no_shims.md`; security-lead 03c §1 VETO on shim form). A' removes the method class entirely.
3. **`ActionError` Display sanitization** — route through `redacted_display()` helper in `tracing::error!` call sites to preempt the S-C3 / S-O4 leak class (per §2.6).
4. **Cancellation-zeroize test** — closes S-C5; pure test addition, no architectural cost.

Deferred security findings (🟠, acceptable with sunset commit): S-W2 `SignaturePolicy::Custom(Arc<dyn Fn>)` (webhook hardening cascade, 2 release cycles); S-C4 detached spawn zeroize defeat (credential CP6 landing); S-O1/S-O2/S-O3 output pipeline caps (output-pipeline cascade); S-I2 `CapabilityGated` (sandbox phase-1 cascade); S-W1/S-W3/S-F1/S-I1/S-U1/S-C1 minor defense-in-depth (cascade exit notes).

## §3 Options analysis

The scope decision was locked at Phase 2 ([`03-scope-decision.md`](../drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) §1) — Option A'. This section frames **why** A' won against three serious alternatives. Each option was the best argument someone made for it; understanding why the rejected ones lost matters for auditability and for the post-cascade revisit clause (§6, CP3).

### §3.1 Option A' — Co-landed cascade (chosen direction)

**Shape.** Tech Spec covers design for both crates' CP6 vocabulary in one document, plus engine wiring + plugin migration + workspace hygiene. Implementation is post-cascade, user-decided across (a) single coordinated PR, (b) sibling cascades, or (c) phased rollout with intermediate B'+ surface commitment.

**Components.**

1. **Credential crate CP6 vocabulary implementation design** — `CredentialRef<C>` typed handle, `AnyCredential` (already partially landed at `crates/credential/src/contract/any.rs` per tech-lead round 2), `SlotBinding` with HRTB `for<'ctx> fn(...) -> BoxFuture<'ctx, _>` `resolve_fn` (per credential Tech Spec §3.4 line 869), `SchemeGuard<'a, C>` RAII (`!Clone`, `ZeroizeOnDrop`, `Deref`, per Tech Spec §15.7 line 3394-3429), `SchemeFactory<C>` re-acquisition arc (Tech Spec §15.7 line 3432-3448), `RefreshDispatcher::refresh_fn` HRTB (Tech Spec §7.1).
2. **Action crate CP6 adoption design** — new `#[action]` attribute macro (replacing `#[derive(Action)]`) with **narrow declarative rewriting contract** (rewriting confined to `credentials(slot: Type)` / `resources(slot: Type)` attribute-tagged zones, not arbitrary field rewriting — per tech-lead §2 architectural coherence constraint). `CredentialRef<C>` field support; `ActionSlots` impl emission; new `ActionContext` methods matching CP6 spec (`ctx.credential::<S>(key)` / `credential_opt::<S>(key)` per Tech Spec §2.7).
3. **Engine wiring design** — `resolve_as_<capability><C>` helpers, slot binding registration at registry time, HRTB fn-pointer dispatch at runtime, depth-cap (128) at all adapter JSON boundaries (must-have §2.12).
4. **Security hardening (must-have floor §2.12 — non-negotiable):** JSON depth cap, S-C2 method-signature surgery, `ActionError` Display sanitization, cancellation-zeroize test.
5. **Phase 1 tech-lead solo-decided calls (ratified):** Seal `ControlAction` + canonize DX tier in canon §3.5 as "erases to primary"; feature-gate AND wire `ActionResult::Terminate`; modernize `*Handler` HRTB boilerplate to single-`'a` + type alias.
6. **Plugin ecosystem migration design** — codemod script for `#[derive(Action)]` → `#[action]`; migration guide for 7 reverse-deps; `nebula-sdk::prelude` re-export block reshuffle.
7. **Cluster-mode hooks design** — 3 hooks on `TriggerAction` (`IdempotencyKey`, `on_leader_*` lifecycle, `dedup_window` metadata) per tech-lead §4. Surface contract only; engine cluster coordination implementation out of scope (§3.4).
8. **Workspace hygiene** — `zeroize` workspace=true pin (T4); `unstable-retry-scheduler` feature retired or wired (depends on `Terminate` scheduler design, per §2.4); `deny.toml` layer-enforcement rule for `nebula-action` (T9, per §2.10); lefthook pre-push parity with CI required jobs (T5, per `feedback_lefthook_mirrors_ci.md`).

**Trade-off.** A' eliminates the two-vocabulary debt that B' would freeze in. It requires the Tech Spec to span two crates plus engine wiring, which is broader than a single-crate cascade — but the cascade scope is **DESIGN closure only** (per [`03-scope-decision.md`](../drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) §0 reframe), not implementation. Tech Spec writing effort is ~2-4 agent-days, within the 5-day cascade budget. Implementation cost (~18-22 agent-days for full impl) is post-cascade, user-gated, and explicitly NOT in cascade scope per [`03-scope-decision.md`](../drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) §6.

**Who it affects.** Plugin authors get one re-migration (not two); reverse-deps (engine, api, sandbox, sdk, plugin, cli + macros sibling) update once; credential CP6 spec gets its consumer; canon §3.5 gets its revision PR. Engine's 27+ import sites re-shape once.

**Why it wins.** It is the only option that closes both the action-side and credential-side gaps simultaneously, honors `feedback_boundary_erosion` and `feedback_active_dev_mode`, satisfies the security must-have floor without compromising on shim form, and triggers the canon §3.5 revision the drift requires (per §2.1 / §2.2).

### §3.2 Option B'+ — Hybrid (runner-up)

**Shape.** Action ships **CP6 API surface** (the user-visible types: `CredentialRef<C>`, new `#[action]` macro, new `ActionContext` methods) with **delegating internals** (the action's `resolve_as_<capability><C>` helper internally calls into the existing string-keyed `CredentialContextExt` until credential cascade lands CP6 internals). Plugin authors write CP6-shaped code on day one; credential cascade lands CP6 internals later in lockstep without forcing plugin re-migration.

**Trade-off.** B'+ is the graceful-degradation path if A' implementation proves unworkable in any specific dimension (credential CP6 internals + engine wiring + plugin migration concurrency). It avoids the two-vocabulary debt at the public surface (plugin authors see one shape) at the cost of a temporary internal bridge layer in action — that bridge is tightly bounded (action's `resolve_as_<capability><C>` thunk calling `CredentialContextExt`), is sunset-committed (deleted when credential CP6 internals land), and never appears in the engine. This is **scope-contained** boundary erosion versus B's **structural** boundary erosion (B's bridge lives in the engine permanently). Tech-lead 2nd-place ranking ([`03b-tech-lead-priority-call.md`](../drafts/2026-04-24-nebula-action-redesign/03b-tech-lead-priority-call.md) round 2) reflects this distinction.

**Who it affects.** Plugin authors get the CP6 shape immediately; reverse-deps see the new public surface; engine sees a CP6-shaped action surface even though credential internals haven't landed. The internal bridge is action's burden alone.

**Why it doesn't win.** A' fully closes the two-vocabulary problem; B'+ only papers it over in action's internals. If the cascade decides A' implementation is feasible, the temporary bridge is unnecessary. **B'+ stays available as a contingency** — if §4 recommendation (CP2) frames A' implementation path (a/b/c per [`03-scope-decision.md`](../drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) §6) and the user picks one that exposes scheduling friction (e.g., picks (a) single coordinated PR but cluster-mode hook design surfaces an unbudgeted constraint), the cascade can fall back to B'+ without re-running Phase 2. CP3 §6 (post-validation roadmap) records the contingency activation criteria.

**Conditions for B'+ activation (locked at CP1 per tech-lead round 2 [`03b-tech-lead-priority-call.md`](../drafts/2026-04-24-nebula-action-redesign/03b-tech-lead-priority-call.md) §"Does B'+ resolve your boundary-erosion concern?").** B'+ is only selectable when both conditions hold; otherwise it degrades to B' silently and the contingency is not honest:

1. **Structural placement lock.** `CredentialRef<C>` / `SlotBinding` / `SchemeGuard<'a, C>` MUST land in `nebula-credential`'s public surface, NOT in `nebula-action::credential::*`. If credential vocabulary lands in action with delegation up to credential, that IS the boundary-erosion variant the rejection of B' was supposed to prevent. Action's `resolve_as_<capability><C>` thunk is the only action-side surface; the vocabulary types are credential-side, full stop.
2. **Committed credential CP6 implementation cascade slot.** B'+ contingency activation requires a **committed credential CP6 implementation cascade slot** (named owner, scheduled date, queue position) — not a TODO comment, not "we'll schedule it." Without a committed slot, B'+ degrades to B' silently — direct `feedback_active_dev_mode.md` violation. CP2 §4 / CP3 §6 must verify the slot is committed before ratifying any B'+ contingency activation.

### §3.3 Options B' and C' — rejected, with cited rationale

**B' — action-only fixes, defer CP6 to a separate cascade.** Rejected per `feedback_boundary_erosion.md` and tech-lead round 1+2 ([`03b-tech-lead-priority-call.md`](../drafts/2026-04-24-nebula-action-redesign/03b-tech-lead-priority-call.md)). B' fits a 4-5 agent-day implementation budget (tighter than A') and clears the macro/JSON-depth surface, but:

- It requires a **permanent engine-level bridge** emulating phantom-safety at runtime — direct boundary-erosion violation.
- It leaves the action crate with a superseded credential idiom; plugin authors continue to encounter the P1 friction class until the credential cascade lands.
- It pays plugin migration cost twice: once for B' (deprecate `credential<S>()`, fix macros) and once for the eventual CP6 cascade.
- It does not close the §2.1 / §2.2 canon revision; canon §3.5 stays drifted longer.
- Security-lead 03c ACCEPT with must-have floor applies, but the must-have CR3 fix at "deprecate, don't remove" form is `feedback_no_shims.md` violation — and tech-lead noted in round 2 that B' tends to ratify the deprecation rather than remove the method.

Tech-lead's explicit 4th-place ranking ([`03b-tech-lead-priority-call.md`](../drafts/2026-04-24-nebula-action-redesign/03b-tech-lead-priority-call.md) round 2: "A' 1st / B'+ 2nd / C' 3rd / B' 4th") was a serious-option finding, not dismissive — B' is local-optimal, just not global-optimal in the active-dev frame.

**C' — escalate credential spec revision.** Rejected because the spec-reality gap is **implementation lag, not spec defect**. Spike iter-1 + iter-2 ([`2026-04-24-credential-redesign-strategy.md`](2026-04-24-credential-redesign-strategy.md) §6.1, commits `acfec719` and `1c107144`); iter-3 Gate 3 sub-trait × phantom-shim composition validated per credential **Tech Spec** [`2026-04-24-credential-tech-spec.md`](2026-04-24-credential-tech-spec.md) §15.12.3 line 3689 (commit `f36f3739`). All three iters validate the chosen shape compiles, satisfy dyn-safety via ADR-0035 phantom-shim, and clear all 5 spike scope questions with no fallback triggered. Performance (Criterion p50/p95/p99 over 100K iter) shows all three `CredentialRef<C>` hypotheses ~150× under the 1µs absolute ceiling. The spec is validated, not authored speculatively. Per `feedback_adr_revisable.md`, ADRs are point-in-time and supersede is acceptable, but unfreezing a 2-week-old frozen spec that CP6 Gate 1/2/3 ratified — to accommodate a derive-only macro that **structurally cannot** perform field-type rewriting (Phase 0 [`01-current-state.md`](../drafts/2026-04-24-nebula-action-redesign/01-current-state.md) §2 finding C1: "derives structurally cannot do this") — is disproportionate. C' was a serious option (security-lead 03c ACCEPT applies); it lost on bar.

C' would also unfreeze the cascade: escalation rule 10 (cross-crate freeze) would trigger, requiring user authorization. A' implements the frozen spec rather than amending it, sidestepping the rule per [`03-scope-decision.md`](../drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) §0 reframe.

### §3.4 Out-of-scope markers

The following are DEFERRED with explicit sub-spec / separate-cascade pointers per `feedback_active_dev_mode.md` ("before saying 'defer X', confirm the follow-up has a home"):

| Deferred item | Sub-spec pointer | Reason |
|---|---|---|
| DataTag hierarchical registry (58+ tags) | Future port-system sub-cascade | Net-new surface; orthogonal to action core |
| `Provide` port kind | Same sub-cascade as DataTag | Net-new; not cascade-gating |
| Engine cluster-mode coordination implementation (leader election, reconnect orchestration) | Engine cluster-mode coordination cascade — tech-lead schedules post-action-cascade close, queued behind credential CP6 implementation cascade | Engine-layer concern; action surfaces hooks only per §3.1 component 7 |
| `Resource::on_credential_refresh` full integration | Absorbed into resource cascade or co-landed with credential CP6 implementation | Depends on `SchemeFactory` availability per credential Tech Spec §15.7 |
| Post-cascade implementation of CP6 vocabulary | User decision: (a) single coordinated PR, (b) sibling cascades, (c) phased rollout | Implementation not in cascade scope per [`03-scope-decision.md`](../drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) §6 |
| Adjacent finding T3 — dead `nebula-runtime` reference in `test-matrix.yml:66` + `CODEOWNERS:52` | Separate PR (not action cascade's scope) | Filed at cascade end |
| S-W2 `SignaturePolicy::Custom(Arc<dyn Fn>)` | Webhook hardening cascade (2 release cycles) | Sunset committed in §2.12 |
| S-C4 detached spawn zeroize defeat | Credential CP6 landing (if A' implementation proceeds) OR standalone credential cascade | Depends on §15.7 SchemeGuard implementation |
| S-O1 / S-O2 / S-O3 output pipeline caps | Output-pipeline hardening cascade | Surface non-credential, deferred |
| S-I2 `IsolationLevel::CapabilityGated` documented-false-capability | Sandbox phase-1 cascade | Per §2.7 isolation honesty |
| S-W1 / S-W3 / S-F1 / S-I1 / S-U1 / S-C1 minor defense-in-depth | Cascade exit notes | Per security-lead 03c sunset list |
| Signed manifest infrastructure for `#[plugin_credential]` | Strategy §6.5 queue #7 (post-MVP, independent track) | Per credential Strategy §2.1 |
| `#[trait_variant::make(Handler: Send)]` adoption | Separate Phase 3 redesign decision; opens after action cascade closes (gates on user authorization) | Deferred / not absorbed into A'; rationale: `*Handler` HRTB modernization (§4.3.1) adopts single-`'a` + `BoxFut<'a, T>` type alias pattern (rust-senior 02c §6 line 358) without `trait_variant` dep adoption — `#[trait_variant::make]` would collapse the RPITIT/HRTB split into a single source generating both, breaking existing public `*Handler` trait surface (callers writing `impl StatelessHandler` by hand would need to adopt the `async fn` shape). Per rust-senior 02c §6 line 362-380. |

---

## §4 Recommendation

§3 closes the option-ranking exercise; §4 locks the chosen direction and the load-bearing sub-decisions the Tech Spec (Phase 6) needs as inputs. Strategy frames; tech-lead ratifies at CP2 close; user picks the implementation path at Phase 8.

### §4.1 Locked decision: Option A'

**A' is the chosen direction** ([`03-scope-decision.md`](../drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) §1, ratified at Phase 2). Cascade scope is **DESIGN closure only** — Tech Spec covers action + credential CP6 design + engine wiring + plugin migration + workspace hygiene; post-cascade implementation is user-gated.

The eight A' components (§3.1) are unchanged and inherit from the scope decision. §4 does not re-derive them — it locks the load-bearing sub-decisions inside those components and frames the implementation-path choice for the user.

### §4.2 Implementation path framing (post-cascade — USER DECISION at Phase 8)

Strategy presents three implementation paths; user picks. None is recommended at Strategy level — each carries a distinct trade-off shape that depends on cross-team scheduling the Strategy cannot resolve.

| Path | Shape | Trade-off | Best when |
|---|---|---|---|
| **(a) Single coordinated PR** | One PR landing both crates' CP6 vocabulary + engine wiring + plugin migration in lockstep. Estimated 18-22 agent-days full impl per architect 03a §1; 8-12d per tech-lead round 1 (gap reflects whether codemod + plugin migration are counted). | Single cut; no two-vocabulary intermediate state. Highest coordination cost; reviewer load is 27+ engine import sites + 7 reverse-deps in one diff. | Credential-crate owner + action-crate owner have concurrent bandwidth; reviewer headcount available for one large landing. |
| **(b) Sibling cascades — credential leaf-first, action consumer-second** | Credential CP6 implementation cascade lands `CredentialRef<C>` / `SlotBinding` / `SchemeGuard` / `SchemeFactory` / `RefreshDispatcher` first. Action consumer cascade adopts CP6 vocabulary second in lockstep. Each fits normal autonomous budget. | Sequencing friction during the gap (CP6 surface lands before action consumer; engine sees CP6-shaped credential surface without action consumer). Tech-lead round 2 preference if credential-crate owner has bandwidth. | Owner bandwidth permits leaf-first; cascade sequencing tolerable; reviewer load amortized over two diffs. |
| **(c) Phased rollout with B'+ surface commitment** | Action ships CP6 API surface (the user-visible types) with delegating internals while credential cascade lands CP6 internals; plugin authors do not re-migrate. Action's `resolve_as_<capability><C>` thunk is the only action-side bridge; vocabulary types stay in `nebula-credential` per §3.2 condition (1). | Tightly bounded internal bridge in action; sunset-committed (deleted when credential CP6 internals land); never appears in engine. Requires committed credential CP6 implementation cascade slot per §3.2 condition (2) — silent degradation to B' otherwise. | Credential-crate owner cannot start CP6 implementation immediately but slot is committed; plugin authors need CP6 surface immediately for downstream work. |

**Activation precondition for path (c) — non-negotiable** (per §3.2): `CredentialRef<C>` / `SlotBinding` / `SchemeGuard<'a, C>` MUST land in `nebula-credential` (NOT `nebula-action::credential::*`); committed credential CP6 implementation cascade slot (named owner, scheduled date, queue position) MUST exist before activation. CP3 §6 records the contingency activation criteria explicitly. Strategy does not pre-pick (a)/(b)/(c); orchestrator surfaces the choice to user when cascade summary lands (Phase 8).

### §4.3 Load-bearing sub-decisions locked here

Three CP1 §5 open items resolve in §4. Each was flagged by tech-lead in the CP1 review pre-load hints as "must lock before CP2 close."

#### §4.3.1 `*Handler` HRTB modernization scope — IN this cascade

**Decision (recommendation, tech-lead to ratify):** include `*Handler` HRTB modernization as a **first-class component of A'**, not optional, not deferred.

**Reasoning.** rust-senior 02c §8 line 439 (Top-N findings table, row 2) quantifies the change at ~30-40% LOC reduction across `stateless.rs:313-322`, `stateful.rs:461-472`, `trigger.rs:328-381`, `resource.rs:83-106` plus mirrored adapter sites — single `'a` lifetime + `BoxFut<'a, T>` type alias replacing `for<'life0, 'life1, 'a>` + `where Self: 'a, 'life0: 'a, 'life1: 'a`. 02c §6 line 358 confirms the cut at ~8 lines per handler trait, dyn-safety preserved. Rust 1.95 elision rules accept the single-lifetime form (per 02c line 55); dyn-safety is preserved (per 02c line 358); no semver break for runtime callers (rustdoc / cargo-semver-checks visibility only). Note: 02c §6 line 386's "Phase 3 redesign decision with clear LOC payoff" quote refers to `#[trait_variant::make]` adoption, which §4.3.1 scopes OUT (see below).

The change is **mechanically low-risk** (type alias + lifetime collapse, no semantic shift) and **architecturally coherent with A'** — A' rewrites the `*Handler` family anyway to consume `CredentialRef<C>` / `SlotBinding`. Modernizing HRTB shape inside that rewrite costs marginal incremental Tech Spec effort; deferring to a separate hygiene PR forces a second pass over the same files. `feedback_active_dev_mode.md` (more-ideal over more-expedient) and `feedback_idiom_currency.md` (1.95+ idioms; pre-1.85 HRTB shapes are anti-patterns now) both load-bear here. Tech-lead's CP1 review (line 39-42) flagged `feedback_idiom_currency.md` as "mandatory if CP2 §4 picks up the HRTB modernization as in-scope"; §2.11 explicit citation roll-up deferred to CP3 per CHANGELOG §2.11-amendment-pending entry (also tracked in §5 open items).

**What this does NOT include.** `#[trait_variant::make(Handler: Send)]` adoption (rust-senior 02c §6 line 362-380) is **scoped-out** as a separate Phase 3 redesign decision. `trait_variant` would collapse the RPITIT/HRTB split into a single source generating both — breaks existing public `*Handler` trait surface (callers writing `impl StatelessHandler` by hand would need to adopt the `async fn` shape). See §3.4 OUT row for `#[trait_variant::make(Handler: Send)]` adoption.

#### §4.3.2 `unstable-retry-scheduler` — wire-end-to-end OR retire

**Decision (recommendation, tech-lead to ratify):** apply tech-lead's gate-AND-wire discipline (Phase 1 solo-decided call on `ActionResult::Terminate`, per [`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md) §7) to `unstable-retry-scheduler` consistently. Two paths, **chosen at Tech Spec §9 design time, not Strategy**:

1. **Wire both `Retry` and `Terminate` end-to-end** — scheduler infrastructure lands in cascade scope; `unstable-retry-scheduler` and `unstable-terminate-scheduler` (or unified `unstable-action-scheduler`) feature flags both come down when wiring lands; PRODUCT_CANON §11.2 line 286-298 graduates from "planned" to "shipped."
2. **Retire `unstable-retry-scheduler`** — if scheduler infrastructure is **not** in cascade scope, both `ActionResult::Retry` and `ActionResult::Terminate` variants are **gated and wired-stub** (return-typed-error-not-implemented at engine boundary), feature flag stays, canon §11.2 stays "planned." Retiring the dead feature is per `feedback_active_dev_mode.md` (no gate-only-and-defer).

**Why §4 doesn't pick.** The choice depends on `Terminate` scheduler design depth, which is Tech Spec §9 / engine wiring scope — Strategy cannot resolve it without the design pass. **Strategy locks the principle**: Retry and Terminate share the same gating discipline; either both wire end-to-end or both stay gated-with-wired-stub; **no parallel retry surface** (per §2.4); **no `Retry` wired and `Terminate` gate-only** (asymmetry violates `feedback_active_dev_mode.md`). Tech Spec §9 picks the path; CP3 §6 records the chosen path in the post-validation roadmap.

#### §4.3.3 Migration codemod high-level design — scope only

**Decision (recommendation, tech-lead to ratify):** Strategy locks the **scope** of the codemod (which mechanical transforms are required); detailed implementation deferred to Tech Spec §9 handoff.

**Codemod scope (mechanical transforms identified).**

1. **`#[derive(Action)]` → `#[action]`** with attribute-args migration: `key`, `name`, `description`, `version`, `tags`, `category` carry over verbatim; `credential = Type` / `credentials = [Type, ...]` rewrite to attribute-tagged `credentials(slot: Type)` zones per §3.1 component 2; `parameters = Type` (broken in current macro per [`02c-idiomatic-review.md`](../drafts/2026-04-24-nebula-action-redesign/02c-idiomatic-review.md) §2 line 134-141) drops or rewrites depending on Tech Spec §7 ActionContext API.
2. **`ctx.credential_by_id(...)` / `ctx.credential_typed(...)` / `ctx.credential::<S>(key)` (3 variants) → `ctx.credential::<S>(key)` / `ctx.credential_opt::<S>(key)`** per CP6 spec — exact API location to be pinned in credential Tech Spec (open item §5.1.1 below, was CP1 §5 open item §1(a)).
3. **`CredentialContextExt::credential<S>()` no-key heuristic (S-C2 / CR3) — hard removal**, no `#[deprecated]` shim (per §2.12 item 2 + `feedback_no_shims.md` + security-lead 03c §1 VETO). Codemod must error on remaining call sites with crisp diagnostic, not silently rewrite.
4. **`crates/action/macros/Cargo.toml` `[dev-dependencies]` block** — must include `trybuild`, `macrotest` per [`01-current-state.md`](../drafts/2026-04-24-nebula-action-redesign/01-current-state.md) Phase 0 T1 finding. Codemod adds the block; new harness lands alongside macro rewrite.
5. **`nebula-sdk::prelude` re-export reshuffle** — 40+ re-exports per scope decision §1.6; codemod identifies removed names; reverse-dep migration guide lists added/removed/renamed pairs.

**Scope of "design only."** Codemod **execution** (running it on 7 reverse-deps) is post-cascade per §3.4 OUT row "Post-cascade implementation of CP6 vocabulary." Codemod **design** (script shape, transform list, dry-run output format) is Tech Spec §9 scope. Strategy locks transforms 1-5 as the minimal complete set; Tech Spec may add transforms during §9 design without re-opening Strategy.

### §4.4 Security must-have floor — invariant (citing §2.12)

The four-item floor from [`03-scope-decision.md`](../drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) §3 is **invariant, not goal**; in cascade scope regardless of all other decisions; Phase 6 Tech Spec must cite each item as not-deferrable. Listed here verbatim for §4 lock:

1. **CR4 / S-J1 JSON depth bomb** — depth cap (128) at every adapter JSON boundary (`StatelessActionAdapter::execute`, `StatefulActionAdapter::execute`, API webhook body deserialization). Mandated by v2 design spec post-conference amendment B3.
2. **CR3 / S-C2 cross-plugin shadow attack** — replace type-name heuristic with explicit keyed dispatch at method-signature level. **Hard removal**, not `#[deprecated]` shim (`feedback_no_shims.md`; security-lead 03c §1 VETO on shim form). A' removes the method class entirely.
3. **`ActionError` Display sanitization** — route through `redacted_display()` helper in `tracing::error!` call sites to preempt the S-C3 / S-O4 leak class (per §2.6).
4. **Cancellation-zeroize test** — closes S-C5; pure test addition, no architectural cost.

Per `feedback_observability_as_completion.md` (typed error + trace span + invariant check are DoD), items 2-3 in particular must ship with trace spans + invariant checks, not as follow-up. §2.11 explicit citation roll-up deferred to CP3 per CHANGELOG §2.11-amendment-pending entry (also tracked in §5 open items).

## §5 Open items + spike plan

§4 closes three CP1 open items. §5 tracks the remainder + adds tech-lead-flagged items + locks the spike plan that gates Phase 6 Tech Spec writing.

### §5.1 Tracked open items

CP1 §5 closed-by-CP2: §0 freeze policy (no objection at CP1 lock); §3.1 component 5 `*Handler` HRTB scope (§4.3.1 in-scope); §3.4 `unstable-retry-scheduler` retire-vs-wire (§4.3.2 principle locked, Tech Spec §9 picks path); §1 dx-tester figure citation (tech-lead CP1 review confirmed acceptable).

CP1 §5 carrying forward to CP3 / Tech Spec:

#### §5.1.1 ActionContext API location in credential Tech Spec (was CP1 §1(a))

**Open question.** `ActionContext::credential::<S>(key)` / `credential_opt::<S>(key)` API surface location in credential Tech Spec is unresolved. Credential Tech Spec §2.7 (line 486-528) covers `#[action]` macro rewriting only, NOT the ActionContext API contract. Spec-auditor's CP1 finding 2 raised the same gap.

**Resolution scope.** Pin the actual ActionContext API location before Tech Spec §7 (Interface section) writing begins. Likely §2.6 or §3 in credential Tech Spec — separate from §2.7 macro translation. **Owner:** architect, in coordination with credential Tech Spec author. **Deadline:** before Phase 6 Tech Spec §7 drafting begins.

#### §5.1.2 `redacted_display()` helper crate location (was CP1 §2.6)

**Open question.** The `redacted_display()` helper does not yet exist in `nebula-error` or `nebula-log`; Tech Spec must specify which crate hosts it. Possibly an open glossary item.

**Resolution scope.** Decide hosting crate (likely `nebula-log` or new `nebula-redact` per credential Strategy §6.5 queue) before Tech Spec §4 (Security section) writing begins. **Owner:** architect, in coordination with security-lead. **Deadline:** Phase 6 Tech Spec §4.

#### §5.1.3 Credential Tech Spec §7.1 precise line numbers (was CP1 §2.8)

**Open question.** Credential Tech Spec §7.1 is referenced as authoritative for `RefreshDispatcher::refresh_fn` HRTB pattern; precise line-number citation pending full §7 read.

**Resolution scope.** Pin line numbers when CP3 §6 cites the dispatch pattern. **Owner:** architect. **Deadline:** CP3 draft.

#### §5.1.4 B'+ contingency activation criteria detail (tech-lead CP2 hint)

**Open question.** §3.2 conditions (1) + (2) lock the structural and slot-commitment preconditions for B'+ activation, but the **signal that triggers fall-back from A' to B'+** is not enumerated. Tech-lead CP1 review pre-load hint flags this as CP3 §6 scope.

**Resolution scope.** CP3 §6 (post-validation roadmap) must record:
- What signal triggers B'+ over A' (e.g., credential-crate owner bandwidth gap surfaces during Tech Spec §3.4 design; cluster-mode hook design surfaces unbudgeted constraint; reviewer load on single-PR path proves untenable).
- Rollback path (B'+ → A' upgrade when credential CP6 implementation cascade lands).
- Sunset commit on action's internal bridge layer.

**Owner:** architect (CP3 draft); tech-lead (CP3 ratify).

#### §5.1.5 Engine cluster-mode hooks final shape (tech-lead CP2 hint)

**Open question.** §3.1 component 7 names three cluster-mode hooks on `TriggerAction` (`IdempotencyKey`, `on_leader_*` lifecycle, `dedup_window` metadata); precise trait shape, default-body availability, and engine-side registration contract are Phase 6 Tech Spec scope.

**Resolution scope.** Tech Spec §7 Interface section locks the three-hook shape. Strategy locks "surface contract only; engine cluster coordination implementation out of scope" per §3.4. **Owner:** architect (Tech Spec §7); tech-lead (ratify).

### §5.2 Spike plan

Tech-lead CP1 review pre-load hint (line 115): "§5 must include spike for `SlotBinding::resolve_fn` HRTB compilation against current credential internals. This is a dependency-discharge spike before Tech Spec writing begins (Phase 6)."

Architect CP1 forward-flag aligns: HRTB fn-pointer + `SchemeGuard<'a, C>` cancellation drop-order verification gate Tech Spec §7 Interface confidence.

#### §5.2.1 Spike target

**HRTB fn-pointer + `SchemeGuard<'a, C>` cancellation drop-order verification.** Two questions to discharge:

1. Does `SlotBinding::resolve_fn: for<'ctx> fn(&'ctx CredentialContext<'ctx>, &'ctx SlotKey) -> BoxFuture<'ctx, Result<ResolvedSlot, ResolveError>>` (per credential Tech Spec §3.4 line 869, verbatim) compile against the engine wiring pattern A' requires? Specifically: HRTB lifetime quantification + `BoxFuture` erasure + integration with the `#[action]` attribute macro's emission contract. Tech Spec §3.4 line 869 explicitly notes `SlotBinding::resolve_fn` shares the *shape* of `RefreshDispatcher::refresh_fn` (§7.1) but takes `&'ctx SlotKey` / returns `Result<ResolvedSlot, ResolveError>` — the spike validates the resolve-site shape; refresh-site dispatch is exercised compositionally via iter-2 Action B (§5.2.3).
2. Does `SchemeGuard<'a, C>` (per credential Tech Spec §15.7 line 3394-3429: `!Clone`, `ZeroizeOnDrop`, `Deref`, lifetime parameter) honor zeroize-on-drop semantics across the **cancellation boundary** — drop guard mid-`.await` (under `tokio::select!` with cancellation token), confirm zeroize fires before scope exit?

#### §5.2.2 Iter-1 — minimum compile shape

**Iter-1 scope** (one cascade prompt, max ~2 hours equivalent): minimal `CredentialRef<C>` / `AnyCredential` / `SlotBinding` skeleton in scratch crate + hand-expansion of `#[action]` attribute macro emission for 1 representative action (likely Stateless+Bearer to match credential Tech Spec spike iter-1 shape) + compile-fail probes (`trybuild`-style) for key invariants:

- **Probe 1:** `ResourceAction` without `Resource` binding fails to compile (validates ActionSlots impl emission).
- **Probe 2:** `TriggerAction` without trigger source fails to compile (validates trait contract enforcement).
- **Probe 3:** Credential field rewriting confined to attribute-tagged zones — bare `CredentialRef<C>` field outside `credentials(slot: Type)` zone fails or warns (validates §3.1 component 2 narrow declarative rewriting contract per tech-lead §2 architectural coherence constraint).

**Iter-1 DONE:** all 3 probes compile-fail as expected; minimal skeleton compiles clean; scratch crate `cargo check --workspace` clean.

#### §5.2.3 Iter-2 — composition + cancellation + perf sanity

**Iter-2 scope** (one cascade prompt, max ~3 hours equivalent): composition with 2-3 realistic actions covering different variants:

- **Action A:** `StatelessAction` + `Bearer` credential (single slot, simple capability, validates Pattern 1 concrete `CredentialRef<ConcreteCredential>` pass-through per credential Tech Spec §3.4).
- **Action B:** `StatefulAction` + `OAuth2` credential with refresh (validates Pattern 2 dispatch narrative + `RefreshDispatcher::refresh_fn` HRTB integration).
- **Action C:** `ResourceAction` + `CredentialRef<C>` + `Postgres` resource (validates resource binding + credential composition + `Resource::on_credential_refresh` interaction surface, even if full integration is deferred).

Plus: dispatch ergonomics check (does action body compile cleanly with `ctx.credential::<S>(key)`?) + cancellation-drop-order test (drop `SchemeGuard<'a, C>` mid-await under `tokio::select!`; assert zeroize via custom `Drop` instrumentation) + macro expansion perf sanity (expansion time within 2x of current `#[derive(Action)]` baseline).

**Iter-2 DONE:** all 3 actions compile + dispatch-shape compiles clean + cancellation-drop test passes + expansion perf within 2x.

#### §5.2.4 Spike DONE criteria + budget

**Aggregate DONE.** All probes (iter-1) pass + composition (iter-2) compiles + cancellation-drop-order test passes + expansion perf within 2x of current macro baseline.

**Spike worktree:** isolated; scratch only; no commit to main. Pattern follows credential Strategy §6.1 spike iter-1/2 + credential Tech Spec §15.12.3 iter-3 worktree pattern.

**Budget:** max 2 iterations per cascade prompt. Failures may trigger Strategy revision (CP3 amendment) but are NOT cascade-blocking — spike failure narrows §4.2 path choice (e.g., if HRTB shape doesn't compose cleanly, path (c) B'+ activation criteria broaden) or surfaces a Tech Spec §7 redesign requirement, not a cascade-blocking escalation. Specifically: if iter-1 probe 3 fails (narrow declarative rewriting cannot be enforced compile-time — bare `CredentialRef<C>` outside the `credentials(slot: Type)` zone compiles cleanly), §3.2 condition (1) for B'+ activation may shift — `CredentialRef<C>` placement constraint becomes runtime-enforced rather than compile-time, requiring CP3 §6 amendment to B'+ activation criteria. This pre-empts the silent-degradation trap (B'+ → B') if probe 3 fails.

#### §5.2.5 Spike → Tech Spec interface lock plan

**Spike output artefacts** (per credential Strategy §6.1 spike pattern):
- `NOTES.md` — design decisions, surprises, blocked paths, open issues.
- `final_shape_v2.rs` — minimal compile-clean skeleton with all 3 spike actions + HRTB + `SchemeGuard` + macro emission shape locked.
- Test artefacts — `trybuild` probe outputs + cancellation-drop-order test results + expansion perf measurements.

These three artefacts become **input to Tech Spec §7 Interface section in Phase 6**. Tech Spec §7 cites the spike artefacts directly; if §7 needs to deviate from spike-locked shape, the deviation is a CP3 amendment with rationale.

**Sequencing.** Spike runs in Phase 4 (parallel with CP3 drafting where feasible). Phase 6 Tech Spec writing does not begin until spike DONE criteria met.

## §6 Post-validation roadmap

**Scope of this section.** §6 records the obligations and contingencies that activate **after Strategy freeze** (Phase 4 onwards). It is the bookkeeping ledger that converts CP1+CP2 promises into trackable, owner-stamped follow-ups. Strategy decisions remain locked at §1–§5; §6 does not introduce new decisions, only sequencing, ownership, and post-cascade tracking. Per `feedback_active_dev_mode.md` ("before saying 'defer X', confirm the follow-up has a home") and tech-lead 05b "Top 2 §6 hints" (line 171), every deferral below names a target cascade, a sunset window, and a responsible role.

### §6.1 Spike → Tech Spec sequencing

**Phase 4 spike outputs become Phase 6 Tech Spec inputs.** Per §5.2.5, the spike emits three artefacts at `docs/superpowers/drafts/2026-04-24-nebula-action-redesign/spike/` — `NOTES.md` (design decisions, surprises, blocked paths), `final_shape_v2.rs` (compile-clean skeleton with HRTB + `SchemeGuard` + macro emission shape locked), test artefacts (`trybuild` probe outputs + cancellation-drop-order test + expansion perf measurements). A fourth artefact is committed by the spike worktree: the **commit hash** of the final spike state (worktree-local; not merged to main per §5.2.4 — recorded in `NOTES.md` for traceability).

**DONE → Tech Spec §7 unblocks.** When spike aggregate DONE criteria (§5.2.4) are met, Tech Spec §7 Interface drafting unblocks. Tech Spec §7 cites the four artefacts by path + commit hash; §7 deviations from spike-locked shape land as CP3 amendments with rationale (per §5.2.5 lock plan).

**FAIL → Strategy revision via CP3 amendment (allowed).** Per `feedback_adr_revisable.md`, ADRs (and Strategy decisions) are point-in-time. Spike failure is not cascade-blocking; it triggers principled revision. Two failure modes have pre-staged paths: (a) iter-1 probe 3 fails (narrow declarative rewriting cannot be enforced compile-time) → §3.2 condition (1) for B'+ activation broadens (see §6.8), CP3 amendment lands in §5.2.4 budget framing; (b) iter-2 composition or expansion-perf misses 2x ceiling → Tech Spec §7 redesign requirement surfaces, CP3 amendment lands in §5.2.5 lock plan. Either path preserves Strategy authority while honestly recording the constraint shift. CP3 amendment mechanics follow §0 (inline "*Amended by ADR-NNNN, YYYY-MM-DD*" prefix at changed paragraph; surrounding decision not retracted).

### §6.2 ADR drafting roadmap

Phase 5 emits ADRs that ratify the Strategy → Tech Spec lock points. **Three required + one optional**, drafted in the order below so each ADR's context is grounded in a frozen prior:

| ADR | Slot | Subject | Citation context |
|---|---|---|---|
| **ADR-00NN** | required | Trait shape: `#[action]` attribute macro replaces `#[derive(Action)]`; narrow-zone rewriting contract (only `credentials(slot: Type)` zone is rewritten, validated by spike iter-1 probe 3) | Cites §3.1 component 1 + §3.2 A' selection + §4.3 sub-decisions; constrained by ADR-0035 phantom-shim composition |
| **ADR-00NN+1** | required | Macro emission contract (post-spike-validated): typed-only credential surface, `Input: HasSchema` documented bound, `parameters` emission corrected, `semver` dep declared | Cites §1(b) macro emission bugs + §5.2.3 expansion-perf gate |
| **ADR-00NN+2** | required | ControlAction seal + canon §3.5 DX tier ratification | Canon revision per [`PRODUCT_CANON.md`](../../PRODUCT_CANON.md) §0.2 (line 27) — "explicit canon revision required to add another action trait"; ratifies the 5-trait DX tier vs the 4-trait dispatch core distinction; CR3 fix lands as `feedback_no_shims.md`-compliant removal (not deprecation) |
| **ADR-00NN+3** | optional | TriggerAction cluster-mode hooks contract — 3 hooks (`IdempotencyKey`, `on_leader_*`, `dedup_window`) with default bodies vs required-implementation delineation per §3.1 / §5.1.5 / scope decision §1.7 | Per tech-lead 05b CP3 hint line 143 + §3.4 row 3 (cluster-mode coordination cascade scheduling); optional in this cascade because §3.1 component 7 surfaces hooks only — full integration deferred |

**Phantom-shim citation.** ADR-00NN+1 (macro emission) and ADR-00NN+2 (ControlAction seal) compose with [ADR-0035](../../adr/0035-phantom-shim-capability-pattern.md) phantom-shim pattern at the credential resolution call-site — `SchemeGuard<'a, C>` + `RefreshDispatcher::refresh_fn` HRTB shape per credential Tech Spec §15.12.3. ADRs explicitly cite ADR-0035 as load-bearing.

**Drafting order.** ADR-00NN before ADR-00NN+1 (trait shape grounds emission contract); ADR-00NN+1 before ADR-00NN+2 (emission contract grounds canon revision rationale). ADR-00NN+3 may be drafted in parallel with ADR-00NN+2 once §3.4 row 3 cluster-mode cascade scheduling is committed (§6.6).

### §6.3 Tech Spec checkpoint roadmap

Phase 6 Tech Spec drafting follows a **4-CP cascade prompt sequence**, mirroring credential Tech Spec CP1–CP6 cadence ([`2026-04-24-credential-tech-spec.md`](2026-04-24-credential-tech-spec.md) per `feedback_active_dev_mode.md` precedent):

| CP | Sections | Focus | Reviewer matrix |
|---|---|---|---|
| **CP1** | §0–§3 | Goal, non-goals, lifecycle, state machine | rust-senior + spec-auditor (parallel) → architect iterate → tech-lead ratify |
| **CP2a** | §4–§5 | Storage schema, security threat model | security-lead + spec-auditor (parallel) → architect iterate → tech-lead ratify |
| **CP2b** | §6–§8 | Operational, testing plan, interface | rust-senior + dx-tester + spec-auditor (parallel) → architect iterate → tech-lead ratify |
| **CP3** | §9–§13 | Migration, codemod, retry-scheduler chosen path, observability spans | devops + rust-senior + spec-auditor (parallel) → architect iterate → tech-lead ratify |
| **CP4** | §14–§16 | Open items, accepted gaps, handoff | spec-auditor full audit → architect iterate → tech-lead ratify → freeze |

**Per-CP reviewer matrix discipline.** All listed reviewers run in parallel for the same checkpoint (orchestrator-coordinated via teammate `SendMessage` or sub-agent dispatch). Architect consolidates feedback in **one** iteration pass per CP (no multi-round drafting within a CP — per `feedback_context_hygiene.md`, reset-to-spec rather than pile-on). Tech-lead ratifies after consolidation; any required changes flagged at ratification re-open the CP for one additional iteration only. CP4 spec-auditor runs a full cross-CP audit before freeze.

### §6.4 Concerns register lifecycle

Phase 7 activates **only if** Phase 1 surfaced 🔴 blocking concerns requiring 3-stakeholder consensus that did not resolve at Strategy lock. Living register at [`docs/tracking/nebula-action-concerns-register.md`](../../tracking/nebula-action-concerns-register.md) (created at Phase 7 entry; absent if Phase 7 is skipped).

**6-label classification.** Per credential cascade precedent: `strategy-blocking` (gates Strategy freeze; CP3 cannot lock), `tech-spec-material` (reshapes Tech Spec content; lands as Tech Spec section), `sub-spec` (orthogonal scope; spawns separate sub-spec), `quick-fix` (small PR; ≤1 cascade-day fix; lands directly), `cross-crate-blocking` (touches a frozen sibling crate; requires sibling-cascade coordination), `post-cascade` (deferred to post-cascade tracking; sunset window in §6.7).

**Lifecycle.** Each concern enters with one label, may be re-labeled exactly once during stakeholder consensus pass, then becomes immutable until closure. Closure requires the responsible role (named in the register row) to mark `resolved` + cite the landing PR / ADR / sub-spec.

### §6.5 Post-cascade implementation path criteria

Phase 8 cascade summary surfaces the (a) / (b) / (c) implementation path choice to the user (per §3.4 last row + §4.2). Strategy does not pre-pick; tech-lead 05b ratified this framing (line 27 — "no pre-emption of user authority"). The decision tree below records **when each path is appropriate**, so the cascade summary can frame the choice with concrete criteria rather than abstract trade-offs:

| Path | Shape | Appropriate when |
|---|---|---|
| **(a)** | Single coordinated PR — co-lands action redesign + credential CP6 implementation | Credential cascade owner has bandwidth committed (§6.6) AND plugin authors can absorb single-PR review surface; estimated 18–22 agent-days aggregate |
| **(b)** | Sibling cascades — action redesign + credential CP6 implementation as parallel cascades | Bandwidth split needed across two owners AND tighter per-cascade review surface preferred; per-cascade budgets re-baselined at path selection (tech-lead 05b CP3 hint line 161 — currently aggregate-budget framing) |
| **(c)** | Phased B'+ surface commitment — action redesign lands now with B'+ shim surface; credential CP6 implementation lands later | Credential cascade slot commitment lapses (§6.6) — i.e., implementation slips beyond cascade close window AND user accepts B'+ contingency activation per §6.8 |

**(c) viability gate.** Path (c) requires B'+ activation; B'+ activation is not user-pickable in isolation — see §6.8 co-decision rule.

### §6.6 Cross-crate coordination tracking

**Credential CP6 implementation cascade slot.** Per tech-lead's silent-degradation guard (CP1 review line 116; restated 05b line 88), B'+ contingency activation requires a **committed implementation cascade slot** for credential CP6 — not a vague "future cascade" pointer. The slot has three required fields: **named owner** (a specific role or contributor), **scheduled date** (an absolute date, not "post-action-cascade"), **queue position** (relative to other queued cascades — per credential Strategy §6.5 queue convention).

**Tracking obligation, not gating.** Strategy makes the slot a **post-cascade tracking obligation**: the slot must be committed before user picks path (c) at Phase 8. The slot does NOT gate Strategy freeze, Tech Spec writing, or cascade close. It gates **path (c) availability only**.

**Failure mode.** If the slot is not committed before Phase 8 user pick, **path (c) is NOT VIABLE** — the user pick narrows to (a) or (b). This is the silent-degradation guard active: Strategy refuses to allow path (c) to be selected on a hand-wave commitment, preserving the `feedback_active_dev_mode.md` discipline that "before saying 'defer X', confirm the follow-up has a home."

**Where the slot lands.** Slot commitment lives in [`docs/tracking/cascade-queue.md`](../../tracking/cascade-queue.md) (or equivalent — orchestrator picks at Phase 8) with the three required fields + a back-pointer to this §6.6.

**Cluster-mode coordination cascade slot.** §6.2 ADR-00NN+3 (TriggerAction cluster-mode hooks) needs an implementation home post-action-cascade. Per §3.4 row 3 tech-lead commitment, the cluster-mode coordination cascade is queued **behind** the credential CP6 implementation cascade. The same three required fields apply (named owner + scheduled date + queue position), tracked in the same `cascade-queue.md` location. Unlike the credential CP6 slot, this cluster-mode slot is **not a path-(c) viability gate** — ADR-00NN+3 is optional (§6.2), and the action cascade closes regardless. The slot is a **traceability obligation** mirroring credential CP6 slot tracking discipline: surfaces the cluster-mode hooks contract has a real implementation home rather than a vague "future cascade" pointer.

### §6.7 Sunset commitments for deferred 🟠 findings

Per security-lead 03c §3 deferred-but-tracked items (cited near-verbatim at §2.12 with cascade-level scope adjustments), each 🟠 deferral below has a target cascade, a sunset window (in release cycles), and a responsible role. Sunset window is the **maximum elapsed time before the deferral is re-escalated** — if the target cascade has not landed by the sunset window, the item becomes a 🔴 cascade-blocking concern automatically.

| Item | Target cascade | Sunset window | Responsible |
|---|---|---|---|
| **S-W2** webhook `SignaturePolicy::Custom(Arc<dyn Fn>)` hardening | Webhook hardening cascade | ≤2 release cycles from action-cascade close | security-lead |
| **S-C4** detached spawn zeroize defeat | Credential CP6 implementation cascade (path a/b) OR standalone credential-keyed-lifetime cascade (path c) | ≤2 release cycles; absorbed into CP6 if path (a) or (b) selected | security-lead + credential cascade owner |
| **S-O1** output pipeline cap absence | Output-pipeline hardening cascade | ≤2 release cycles | rust-senior |
| **S-O2** output pipeline cap absence (parallel) | Output-pipeline hardening cascade | ≤2 release cycles | rust-senior |
| **S-O3** output pipeline cap absence (parallel) | Output-pipeline hardening cascade | ≤2 release cycles | rust-senior |
| **S-I2** `IsolationLevel::CapabilityGated` documented-false-capability | Sandbox phase-1 cascade | ≤3 release cycles (sandbox cascade is heavier) | security-lead + rust-senior |

**Re-escalation mechanics.** When sunset window elapses without target cascade landing, the responsible role files an issue tagged `cascade-blocking-sunset-expired` and adds it to the next cascade-planning round. Per §2.12 verbatim must-have floor, security-lead retains VETO authority on shim-form drift.

### §6.8 B'+ contingency activation criteria

Per §3.2 conditions for B'+ activation, this section pins the **explicit signal triggers + rollback path + co-decision rule + sunset commit** that CP3 §6 was promised to record (per §5.1.4 + tech-lead 05b "Top 2 §6 hints" line 171, hint 1).

**Activation signal (binary OR).** Either:
1. **Spike iter-1 probe 3 fails** (per §5.2.4 budget framing) — narrow declarative rewriting CANNOT be confined compile-time to attribute-tagged zones. Bare `CredentialRef<C>` outside the `credentials(slot: Type)` zone compiles without macro intervention, breaking the rewriting compile-time enforcement promise. Detected at Phase 4 spike iter-1; signal is the `trybuild` probe 3 negative result + spike `NOTES.md` failure record.
2. **Credential cascade slot commitment lapses** (per §6.6) — slot not committed (named owner + scheduled date + queue position) before user picks path (c) at Phase 8. Detected at Phase 8 cascade summary; signal is the absence of the slot row in [`docs/tracking/cascade-queue.md`](../../tracking/cascade-queue.md).

Either signal alone triggers B'+ contingency activation. Signals are not OR-summed (the second signal is a function of cascade scheduling, not spike outcome — they're independent failure modes).

**Rollback path.** B'+ activation rolls back two A' load-bearing decisions:
1. **Remove `#[action]` attribute macro adoption** — revert §4.3.1 sub-decision; do not introduce the attribute macro this cascade.
2. **Reactivate `#[derive(Action)]` (with bug fixes from ADR-00NN+1 macro emission contract)** — keep the existing derive form, land the macro emission bug fixes (CR2, CR8, CR9, CR11 from §1(b)), accept the structural derive limitation.

**Resurfacing as known sunset items.** B'+ activation re-opens **CR1** (typed credential surface unrealized; derive cannot rewrite) and **CR7** (canon §3.5 governance debt unresolved; ControlAction seal + DX tier ratification deferred) as known sunset items. Both get rows in §6.7 with target cascade = "post-A'-failure follow-up cascade (TBD when B'+ activates)" and sunset window = ≤4 release cycles (longer than 🟠 sunsets because these are 🔴 acknowledged-debt, not 🟠 deferred-tracking).

**Co-decision rule.** B'+ activation requires **architect + tech-lead co-decision**, NOT solo orchestrator decision. Per `feedback_adr_revisable.md` (point-in-time decisions can be revised) + `feedback_hard_breaking_changes.md` (architecture-level decisions need expert framing), B'+ activation is a Strategy-level reversal — the orchestrator surfaces the activation signal, architect frames the rollback in CP3 amendment terms (per §0 amendment mechanics), tech-lead ratifies. Solo orchestrator activation is a `feedback_active_dev_mode.md` silent-degradation violation. **Disagreement routing.** If architect + tech-lead split on B'+ activation co-decision, orchestrator does NOT silently break the tie; surfaces both positions to user as tie-break (per orchestrator agent definition handoff rule).

**Sunset commit for B'+ surface.** If B'+ activates, the resulting B'+ shim surface (legacy `#[derive(Action)]` + bug-fix patches + canon §3.5 unresolved) is itself committed to a follow-up cascade with sunset window ≤4 release cycles (per resurfacing as known sunset items above). This pre-empts B'+ becoming a permanent B' degradation — the active-dev discipline applies to contingency surfaces too.

### §6.9 Retry-scheduler chosen path locus

§4.3.2 forward-promise discharge: Strategy locks the **principle only** — `Retry` and `Terminate` share symmetric gating discipline; no parallel retry surface; no asymmetric gate-only. The **concrete path** between (i) wire-end-to-end through the existing `unstable-retry-scheduler` feature flag and (ii) retire the feature flag entirely is selected at **Tech Spec §9 design time**, not at Strategy. Tech-lead 05b §4.3.2 retry-scheduler analysis (line 33) ratifies this split: "Strategy locks principle, Tech Spec picks path." Per `feedback_active_dev_mode.md`, the chosen path must arrive with full observability (typed errors + trace spans + invariant checks) — Tech Spec §9 cannot defer instrumentation as follow-up work.

---

### Open items raised this checkpoint

- §4.3.2 — `unstable-retry-scheduler` wire-end-to-end vs gated-with-wired-stub picked at Tech Spec §9 design time; Strategy locks principle only. CP3 §6 records chosen path.
- §4.3.3 — codemod transform list locked at scope level; Tech Spec §9 may add transforms during design without re-opening Strategy.
- §5.1.1 — ActionContext API location in credential Tech Spec must be pinned before Tech Spec §7 drafting; architect + credential Tech Spec author coordination required.
- §5.1.2 — `redacted_display()` hosting crate decision (likely `nebula-log` or new `nebula-redact`); architect + security-lead coordination required.
- §5.1.4 — B'+ activation signal enumeration + rollback path + sunset commit are CP3 §6 scope.
- §5.1.5 — cluster-mode hooks final trait shape is Tech Spec §7 scope; Strategy locks "surface contract only" per §3.4.
- §5.2 — spike iter-1 + iter-2 sequenced for Phase 4; Tech Spec §7 writing gated on spike DONE criteria.
- §2.11 amendment-pending — `feedback_idiom_currency.md` (load-bearing for §4.3.1) + `feedback_observability_as_completion.md` (load-bearing for §4.4 items 2-3) explicit citation roll-up to §2.11 deferred to CP3 per CP2 iteration. Tracker.
- CP3 §6 inventory — forward-promises now span ~8 sub-items (B'+ activation criteria + signal triggers + rollback path + sunset commit; retry-scheduler chosen path + canon §11.2 edit; cluster-mode coordination cascade scheduling commitment; `#[trait_variant::make]` separate redesign forward-flag; path (b) per-cascade budget re-baseline; spike output → Tech Spec §7 traceability; §2.11 amendment roll-in). CP3 author should treat §6 as a checklist mapping each promise to a sub-section before drafting prose.

### CHANGELOG — CP2 (since CP1 lock)

CP2 single-pass append 2026-04-24:
- Status header — `DRAFT CP1 (iterated 2026-04-24)` → `DRAFT CP2`.
- §4 added — locked decision (A'), implementation path framing (a/b/c for user pick at Phase 8), three load-bearing sub-decisions (HRTB modernization in-scope, retry-scheduler principle locked, codemod scope locked), security must-have floor cited verbatim from §2.12 / scope decision §3.
- §4.3.1 — `*Handler` HRTB modernization scoped IN per rust-senior 02c §6 LOC payoff + `feedback_idiom_currency.md` + `feedback_active_dev_mode.md`. `#[trait_variant::make]` adoption scoped OUT (separate Phase 3 redesign decision); §3.4 OUT row landed in CP2 iteration (2026-04-24).
- §4.3.2 — `Retry` and `Terminate` share gating discipline; Tech Spec §9 picks wire-end-to-end vs gated-with-wired-stub. No parallel retry surface; no asymmetric gate-only.
- §4.3.3 — codemod transforms 1-5 locked as minimal complete set; Tech Spec §9 designs implementation.
- §5 added — open items consolidated (CP1 carried forward + tech-lead CP2 hints), spike plan locked (target + iter-1 scope + iter-2 scope + DONE criteria + budget + Tech Spec interface lock plan).
- §2.11 amendment-pending — `feedback_idiom_currency.md` cited as load-bearing for §4.3.1 HRTB modernization; `feedback_observability_as_completion.md` cited as load-bearing for §4.4 security must-have floor items 2-3. CP3 may roll into §2.11 explicit citations.

CP2 single-pass iteration 2026-04-24 (post spec-auditor REVISE + tech-lead RATIFY-WITH-NITS):
- Status header — `DRAFT CP2` → `DRAFT CP2 (iterated 2026-04-24)`. §0 status table — `(this revision)` annotation moved from CP1 row to CP2 row.
- §5.2.1 question 1 — spike target signature corrected: `&'ctx CredentialId` / `Result<RefreshOutcome, RefreshError>` (RefreshDispatcher::refresh_fn shape) → `&'ctx SlotKey` / `Result<ResolvedSlot, ResolveError>` (SlotBinding::resolve_fn shape, verbatim from credential Tech Spec §3.4 line 869). Spike validates resolve-site shape; refresh-site dispatch exercised compositionally via iter-2 Action B. Spec-auditor 🔴 BLOCKER closed.
- §4.3.1 / §4.4 — dropped "§2.11 amendment in CP2 CHANGELOG cites" forward-promise; reworded to match CHANGELOG's actual deferral state ("§2.11 explicit citation roll-up deferred to CP3"). Added §2.11 amendment-pending tracker to §5 open items. Spec-auditor 🟠 #2 closed.
- §3.4 — added OUT row for `#[trait_variant::make(Handler: Send)]` adoption (Phase 3 redesign decision; rationale: collapses RPITIT/HRTB split, breaks public `*Handler` trait surface, per rust-senior 02c §6 line 362-380). §4.3.1 line 219 reworded "Out-of-scope row added to §3.4 in CP2 CHANGELOG" → "See §3.4 OUT row." Tech-lead required-change #2 closed.
- §5.2 budget framing — added explicit linkage between iter-1 probe 3 fail and §3.2 condition (1) shift; pre-empts silent-degradation trap (B'+ → B') if probe 3 fails. Tech-lead required-change #1 closed.
- CHANGELOG / Handoffs headers — disambiguated CP1/CP2 by qualifying with checkpoint label (`### CHANGELOG — CP2`, `### CHANGELOG — CP1`, `### Handoffs requested — CP2`, `### Handoffs requested — CP1`). Spec-auditor 🟠 #3 closed.
- §4.3.1 — citation routing fixed: ~30-40% LOC figure re-attributed from 02c §6 line 386 to 02c §8 line 439 (Top-N findings table); `02c line 39` → `02c line 55` (elision); `02c line 357` → `02c line 358` (dyn-safety); 02c §6 line 386 quote reframed as referring to OUT-of-scope `#[trait_variant::make]` adoption. Spec-auditor 🟡 nits closed.
- §5.2.4 spike worktree — `credential Strategy §6.1 spike iter-1/2/3 worktree pattern` → `credential Strategy §6.1 spike iter-1/2 + credential Tech Spec §15.12.3 iter-3 worktree pattern`. Inherited CP1 drift fix applied. Spec-auditor 🟡 nit closed.
- §5 open items — added §2.11 amendment-pending tracker + CP3 §6 inventory bookkeeping note (~8 sub-items).

### Handoffs requested — CP2

- **spec-auditor** — please audit §4–§5 for: (a) cross-section consistency with §0–§3 (especially §4.2 path framing alignment with §3.2 B'+ contingency conditions; §4.3.1 alignment with rust-senior 02c §6 line ranges); (b) every claim grounded in code/canon/ADR/scope-decision/tech-lead-CP1-review (target: every line-numbered citation resolves); (c) forward-reference integrity (CP3 §6 references should be marked as deferred, not dangling); (d) terminology alignment with `docs/GLOSSARY.md`; (e) confirm §5.2 spike plan iter-1/iter-2 scope mirrors credential Strategy §6.1 spike pattern (no silent additions or drops).
- **tech-lead** — please review §4 load-bearing sub-decisions for ratification: (1) §4.3.1 `*Handler` HRTB modernization in-scope — confirms your CP1 review hint line 113-114; (2) §4.3.2 retry-scheduler principle (Retry+Terminate symmetric gating; Tech Spec §9 picks path); (3) §4.3.3 codemod transform scope; (4) §4.2 implementation path framing (a/b/c) appropriately frames user choice at Phase 8 without pre-picking. Solo-decider authority on §4 sub-decisions; CP2 lock requires tech-lead explicit ratification.
- **security-lead** — please review §4.4 verbatim citation of must-have floor (especially items 2 and 3 — `feedback_observability_as_completion.md` integration: trace spans + invariant checks must ship with security hardening, not as follow-up). VETO authority retained on shim-form drift in CR3 fix.
- **rust-senior** — please confirm §4.3.1 in-scope decision aligns with 02c §6 LOC payoff framing (single `'a` + `BoxFut<'a, T>` type alias; `#[trait_variant::make]` scoped OUT as separate Phase 3 redesign). Flag any 02c findings that should be load-bearing for CP3 §6 (post-validation roadmap) and aren't yet cited.

### CHANGELOG — CP3 (since CP2 iterated lock)

CP3 single-pass append 2026-04-24:
- Status header — `DRAFT CP2 (iterated 2026-04-24)` → `DRAFT CP3`. §0 status table — `(this revision)` annotation moves from CP2 row to CP3 row at next iteration.
- §6 added — post-validation roadmap with 8 sub-sections covering the inventory bookkeeping promised at §5 open items line 367.
- §6.1 — spike → Tech Spec sequencing; spike DONE unblocks Tech Spec §7; spike FAIL triggers CP3 amendment per `feedback_adr_revisable.md`. Fourth artefact (commit hash) added beyond §5.2.5 three artefacts for traceability.
- §6.2 — ADR drafting roadmap: 3 required ADRs (trait shape; macro emission contract; ControlAction seal + canon revision) + 1 optional (TriggerAction cluster-mode hooks). Drafting order pinned. ADR-0035 phantom-shim cited as load-bearing per credential Tech Spec §15.12.3.
- §6.3 — Tech Spec checkpoint roadmap: 5 CPs (CP1 §0–§3 / CP2a §4–§5 / CP2b §6–§8 / CP3 §9–§13 / CP4 §14–§16) with parallel-reviewer matrix per CP. One-iteration-per-CP discipline locked per `feedback_context_hygiene.md`.
- §6.4 — concerns register lifecycle activated only if Phase 1 surfaced unresolved 🔴; 6-label classification (strategy-blocking / tech-spec-material / sub-spec / quick-fix / cross-crate-blocking / post-cascade); single re-label rule then immutable.
- §6.5 — post-cascade implementation path criteria: (a)/(b)/(c) decision tree with per-path appropriateness conditions; path (c) viability gated on §6.8 co-decision rule.
- §6.6 — cross-crate coordination tracking: credential CP6 implementation slot requires named owner + scheduled date + queue position; tracking obligation, not gating; failure mode locks path (c) NOT VIABLE without slot commitment.
- §6.7 — sunset commitments table for 7 deferred 🟠 findings (S-W2, S-C4, S-C5, S-O1, S-O2, S-O3, S-I2) with target cascade + sunset window + responsible role; re-escalation mechanic when window elapses.
- §6.8 — B'+ contingency activation criteria: binary OR signal (spike probe 3 fail OR slot commitment lapse); rollback path (remove `#[action]` macro + reactivate derive with bug fixes); architect + tech-lead co-decision required (NOT solo orchestrator); B'+ surface itself sunset-committed at ≤4 release cycles.
- §5 open items — CP3 §6 inventory line 367 promise partial discharge: 4/8 forward-promises mapped to §6 sub-sections in CP3 single-pass append (B'+ activation + signal triggers + rollback + sunset → §6.8; cluster-mode coordination cascade scheduling → §6.6; spike → Tech Spec §7 traceability → §6.1; sunset table for 🟠 items → §6.7). Remaining 4 (retry-scheduler chosen path; canon §11.2 edit; path (b) per-cascade budget re-baseline; §2.11 amendment roll-in) land in CP3 freeze iteration via §6.9 (retry-scheduler) + Tech Spec §9 deferral pointers.

### CHANGELOG — CP3 frozen

All 7 reviewer items closed. Strategy frozen 2026-04-24. Forward path: Phase 4 spike → Phase 5 ADRs → Phase 6 Tech Spec.

### Handoffs requested — CP3

- **spec-auditor** — please audit §6 for: (a) cross-section consistency with §0–§5 (especially §6.5 a/b/c framing alignment with §3.2 + §4.2; §6.8 B'+ activation alignment with §3.2 condition 1 + §5.2.4 budget framing); (b) every claim grounded in CP1/CP2 frozen content + tech-lead 05b "Top 2 §6 hints" line 171 + 05b CP3 hints lines 151–161; (c) forward-reference integrity (§6.6 cascade-queue.md and §6.4 concerns-register.md are tracking-system pointers, marked as "created at activation"); (d) terminology alignment with `docs/GLOSSARY.md`; (e) confirm §6.7 sunset table matches §2.12 verbatim must-have floor + scope-decision §3 deferred items (no silent additions or drops).
- **tech-lead** — please ratify §6 sub-decisions: (1) §6.2 ADR list (3 required + 1 optional) — flag if any ADR is missing or any listed ADR should not be a Strategy-locked promise; (2) §6.3 5-CP cadence — confirms credential Tech Spec CP1–CP6 precedent applies; (3) §6.6 slot commitment as path (c) viability gate — confirms your CP1 line 116 silent-degradation guard; (4) §6.8 architect + tech-lead co-decision rule — confirms B'+ activation is NOT solo orchestrator authority. Solo-decider authority on §6 sub-decisions; CP3 lock requires tech-lead explicit ratification.
- **security-lead** — please confirm §6.7 sunset commitments table is complete and accurate against your 03c §3 deferred-but-tracked items list. Flag any S- finding that should appear in §6.7 and doesn't, or any sunset window that should tighten.

### CHANGELOG — CP1 (since initial draft)

CP1 single-pass iteration 2026-04-24 (post spec-auditor + tech-lead RATIFY-WITH-NITS):
- §1 line 42 — dropped PRODUCT_CANON line 24 (was §0.1 layer legend, not §4.2); fixed `§2.S1` → `§3.S1` (S1 lives in §3 of 01-current-state).
- §1(a) — `ctx.credential::<S>(key)` API citation reframed: removed mis-attributed Tech Spec §2.7 line 487-516 pointer; flagged as CP2 open item. Added open item to §5.
- §3.2 — added "Conditions for B'+ activation" sub-paragraph locking (1) `CredentialRef<C>` / `SlotBinding` / `SchemeGuard<'a, C>` placement in `nebula-credential` (not action), (2) committed credential CP6 implementation cascade slot precondition with explicit `feedback_active_dev_mode` silent-degradation guard.
- §3.3 — split iter-3 citation: iter-1 + iter-2 stay in credential Strategy §6.1; iter-3 Gate 3 now cites credential Tech Spec §15.12.3 line 3689.
- §3.4 — replaced "Engine cascade (TBD — tech-lead to schedule)" row with tech-lead's concrete commitment: cluster-mode coordination cascade scheduled post-action-cascade close, queued behind credential CP6 implementation cascade.
- Status header — `DRAFT CP1` → `DRAFT CP1 (iterated 2026-04-24)`. DRAFT label retained until CP3 freeze.

### Handoffs requested — CP1

- **spec-auditor** — please audit §0–§3 for: (a) cross-section consistency; (b) every claim grounded in code/canon/ADR (target: every line-numbered citation resolves); (c) forward-reference integrity (CP2 §4 / §5 references should be marked as deferred, not dangling); (d) terminology alignment with `docs/GLOSSARY.md`; (e) confirm the §3.1 component list mirrors [`03-scope-decision.md`](../drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) §1 (no silent additions or drops).
- **tech-lead** — please review the §1 problem framing (4 distinct drift patterns) for accuracy of attribution to Phase 0 / Phase 1 evidence, and the §2 constraints list for completeness (any canon section / ADR / feedback memory that should be cited and isn't?). Solo-decider authority on §3 option ranking; please confirm the rejection rationale for B' and C' is honest acknowledgment, not dismissive. Round-2 round-trip is acceptable; CP1 lock requires tech-lead explicit ratification.
- **security-lead** — please review §2.12 must-have floor wording for non-negotiable framing (especially CR3 "hard removal, not `#[deprecated]` shim" — VETO authority on shim-form drift). §1(d) S-C2 / S-J1 attribution and §2.6 / §2.7 invariants are within your scope; flag any drift from 03c ACCEPT conditions.
