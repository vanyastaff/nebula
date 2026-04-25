---
reviewer: devops
mode: focused review (CI / migration / workspace impact slice — CP3 territory)
date: 2026-04-24
target: docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md (DRAFT CP3, §10 + §13)
parallel: rust-senior, security-lead (focused §9.5), spec-auditor
inputs:
  - docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md (§10 + §13)
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/08e-cp1-devops-review.md (T4/T5/T9 carry-forward)
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/09e-cp2-devops-review.md (nebula-redact workspace concern raised)
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/01b-workspace-audit.md (Phase 1 ground truth)
---

## Verdict

**RATIFY-WITH-NITS.** §10 codemod runbook is design-correct: T1-T6 transforms with AUTO/MANUAL split aligns with `feedback_no_shims.md` (manual sites are exactly where hard-removal discipline requires human judgment, not silent rewrite); execution model (`cargo`-style binary + idempotent + `--dry-run`) is the right shape; per-consumer step counts (§10.3) sanity-check against the real repo (engine 42 occurrences across 6 files vs §10.3 estimate of "27+ import sites + ~10 T4 sites + ~5 T2 sites + ~5-7 T5 sites" → broadly consistent). §13.4 T4/T5/T9 dispositions are correct calls per `feedback_active_dev_mode.md` — cascade-absorb T4+T9, separate housekeeping for T5 with named owner. **Three load-bearing nits** below; one is a `nebula-redact` workspace-integration gap that CP2 09e devops review explicitly flagged but CP3 §13 did not absorb.

## §10.5 codemod AUTO/MANUAL realism

**§10.5 ratio (~70% AUTO / ~30% MANUAL) is realistic and devops-coherent**, contra CP2 09e NIT 4's concern that the split was un-quantified.

Verification:

- **T1 AUTO** — `#[derive(Action)]` → `#[action(...)]` is mechanical attribute-fold; the per-consumer step count §10.3 shows zero engine T1 sites (engine doesn't impl `#[derive(Action)]`), ~3 cli sites, ~1-2 plugin sites — concentrated in test fixtures and authoring sites where mechanical rewrite has lowest semantic risk. AUTO classification correct.
- **T3 AUTO** — `Box<dyn>` → `Arc<dyn>` is a token-form rewrite. Verified via `crates/action/src/handler.rs:39-50` that `Arc<dyn>` is already canonical (per §10.2 row T3 "transform applies only to legacy `Box<dyn>` patterns if any"). Sandbox §10.3 estimates ~3 sites for T3 — this is residual cleanup, not architectural change. AUTO classification correct.
- **T4 AUTO** — HRTB collapse to `BoxFut<'a, T>` is also token-form (per Phase 1 02c §6 line 358 "8-line cut per handler"). Engine §10.3 estimates ~10 sites — high count but mechanically equivalent. AUTO classification correct.
- **T2 MANUAL** — no-key `credential::<S>()` removal — engine §10.3 estimates ~5 sites; cli ~1-2 sites. The "MANUAL REVIEW required for each call site" gating per §10.2 row T2 enforces hard-removal discipline per `feedback_no_shims.md` (auto-rewrite would silently pick the wrong slot when type inference is ambiguous). MANUAL classification correct.
- **T5 MANUAL** — `redacted_display` wrap — engine §10.3 estimates ~5-7 sites, sandbox ~2, api ~1, cli ~1. The "cannot mechanically distinguish leak-prone from safe" rationale at §10.2 row T5 is the right call; auto-wrapping all `tracing::error!(.. = %e)` would over-redact (numeric error codes wrapped unnecessarily) and is a `feedback_observability_as_completion.md`-coherent decision. MANUAL classification correct.
- **T6 MANUAL/AUTO split** — ControlAction → StatelessAction migration. AUTO for trivial pass-through; MANUAL for custom Continue/Skip/Retry reasons + Terminate interaction + test fixtures. §12.4 codemod coverage paragraph spells out exactly which patterns fall where; this is the right granularity.

**~70/30 ratio aggregate.** Counting §10.3 grand totals: T1+T3+T4 ≈ 5 + 3 + 16 = 24 AUTO sites; T2+T5+T6 ≈ 8 + 11 + 0 = 19 MANUAL sites. **24/(24+19) ≈ 56% AUTO / 44% MANUAL.** §10.5's "~70% / ~30%" is **slightly optimistic but within the band**; the headline ratio depends on whether we count by file-touches (which weights T4 mechanical sites) or by author-intervention-events (which weights T2/T5 case-by-case review). For sizing implementation effort, both axes matter; current §10.5 wording is fine but could specify *axis* (e.g., "by file-touch count: 70% / 30%; by author-intervention count: ~55% / ~45%"). **Minor wording tightening, not RATIFY-blocking.**

## §10 reverse-deps step sizing

**§10.3 step counts are realistic for sizing implementation effort**, with one verifiable cross-check using the real repo.

Verification against current repo state (`crates/{engine,sandbox,api}/src/`):

| Consumer | §10.3 sum estimate | Real-repo `nebula_action::` import count | Consistency |
|---|---|---|---|
| `nebula-engine` | ~23 sites (T1=0 + T2=5 + T3=3 + T4=10 + T5=5-7 + T6=0) | 42 occurrences across 6 files (engine.rs alone = 21) | Consistent — §10.3 is per-transform applicability, not raw import count. The "27+ import sites" total at §10.1 / Phase 0 §9 line 273 is the umbrella figure; transforms touch a subset. |
| `nebula-sandbox` | ~9 sites (T3=3 + T4=4 + T5=2) | 12 occurrences across 8 files | Consistent. |
| `nebula-api` | ~4 sites (T2=1 + T4=2 + T5=1) | 17 occurrences across 4 files | Consistent — most api occurrences are webhook plumbing that don't trigger any T1-T6 transform; api §10.3 row correctly limits to actual rewrite sites. |
| `apps/cli` | ~5 sites (T1=3 + T2=1-2 + T5=1) | (not separately counted; matches §10.1 5-files) | Consistent. |
| `nebula-sdk` | 0 transform sites; covered by §9.3 prelude reshuffle | (re-export only) | Consistent. |

**Aggregate.** §10.3 line 1697 "~55 file edits across 6 crates + 1 app... ~12-20 manual-review sites." Cross-check: 24 AUTO + 19 MANUAL = ~43 transform-events; with overlap (some files take 2+ transforms), the ~55 file-edit aggregate is plausible. **Step counts are sufficient for sizing implementation effort.** Implementation engineer can plan: ~2 days mechanical AUTO sweep + ~3-4 days manual T2/T5/T6 review + tests.

**One sizing concern.** §10.3 row `nebula-sdk` says "0 transform sites... mostly re-export changes; covered by §9.3 reshuffle, not codemod transforms." This is accurate, BUT: §9.3.3 "Reshuffled" notes 40+ SDK prelude items stay re-exported through `nebula-sdk::prelude` and the codemod transform T6 (§10.2) flags reverse-dep import sites for review when prelude paths change. **The `nebula-sdk::prelude` propagation surface is NOT counted in any §10.3 row.** The migration effort for downstream sdk consumers (any `use nebula_sdk::prelude::*;` site that touches removed/added prelude items) is implicit. **Migration guide §10.4 step 7 references it but doesn't enumerate**; if a consumer relies on now-removed `CredentialContextExt::credential` no-key form via prelude path, that's a compile error not a codemod fix. This is sizing-impact for the cascade landing PR, not a blocker. **Recommend NIT 1 below.**

## §13.4 hygiene fold-in decisions

All three §13.4 dispositions align with `feedback_active_dev_mode.md` ("before saying 'defer X', confirm the follow-up has a home"):

### §13.4.1 T4 zeroize workspace=true (cascade-scope absorb) — **CORRECT**

Verified: `crates/action/Cargo.toml:36` currently has `zeroize = { version = "1.8.2" }` inline (confirmed against repo); workspace declares `zeroize = { version = "1.8.2", features = ["std"] }` at root `Cargo.toml:106`. The inline pin silently drops `std`. §13.4.1 absorbing this into the cascade-landing PR is the active-dev-coherent call: the change is one-line, version-stable (no actual version bump — feature unification only), and the zeroize crate is **load-bearing for `SchemeGuard<'a, C>`** per credential Tech Spec §15.7 — leaving the inline pin in place during cascade landing risks crypto-dep version skew with `crates/credential` (which uses workspace-pinned zeroize) and would be `feedback_boundary_erosion.md`-territory. **Cascade-scope absorb is the right call.**

### §13.4.2 T5 lefthook out-of-scope (separate PR) — **CORRECT**

Verified: lefthook parity is workspace-wide (touches `doctests`/`msrv`/`doc` jobs across all crates, not just action) per `feedback_lefthook_mirrors_ci.md`. Bundling it into the action cascade PR would conflate scopes — exactly the mistake `feedback_active_dev_mode.md` warns against ("active dev ≠ prod release. Never settle for cosmetic / quick win"). The §13.4.2 forward-pointer ("if a community plugin author hits a CI-pre-push divergence, the housekeeping PR is the resolution path — not an action cascade re-open") correctly names the cascade-home (separate housekeeping PR) and the deadline (`feedback_lefthook_mirrors_ci.md` discipline ≤2 release cycles). This is **honest deferral with named home**, NOT silent omission. **Out-of-scope with named cascade-home is the right call.**

### §13.4.3 T9 deny.toml layer-ban (cascade-scope absorb) — **CORRECT but with §13.4.3 wording NIT**

Verified: current `deny.toml:48-81` has positive bans for `nebula-api`, `nebula-engine`, `nebula-sandbox`, `nebula-storage`, `nebula-sdk`, `nebula-plugin-sdk` — NOT `nebula-action`. CP3 §13.4.3 absorbing this is correct: action's `[dev-dependencies]` `nebula-engine` edge per CP2 §5.3-1 is exactly the situation where the missing layer-ban becomes load-bearing. **Cascade-absorb is the right call.**

**Wording NIT.** §13.4.3 line 2036-2040 uses incomplete ban-rule syntax:

```toml
{ name = "nebula-engine", wrappers = ["nebula-action-macros"] },  # action-macros dev-dep ONLY
```

But the existing `deny.toml:59-66` rule already has `nebula-engine` listed with two wrappers (`nebula-cli`, `nebula-api` for the §13 knife integration test). The §13.4.3 edit needs to be an **amendment to the existing rule** (adding `nebula-action-macros` to the `wrappers` list), NOT a new line. The current §13.4.3 toml snippet shows what looks like a parallel deny entry, which would be a duplicate-rule conflict. **Recommend NIT 2 below** — clarify the edit is a wrappers-list extension, not a new rule.

Additionally: §13.4.3 should add a **second positive ban for `nebula-action` itself**, symmetric with `nebula-engine`/`nebula-sandbox`/etc. Today `deny.toml` enforces "engine MUST NOT be depended on except by `nebula-cli` and `nebula-api`"; the redesign opportunity is to add "everything below `nebula-action` (i.e., engine/api/storage/sandbox/sdk/plugin-sdk) MUST NOT be a runtime dep of `nebula-action`." Per Phase 1 audit §11 row 9 ("missing guardrail for the redesign"), this was T9's full intent. §13.4.3 only addresses the macro-crate dev-dep edge; the **layer-ban for the action crate's runtime layer** is the bigger absent rule. **Recommend NIT 3 below.**

## nebula-redact workspace integration

**§13 does NOT cover `nebula-redact` workspace member addition or deny.toml entry.** This is a CP2-introduced concern that CP3 §13.4 silently dropped despite CP2 09e §"Workspace hygiene items T4/T5/T9" explicitly flagging it (line 213 verbatim: "T4 expansion: `nebula-redact` crate (NEW) will need a workspace-pin entry in `Cargo.toml [workspace.dependencies]` — this is a fresh-pin opportunity, not an inheritance issue").

Verification:

- `crates/redact/` does NOT exist in the current repo (confirmed via `ls crates/`).
- Workspace `Cargo.toml [workspace] members` (lines 2-37) lists 36 members; `crates/redact` is absent.
- `Cargo.toml [workspace.dependencies]` (lines 53-149) has no `nebula-redact` entry.
- `deny.toml` has no `nebula-redact` reference.
- Tech Spec §6.3.2 (line 1280-1312) **commits to creating `nebula-redact` as a NEW dedicated crate**.
- Tech Spec §11.3.2 (line 1823) names `nebula_redact::redacted_display(&e)` as a load-bearing call site for adapter responsibility table — production code, not just docs.
- Tech Spec §13.4 lists T4/T5/T9 dispositions but does NOT enumerate `nebula-redact` workspace integration as a fold-in item.

**Why this is a NIT, not a blocker.** §6.3.2 is explicit that `nebula-redact` is a NEW crate; the implementation engineer landing the cascade PR will see `crates/redact/src/lib.rs` is required and will create the workspace member. BUT: per `feedback_active_dev_mode.md` ("DoD includes... typed error + trace span + invariant check"), the cascade-landing PR's DoD must cover the workspace-add atomically. If §13 does not enumerate it, the engineer might land the action changes without the redact crate (compile error) OR land the redact crate as a separate PR (cascade-fragmentation, which is a `feedback_active_dev_mode.md` violation ("finish partial work in sibling crates")).

**The §13.4 disposition table at line 2052-2057 should add a fourth row:**

| Item | Disposition | Lands at |
|---|---|---|
| **`nebula-redact` (NEW crate creation)** | Cascade-scope absorb (preliminary — must land BEFORE action cascade PR or as part of same atomic PR) | `crates/redact/Cargo.toml` + `crates/redact/src/lib.rs` + workspace `Cargo.toml [workspace] members` + `[workspace.dependencies]` + `deny.toml` no new ban needed (it's a leaf utility) |

**Recommend NIT 4 below** — add the row.

## Required edits (if any)

**Load-bearing nits (4):**

1. **§10.3 `nebula-sdk::prelude` propagation surface enumeration.** §10.3 row says "0 transform sites... covered by §9.3 reshuffle." Add explicit cross-ref: "Reverse-deps that `use nebula_sdk::prelude::*;` and depend on now-removed prelude items (`CredentialContextExt::credential`, `credential_typed`, `CredentialGuard` legacy, `nebula_action_macros::Action` derive per §9.3.1) experience compile errors not caught by codemod. Migration guide §10.4 step 7 must reference §9.3.1 removed-items table for completeness." Sizing effort estimate for this propagation surface is currently invisible.

2. **§13.4.3 toml syntax — wrappers-list extension vs new rule.** Current §13.4.3 snippet (line 2036-2040) shows what reads as a duplicate `nebula-engine` deny entry. Clarify the edit is a **wrappers-list extension** to the existing `deny.toml:59-66` rule (adding `nebula-action-macros` to the existing wrappers `["nebula-cli", "nebula-api"]`), NOT a parallel rule. Wrong syntax causes `cargo deny check` failure.

3. **§13.4.3 missing positive ban for `nebula-action` runtime layer.** §13.4.3 only addresses the macro-crate dev-dep edge. T9's full intent per Phase 1 audit §11 row 9 is "everything below `nebula-action` MUST NOT be a runtime dep of `nebula-action`" — symmetric with existing `engine`/`sandbox`/`storage`/`sdk`/`plugin-sdk` rules. Add a second `[bans] deny =` entry: `{ crate = "nebula-action", wrappers = [...all 7 reverse-deps...], reason = "Action is business layer; engine/api/storage/sandbox/sdk/plugin-sdk are upward layers per Strategy §1.6" }`. Without this, the redesign cascade lands without the layer-ban guardrail and Phase 1 §11 row 9 stays open.

4. **§13.4 disposition table — add `nebula-redact` workspace integration row.** §13.4.4 disposition table at line 2052-2057 lists T4/T5/T9 only. CP3 §13 must explicitly fold in `nebula-redact`'s workspace-add as a cascade-scope absorb item: new workspace member at `crates/redact/`, new entry in `Cargo.toml [workspace.dependencies]` (likely `nebula-redact = { path = "crates/redact" }`), no new `deny.toml` ban needed (it's a leaf utility crate consumed by `nebula-action` only at first). Without this row, the cascade-landing engineer may land action changes that fail to compile (missing `nebula_redact` dep) OR land the redact crate as a separate PR (cascade-fragmentation per `feedback_active_dev_mode.md`).

**Non-blocking observations:**

- §10.5 ratio wording could specify axis (file-touch count vs author-intervention count) for sizing clarity. ~70/30 by file-touch is fine; ~55/45 by author-intervention is the more honest figure for effort estimation.
- §10.4 plugin author migration guide is well-structured (7 steps; <30min trivial / 2-4hr complex). The `MIGRATION.md` ships in `crates/action/` per §10.4 line 1716 — good `feedback_active_dev_mode.md` alignment (DoD includes migration guide).
- `nebula-action-codemod` host crate name + binary location are §10.2.1 forward-tracked to CP4 §15. Consistent with CP3 scope.

## Summary

§10 codemod runbook is design-correct for cascade implementation: AUTO/MANUAL split aligns with `feedback_no_shims.md` (manual sites are exactly where hard-removal needs human judgment); per-consumer step counts (engine ~23 transforms / sandbox ~9 / api ~4 / cli ~5) sanity-check against real-repo import counts (engine 42, sandbox 12, api 17 occurrences). §13.4 T4/T5/T9 dispositions are correct: T4 cascade-absorb closes the inline-pin drift risk; T5 separate-housekeeping with named owner is honest deferral; T9 cascade-absorb for the macro-crate dev-dep edge is right but **needs syntax fix and a second positive ban** for the full action-layer guardrail. **Top two CI/migration concerns: (1) `nebula-redact` workspace member integration is missing from §13.4 despite being a NEW crate per §6.3.2 — the cascade-landing PR would compile-fail without it; (2) §13.4.3 toml syntax shows a parallel deny rule where it should be a wrappers-list extension to the existing `nebula-engine` rule, plus the §13.4.3 lacks the positive ban on `nebula-action` runtime-layer upward edges that Phase 1 §11 row 9 actually flagged.**
