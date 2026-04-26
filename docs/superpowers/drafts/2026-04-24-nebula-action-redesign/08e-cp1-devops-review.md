---
reviewer: devops
mode: parallel review (CI / migration / workspace impact slice)
date: 2026-04-24
target: docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md (DRAFT CP1, §0-§3)
parallel: rust-senior (signature review), security-lead (security-floor invariants), spec-auditor (structural)
inputs:
  - docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/01b-workspace-audit.md (Phase 1)
  - docs/superpowers/specs/2026-04-24-action-redesign-strategy.md §4.3
---

## Verdict

**RATIFY-WITH-NITS.** §2.7.1 wire-end-to-end pick is the principled call and resolves T2 (the dead `unstable-retry-scheduler = []` empty flag); §2.7.2 introduces a *new* feature name (`unstable-action-scheduler`) into the signature freeze and §0.2 binds the freeze around §2 — that creates a CP1↔CP3 boundary nit (§2.7.1 binds principle, §2.7.2 names a flag whose granularity is CP3-locked). §3 outlines runtime model but does not enumerate engine migration impact (the 27+ engine import sites from Phase 0 §8); that's a documentation gap, not a design gap. §1.2 N7 lists workspace hygiene cascade-homes correctly but T1/T2/T4/T5/T9 from Phase 1 audit are NOT explicitly absorbed — they map to CP2/CP3, not CP1. Three nits below; one is load-bearing for the freeze surface.

## §2.7 feature flag CI impact

### T2 resolution — yes, but partial

Tech Spec §2.7.1 picks **wire-end-to-end**. This is exactly what T2 needed. Phase 1 audit row 3 (`01b-workspace-audit.md` §11): "`unstable-retry-scheduler = []` is a dead empty feature... `cargo check --all-features` turns it on every run... documented drift." The CP1 decision graduates the feature from documentary-only to wired-with-cascade-scope-scheduler. **T2 dead-flag closes when CP3 §9 lands the wiring**, not at CP1 lock — but the principle is now bound, so T2 is **on track to close**, not closed. CP1 should not claim closure.

### CI impact — `cargo check --all-features` (`ci.yml:109`)

Today: `unstable-retry-scheduler = []` is empty; `--all-features` activates a no-op gate. Type-side, `ActionResult::Retry` un-hides; engine path `is_retry()` predicate is always-available so feature unification is correctness-safe (per `crates/engine/Cargo.toml:11-21` comment).

Post-§2.7.1: both `Retry` (existing flag) and `Terminate` (new — `unstable-action-scheduler` per §2.7.2) are `#[cfg(feature = ...)]`-gated **on the type side**. CI's `cargo check --workspace --all-features --all-targets` will activate both; engine forwards `unstable-retry-scheduler` already (`crates/engine/Cargo.toml:21`). Engine will need to forward the second flag too once CP3 picks granularity (unified vs split). **CI gating mechanism stays unchanged**; only the feature-name set grows.

**Concern (NIT 1)**: §2.7.2 emits the variant inside `#[cfg(feature = "unstable-action-scheduler")]` but §2.7-1 explicitly defers the granularity decision to CP3 §9 (unified vs split). The signature block in §2.7.2 is **freeze-grade Rust** per §2 ("This is the **signature-locking section**... Tech Spec ratification freezes these signatures"), and §0.2 invariant 4 says spike-shape divergence invalidates the freeze. If CP3 picks split (`unstable-terminate-scheduler`), the §2.7.2 `#[cfg]` literal changes — that's a §0.2 freeze-invalidating edit. **Recommend**: §2.7.2 should use a placeholder/sentinel comment `<terminate-feature-flag>` with explicit "name TBD CP3 §9" annotation, OR pin one name now (and treat CP3 §9 as a binding decision with no refactor cost). Today's signature pretends to be frozen but §2.7-1 itself says it's not.

### Both flags activating cleanly under `--all-features`

Yes — feature unification semantics: `cargo check --all-features` enables every declared feature on every workspace member. If `nebula-action` declares both `unstable-retry-scheduler` and `unstable-action-scheduler` (or `unstable-terminate-scheduler`), both `#[cfg]` variants compile. Engine forwarding (whichever shape CP3 picks) just needs a parallel forwarder for the new flag. **No CI compile-side surprise expected** — but if CP3 picks **unified `unstable-action-scheduler`**, `nebula-engine/Cargo.toml:21` needs to renamed-rather-than-added (semver consideration, advisory-only at alpha per Phase 0 §6 — not a CI gate today).

## Macro harness CP2 readiness check

CP1 trait contract creates **no blockers** for trybuild/macrotest harness. Specifically:

- §2.1 `Action: ActionSlots + Send + Sync + 'static` supertrait — does not constrain dyn-safety of macro-emitted impls; the macro emits `impl ActionSlots for X` + `impl StatelessAction for X` + a blanket `impl Action for X` (per ADR-0039 §2 line 67-70). Compile-fail tests can target rejection paths (`credential = "string"`, non-unit struct, missing `Source`/`Resource`) without hitting dyn-safety at all.
- §2.4 `*Handler` traits use `BoxFut<'a, T>` (which is `Pin<Box<dyn Future + Send + 'a>>` per §2.3) — these are **dyn-safe** (rust-senior 02c §6 line 358 cited). A `cargo expand` snapshot test (macrotest) on a `#[action]`-decorated struct doesn't need to instantiate `Arc<dyn StatelessHandler>`; it just compares the emitted token tree to a fixture. **No conflict between RPITIT primary traits and dyn-safe handler companions** for snapshot purposes — they live in separate token-emission scopes.
- §2.6 sealed-DX trait pattern (per-capability inner sealed module) — sealing is a **public-trait surface concern**, not a macro-emission concern. The macro emits `impl ControlAction for X` only when the user writes `#[action(control)]` (or equivalent CP3 attribute); the inner sealed-trait impl is crate-internal. trybuild compile-fail probes for "implementing `ControlAction` outside the crate fails" become **standard sealed-trait probes** — no design-side blocker.

**Workspace-side prerequisite (NIT 2, soft)**: `nebula-action-macros/Cargo.toml` (`crates/action/macros/Cargo.toml`, currently 28 lines) **has no `[dev-dependencies]` block at all** (Phase 1 audit §2). CP2 §5 will need to add one with `trybuild` + `macrotest` + likely `nebula-action` (path-dep) for the rejected-input probes to compile against the real attribute parser. `trybuild` is **not workspace-pinned** today — Phase 1 only `nebula-validator/Cargo.toml:46` (`trybuild = "1"`) and `nebula-schema` use it inline. CP2 §5 design should decide: workspace-pin trybuild (consistent with the audit hygiene direction) or per-crate inline (matches existing pattern). Either is fine; surfacing the choice now prevents CP2 from being blocked on a workspace-deps decision.

CP1 §2 trait shape: **CP2 macro harness has a clean target**. Recommend CP2 §5 designers cite this paragraph rather than re-deriving.

## §3 engine integration migration impact

§3 describes the **runtime composition** (slot registration → HRTB dispatch → resolve helper → SchemeGuard) but **does not quantify engine-side migration impact**. Phase 0 §8 (referenced in user prompt) and Phase 1 audit §9 both flag: **27+ engine import sites** binding `ActionHandler`, `ActionResult`, `ActionMetadata`, `ActionError`, `PortKey`, `TerminationCode` — and the resolve helpers per §3.3 ("live in `nebula-engine`, not `nebula-action`") **add net-new engine surface** rather than touching existing imports.

Specific migration-impact items §3 raises but does NOT enumerate:

1. **§3.1 `Map<ActionKey, &'static [SlotBinding]>` registry** — "Engine maintains a registry-time map populated when `ActionRegistry::register*` is invoked" (§3.1 second paragraph). Concrete location is "`crates/runtime/src/registry.rs`, exact line range CP3 §7 scope". **Concern: there is no `crates/runtime/`** — Phase 1 audit §6 row 4 documents this as a 🟠 MAJOR finding (`test-matrix.yml:66` lists dead `nebula-runtime`; `.github/CODEOWNERS:52` references `/crates/runtime/`). The registry actually lives in `nebula-engine` per Phase 1 audit §9. §3.1 should cite `crates/engine/src/registry.rs` (or wherever it actually is) rather than the dead path. **Not CP1's bug per se** — it inherits the broken Phase 0 reference — but the citation is wrong and freezing it forwards a broken reference into Tech Spec. **NIT 3, load-bearing for citation hygiene**: §3.1 should use `nebula-engine` as the host crate (or explicitly mark "host crate name TBD CP3 §7 scope" if the engine/runtime split is itself open).

2. **§3.3 resolve helpers** — three new pub fns in `nebula-engine` (`resolve_as_bearer`/`_basic`/`_oauth2`) with `where C: Credential<Scheme = X>` bounds. These are **net-new public surface in engine** even if used only via `SlotBinding::resolve_fn`. semver-checks.yml is advisory-only at alpha (Phase 0 §6) so no CI block. But this should appear in CP3 §9 engine-migration scope.

3. **§3.2 dispatch path step 6** — "`&'a SchemeGuard<'a, C>` is exposed to the action body via the ActionContext API. Action body calls `Deref` (`&BearerScheme` directly..." — this changes the **`ActionContext` public surface** of `nebula-action`, which is re-exported by `nebula-sdk::prelude` (Phase 1 audit §9: "SDK prelude re-exports a very wide slice of nebula-action... ~40+ types"). Tech Spec §3 does not flag the prelude propagation; CP3 §7 needs to absorb it.

§3 is **design-correct** for the runtime model; the migration scope (engine import-site delta + sdk-prelude propagation) is left implicit. CP3 §9 will need to enumerate. **Not RATIFY-blocking** because Strategy §4.3 + Phase 1 audit already catalogue the touchpoints; CP1's job is the type contract, not the migration plan.

## Workspace hygiene placement

Phase 1 audit raised T1/T2/T4/T5/T9 (the user prompt names these as "absorbed in cascade scope per Strategy §1.8"). Strategy §1.8 absorption claim verified against CP1 §1 / §1.2:

| Phase 1 finding | CP1 placement | Status |
|---|---|---|
| T1 — no macro harness (trybuild/macrotest in `nebula-action-macros`) | G2 explicitly: "`crates/action/macros/Cargo.toml` gains `[dev-dependencies]` block with `trybuild` + `macrotest`" (line 62) | **In CP1 G2 scope ✅** |
| T2 — `unstable-retry-scheduler = []` dead empty flag | G6 + §2.7.1 wire-end-to-end pick (line 77 + line 326-339) | **In CP1 G6 + §2.7.1 scope ✅** (closes when CP3 §9 wires) |
| T4 — `zeroize = "1.8.2"` inline pin instead of workspace | **NOT mentioned in CP1.** §2.7.2 / §3.4 use zeroize; G3 floor item 4 references zeroize. No goal/non-goal binds it. | **Implicit OUT-of-scope at CP1**, no cascade home named |
| T5 — lefthook does not mirror `doctests/msrv/doc` CI jobs | **NOT mentioned in CP1.** Out of action-trait scope. | **Implicit OUT-of-scope**, no cascade home named |
| T9 — no `deny.toml` layer-ban for `nebula-action` | **NOT mentioned in CP1** as a §1 Goal/Non-goal item. Phase 1 audit row 9 flagged "missing guardrail for the redesign." | **Implicit OUT-of-scope at CP1** |

**Finding**: T1 and T2 are correctly absorbed. **T4, T5, T9 have no cascade home named in Tech Spec**. Per `feedback_active_dev_mode.md` ("before saying 'defer X', confirm the follow-up has a home") and CP1 §1.2 framing ("each non-goal cites the Strategy §3.4 OUT row... This is **honest deferral** per `feedback_active_dev_mode.md`"), these three items violate the same rule.

That said: T4 (zeroize inline pin), T5 (lefthook doctest gap), T9 (deny.toml layer-ban for action) are **workspace-hygiene items that were not raised through Strategy §3.4** because Strategy framed scope around the trait contract, not the workspace audit. Phase 1 audit was the venue that surfaced them. Tech Spec CP1 inherits Strategy's scope framing.

**Recommendation (NIT, soft)**: CP1 §1.2 N7 currently says "Sub-spec out-of-scope rows from Strategy §2.12... are all deferred-with-cascade-home." Add a closing sentence: "Workspace hygiene findings T4 (zeroize inline pin), T5 (lefthook doctest mirror gap), T9 (deny.toml layer-ban for `nebula-action`) from `01b-workspace-audit.md` §11 rows 5/6/9 are NOT in cascade scope per Strategy §3.4 framing; cascade-home is workspace hygiene sweep, deadline TBD." This is **honest deferral with named home**, not silent omission. Optional CP1 iteration; mandatory if the active-dev rule binds at Tech Spec ratification.

## Required edits (if any)

**Load-bearing nits (3):**

1. **§2.7.2 `#[cfg]` literal vs §2.7-1 deferred granularity** — pick a placeholder annotation OR commit one name now. Current state freezes a feature-name signature whose name §2.7-1 says is unfrozen.
2. **§3.1 `crates/runtime/src/registry.rs` citation** — `crates/runtime/` does not exist; either say `nebula-engine` (likely correct) or mark "host crate name TBD CP3 §7 scope".
3. **§1.2 N7 workspace hygiene completion** — add closing sentence naming T4/T5/T9 deferral and cascade-home (or explicitly admit they are out-of-cascade-scope).

**Non-blocking observations:**

- §3 should cite Phase 0 §8 / Phase 1 §9 27+ engine import sites as migration-touchpoint inventory pointer, not enumeration.
- CP2 §5 macro harness designer should cite this review §"Macro harness CP2 readiness check" to confirm the trait-shape gate is open.
- T1 and T2 absorption is accurate; CP1 should not claim closure of T2 (closes at CP3 §9 wiring landing).

## Summary

§2.7.1 picks the principled wire-end-to-end path; T2 dead-flag absorbs cleanly. CI feature-unification semantics are unchanged; both new flag names will activate under `--all-features` once CP3 picks granularity. CP1 trait contract creates no dyn-safety blockers for trybuild/macrotest CP2 harness — `BoxFut<'a, T>` companions and primary RPITIT shapes coexist cleanly in macro snapshot scope. §3 runtime model is design-correct but does not enumerate the 27+ engine import-site migration delta or sdk-prelude propagation — both belong to CP3 §9. Top two CI/migration concerns: (1) §2.7.2 freezes a feature-name `unstable-action-scheduler` whose granularity §2.7-1 explicitly defers to CP3 — pick a name or use a placeholder; (2) §3.1 cites `crates/runtime/src/registry.rs` which does not exist (Phase 1 audit row 4) — fix to `nebula-engine` or annotate TBD.
