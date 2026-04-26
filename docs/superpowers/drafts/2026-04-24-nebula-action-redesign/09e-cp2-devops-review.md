---
reviewer: devops
mode: parallel review (CI / dep / migration / workspace impact slice)
date: 2026-04-24
target: docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md (DRAFT CP2, §4-§6)
parallel: rust-senior (signature review), security-lead (security-floor invariants), spec-auditor (structural)
inputs:
  - docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md (§4-§6)
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/08e-cp1-devops-review.md (CP1 review)
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/01b-workspace-audit.md (Phase 1)
re-verified-against:
  - crates/action/macros/Cargo.toml (28 lines, no [dev-dependencies] — confirmed)
  - crates/action/Cargo.toml (zeroize inline pin still live at line 36 — confirmed)
  - crates/engine/Cargo.toml lines 11-22 (forwards `unstable-retry-scheduler` only; `unstable-terminate-scheduler` not yet wired)
  - Cargo.toml [workspace.dependencies] lines 53-145 (no trybuild / macrotest workspace pin)
  - deny.toml lines 41-81 (no layer-ban for nebula-action OR nebula-redact yet)
  - .github/workflows/test-matrix.yml lines 60-66 (FULL list does NOT contain action-macros — covered by `nebula-action` shard via -p workspace member; macros sub-crate has its own package name `nebula-action-macros`)
  - crates.io: trybuild max_version = 1.0.116 (spec proposes 1.0.99 — 17 patches stale); macrotest max_version = 1.2.1 (spec proposes 1.0.13 — minor-line stale, API drift risk)
  - existing trybuild use sites: crates/schema/Cargo.toml:40 (`trybuild = "1"`), crates/validator/Cargo.toml:46 (`trybuild = "1"`) — both inline, both float on minor; macrotest is NOT used anywhere in the workspace today
  - crates/redact/ does NOT exist (confirmed via crates/ ls — 24 dirs, no `redact`)
---

## Verdict

**RATIFY-WITH-NITS.** §5.1 closes T1 (the macro-harness gap I flagged in 08e §"Workspace-side prerequisite") with a real `[dev-dependencies]` block; §5.2 layout is correct and §5.3's seven-probe table covers the spike's six probes plus the `parameters = Type` Probe 7 needed to fix the §4.6.1 silent-drop bug. §6.3 introduces `nebula-redact` as a NEW workspace crate with sound rationale (single audit point, layering, review surface). §2.7-1 is **closed by amendment** at line 438 (parallel-flag `unstable-retry-scheduler` + `unstable-terminate-scheduler` committed) — this resolves my CP1 NIT 1 about freeze-vs-deferred. Two load-bearing nits below are dep-pin currency (the proposed pins are stale at the major-API line for `macrotest`) and one workspace-hygiene gap that CP2 inherits but does not close. None blocks ratification; all are CP2 inline-edit scope.

---

## §5.1 dev-deps pinning + workspace discipline

**The block:**
```toml
[dev-dependencies]
trybuild = "1.0.99"        # spec proposal
macrotest = "1.0.13"       # spec proposal
nebula-action = { path = ".." }                     # §5.3
nebula-credential = { path = "../../credential" }   # §5.3
nebula-engine = { path = "../../engine" }           # §5.3
```

### Pinning currency (NIT 1, load-bearing)

`crates.io` ground truth as of 2026-04-24:

| Dep | Spec proposes | crates.io max | Delta | API risk |
|---|---|---|---|---|
| `trybuild` | `1.0.99` | `1.0.116` | 17 patch versions | None — caret-`^1` resolves forward; same major.minor surface |
| `macrotest` | `1.0.13` | `1.2.1` | **Two minor versions ahead** | **Real** — `macrotest 1.2.x` may have moved or renamed `expand_args` API that §5.5 line 1050 names verbatim |

**`trybuild` issue is cosmetic** — `"1.0.99"` resolves to `1.0.116` under semver caret and the API is stable; no code-side break. Cosmetic point only: per `feedback_idiom_currency.md`, citing `1.0.99` with "latest stable as of cascade close" is **factually wrong** — the latest stable is 1.0.116 at this commit. Recommend bump the citation OR drop the version specificity (`trybuild = "1"` matches what `nebula-schema` and `nebula-validator` already do — see "Workspace discipline" below).

**`macrotest` issue is load-bearing.** `1.0.13` is on the `1.0.x` line; the current major is `1.2.1`. The spec's §5.5 cite "macrotest::expand_args per macrotest 1.0.13 API" needs validation against what `1.2.x` exposes — if `expand_args` was renamed or its signature changed, the snapshot driver will break. Concrete action: pin `macrotest = "1.2"` (caret-resolves to 1.2.1) and verify the §5.5 snippet against the 1.2.x docs at impl time. If 1.0.x and 1.2.x differ enough that the snippet must change, the spec sentence at line 1050 needs an inline fix.

### Workspace pin vs inline pin (NIT 2, soft — CP1 already flagged)

§5.1 line 946 says: "**Workspace inheritance (`trybuild = { workspace = true }`) is *not* preferred here** — the macro test harness is the only consumer; localizing the pin to `crates/action/macros/Cargo.toml` keeps the dependency surface narrow per `feedback_boundary_erosion.md`."

This **contradicts** the existing inline `trybuild = "1"` at `crates/schema/Cargo.toml:40` and `crates/validator/Cargo.toml:46` — there are already three trybuild consumers (schema, validator, action-macros after CP2). "Macro test harness is the only consumer" is wrong by two existing call sites. Per Phase 1 audit §2 finding 3 ("workspace-level pin would be more maintainable"), the principled call is to **promote `trybuild` to workspace dep at this CP2 cascade close** — three consumers make it worth promoting; the audit already named this. Same applies if a future macro-bearing crate (credential/macros, plugin/macros, etc.) wants compile-fail probes.

**Recommendation:** §5.1 should either (a) commit to workspace promotion of `trybuild` (consistent with audit direction), or (b) acknowledge the three-consumer count and explicitly defer promotion to a workspace-hygiene sweep (Phase 1 audit T-style cascade item with named home). Today's wording cites `feedback_boundary_erosion.md` as reason to localize — that rule is about cross-crate misplacement, not about avoiding workspace deps for shared dev-tools. Inline pin was the right CP1 stance (one consumer); two more consumers exist already, and CP2 adds a third, so workspace pin is the now-correct call.

`macrotest` is the only consumer at this point (no other crate uses it), so the localized inline pin for `macrotest` is consistent with the rule even after promoting `trybuild`.

### Path-deps `nebula-action`, `nebula-credential`, `nebula-engine` as dev-deps (Open item §5.3-1)

The §5.3 block adds three internal path-deps as `[dev-dependencies]` on `nebula-action-macros`. Layering check:

- Production path: `nebula-action-macros` (proc-macro) is depended-on by `nebula-action` only. No upward edge.
- Dev path: dev-deps are **additive at link time**. They participate in cargo's feature-unification graph for `cargo check --workspace --all-features` but do NOT create a cyclic compile edge unless someone takes a code dep on the dev-deps from non-test code.

`nebula-action` → (dep) → `nebula-action-macros` → (dev-dep) → `nebula-action` would be a cycle if cargo resolved dev-deps as compile edges — but it does not (dev-deps only participate in the test-target build of the owning crate). **Cargo handles this correctly**: when building `nebula-action`'s tests, cargo builds `nebula-action-macros` without its dev-deps; when building `nebula-action-macros`'s own tests, cargo pulls the path-dep `nebula-action`. No cycle.

**Verdict on §5.3-1:** acceptable. The pattern (proc-macro crate depending on its parent for compile-fixtures) is standard in proc-macro testing — `serde_derive` does it, `thiserror-impl` does it, etc. No `feedback_boundary_erosion.md` violation.

**Caveat (NIT 3, soft):** the dev-dep on `nebula-engine` is the unusual edge. `nebula-engine` is the exec-layer host and pulls in the entire dispatcher subtree (registry, runtime, scheme registry, etc.) — adding it as a dev-dep on a proc-macro crate makes `cargo nextest run -p nebula-action-macros` build the entire engine subtree. That is a non-trivial test-cycle cost (engine build is ~30-60 sec cold). Alternative: spike Probe 6 (the wrong-Scheme `resolve_as_bearer::<BasicCred>` probe) could ship a **fixture stub** in the test crate that replicates the function-pointer shape (`for<'ctx> fn(...) -> BoxFuture<'ctx, ...>`) without dragging in the real engine. Spike used the real helper; production should consider whether the test-isolation cost justifies the build-time saving.

§5.3 line 1000 already flags this as Open Item §5.3-1. Recommend rust-senior CP2 review weighs in on whether a stub or the real helper is preferred. CI-side concern is build-time inflation on every `nebula-action-macros` run.

---

## §5.2 harness CI integration

### Layout (line 952-971)

The `tests/compile_fail/` + `tests/expansion/` split is the standard trybuild + macrotest layout. Filenames `probe_N_<reason>.rs` + `.stderr` paired files match trybuild's directory-driven scan idiom. CP2 commits `compile_fail.rs` + `expansion.rs` as the test drivers (each runs the directory). **Layout is correct.**

### CI shard scoping

**Test-matrix shape today** (`test-matrix.yml:66`): the FULL list does NOT contain `nebula-action-macros` as a separate shard. The matrix iterates workspace member packages by name; `nebula-action-macros` is a member crate (per workspace `Cargo.toml:29`) and would only run via the diff-scoped path (line 99-108) IF a PR touches `crates/action/macros/`. On full-matrix runs (push/workflow_dispatch), the FULL list at line 66 must be amended to include `"nebula-action-macros"` for the new harness to execute.

**This is a CI workflow change** — explicitly out of CP2 design scope per the user prompt ("Do not propose CI workflow changes"). Flagging as **CP3 §9 inheritance**: the test-matrix.yml FULL list will need `"nebula-action-macros"` added. Without that edit, the new compile-fail / expansion tests run only on PRs that diff into `crates/action/macros/**` AND on local `cargo nextest run -p nebula-action-macros`. That is sufficient for landing the harness; the full-matrix gate is a CP3 polish item.

**Recommendation for §5.2:** add a sentence at line 973 noting that test-matrix.yml FULL list will need `"nebula-action-macros"` appended — flagged for CP3 §9 cascade, not enacted by CP2. This is the kind of cascade-home naming `feedback_active_dev_mode.md` requires.

### Macrotest profile

§5.2 line 1050 says CI runs `cargo nextest run -p nebula-action-macros --profile ci`. Verified — this matches `test-matrix.yml:176` generic shard form. Profile `ci` exists (per `.config/nextest.toml` if present; would not change behavior for a fresh shard).

---

## §6.3 nebula-redact crate scope

### Decision rationale

§6.3.2 commits **NEW dedicated `nebula-redact` crate** (not co-resident in `nebula-log`). Three-bullet rationale (line 1170-1173) is sound:

1. **Single audit point** — security-lead 08c §Gap 3 explicitly preferred this; this is the architect-level call security-lead pre-cleared.
2. **Layering** — `nebula-error::Display` cannot depend on `nebula-log` (inverted dep — `nebula-log` is a logging facade, errors are core). Putting redact in `nebula-log` would force this inversion. Putting it in a standalone crate puts it cleanly at the bottom of the dep stack — `nebula-error` and `nebula-action` can both depend on it.
3. **Review surface** — standalone crate gets its own `cargo doc`, CHANGELOG, codeowner. Aligns with security-lead's audit-discipline direction.

**My add: layer placement.** `nebula-redact` should be a **leaf** crate (no deps on other Nebula crates except possibly `nebula-error` for error types if any). It exposes one function: `redacted_display<T: Display>(&T) -> String`. Layer-wise it belongs with `nebula-core`, `nebula-error`, `nebula-log` at the foundation. Phase 1 audit §3 finding "nebula-action is not in deny.toml layer-ban list" applies: when adding `nebula-redact`, the CP3 §9 designer should also add it to deny.toml's allowed-consumer layer (it's likely consumable by ANYONE — error, action, audit, observability, etc. — so a permissive layer entry).

### Workspace member addition

CP2 §6.3.2 commits the crate stub but does NOT name the workspace `Cargo.toml [workspace.members]` addition. Required at impl time:

```toml
# Cargo.toml [workspace.members] line 2-37 — add:
"crates/redact",
```

This is mechanically trivial and explicitly out of CP2 design scope per "Do not propose CI workflow changes" (workspace.toml is not a CI workflow but is configuration). Flag for CP3 §9 to enumerate the workspace-member edit alongside the crate creation.

### Deny.toml impact

`deny.toml` currently has zero layer-ban entries for `nebula-action` and zero for the (NEW) `nebula-redact`. Per Phase 1 audit row 9 ("missing guardrail for the redesign"), CP3 §9 should add `nebula-redact` permissively (any consumer allowed) but should also lock its **outbound** dep set — a redaction-rule crate should NOT pull in heavy deps. Reasonable scope:

- Direct deps: `regex` (workspace, line 95) for pattern stripping; OR pure-string-matching for simpler scope.
- No `nebula-log`, no `nebula-error` direct (avoid the cycle).
- Optional `tracing` for span-level emission (low risk).

**Concern:** §6.3.2 line 1186-1187 says "(impl details: CP3 §9)" — including which crates `nebula-redact` pulls in. The rust-senior or architect should weigh in on whether `regex`-based redaction or pure-substring redaction. `regex` is workspace-pinned at `1.12` (line 95) and already a transitive dep across the workspace, so adding it at the redact crate is feature-unification-safe. Defer scope question to CP3 §9.

### CI matrix add

When the crate lands, `test-matrix.yml:66` FULL list will need `"nebula-redact"` appended (same edit pattern as the `nebula-action-macros` add). Out of CP2 design scope. Reasonable scope, easy edit, no design risk.

### Reasonable scope verdict

**Reasonable.** Single-function crate with a narrow contract (`redacted_display<T: Display>(&T) -> String`), one transitive dep (probably `regex`), single audit point. No `feedback_boundary_erosion.md` violation — this is the **opposite**: extracting a security-critical helper into its own auditable surface, not stuffing it into an already-busy crate.

---

## §4 feature flag CI impact

### §2.7.2 freeze closure (resolves CP1 NIT 1)

Tech Spec line 438 says verbatim: "**Decision (CP1) on feature-flag granularity** — committed: **parallel flags** `unstable-retry-scheduler` + `unstable-terminate-scheduler`. ... CP3 §9 may amend the *internal scheduler implementation* but cannot rename or unify the public flags without an ADR amendment."

This is the CP1-side amendment that closes my CP1 NIT 1 (08e line 83). The §0.2 invariant 4 ("spike-shape divergence invalidates the freeze") now binds the parallel-flag names; the CP3 §9 design has no degree of freedom on flag names without an ADR amendment. **Closed.**

### `cargo check --all-features` impact (CP1 R8 carry-forward)

The CP1 review (08e §"§2.7 feature flag CI impact") established the type-side semantics: `--all-features` activates both flags; `is_retry()` predicate is feature-unification-safe. CP2 §4 adds the **macro-side** dimension — the new question is whether `#[action]` macro emission emits any **`#[cfg(feature = ...)]`** attributes that depend on these flags.

**Verified from §4 emission contract** (line 800-820, §4.3):
- Macro emits `impl ActionSlots for X` with `&'static [SlotBinding]` slice — no feature gating.
- Macro emits `impl Action for X` with metadata literal — no feature gating.
- Macro emits primary-trait impl (`StatelessAction` etc.) — no feature gating.

The macro emission is **feature-flag-agnostic**. The `unstable-retry-scheduler` / `unstable-terminate-scheduler` gates live entirely on the **`ActionResult` enum variants**, not on macro emission. This means:

1. `cargo expand` snapshots in §5.5 are **stable** under `--all-features` flip — the expansion does not change shape based on feature activation.
2. `cargo check --all-features --all-targets` activates both flags on the type side; macro snapshots remain identical.
3. **No new CI surprise from CP2 macro emission.** Feature unification semantics carry forward unchanged from CP1.

**This is a positive finding.** The macro contract being feature-agnostic means macrotest snapshot fixtures will not fragment by feature combination. One snapshot per fixture; not 4-snapshot-per-fixture (`{retry, !retry} × {terminate, !terminate}`). Saves a 4x snapshot-count multiplier and corresponding CI time.

### `unstable-terminate-scheduler` engine forwarding (CP3 inheritance)

`crates/engine/Cargo.toml:21` currently declares only `unstable-retry-scheduler = ["nebula-action/unstable-retry-scheduler"]`. CP3 §9 will need to add the parallel forwarder:

```toml
unstable-terminate-scheduler = ["nebula-action/unstable-terminate-scheduler"]
```

Plus the analogous comment-block explanation (lines 13-20 today) extended to `Terminate`. This is mechanical; not a design risk. Out of CP2 scope.

---

## Migration prep for CP3 readiness

### 7 reverse-deps inheritance from Phase 1 audit

Phase 1 audit §9 enumerated 7 direct reverse-deps: `nebula-engine` (heaviest, 27+ import sites), `nebula-api`, `nebula-sandbox`, `nebula-sdk` (40+ prelude re-exports), `nebula-plugin`, `apps/cli`, plus the action-macros sibling. CP2 §6.2 commits **hard removal** of `CredentialContextExt::credential<S>()` no-key heuristic (line 1112-1118) — this is a **breaking-change cascade** across 7 reverse-deps.

CP2 §6.2.4 line 1136-1140 names the codemod scope:
- **Auto-rewritable**: explicit-type-annotation call sites with known-concrete credential type.
- **Manual-review marker**: type-erased / unknown-type call sites.

CP3 §9 inherits the codemod runbook. **CP2 §6.2 is sufficiently flagged** for CP3 — the hard-removal contract is locked, the codemod fork-points are named, the runbook author has an unambiguous spec. Per `feedback_active_dev_mode.md` ("before saying 'defer X', confirm the follow-up has a home"), the cascade home is named: CP3 §9 codemod runbook.

**Concern (NIT 4, soft):** §6.2.4 names the auto-rewrite path but does NOT enumerate which reverse-deps' call sites fall into "auto" vs "manual". Phase 1 audit §9 has the call-site fingerprint:
- `nebula-engine`: 27+ uses of `ctx.credential::<S>()` likely auto-rewrite (engine knows its credential types).
- `nebula-sandbox`: dyn-handler shapes — likely manual-review (type-erasure).
- `nebula-api`: webhook domain — ctx.credential calls are rare; likely auto-rewrite.
- `apps/cli`: dev surface — likely auto-rewrite.
- `nebula-sdk`: re-exports the API; depends on user code, can't pre-classify.
- `nebula-plugin`, action-macros: minimal call sites.

CP3 §9 should enumerate this concretely (per-crate auto-vs-manual count); CP2 §6.2 leaves it implicit. Acceptable for CP2 — concrete enumeration is implementation-time work — but flagging that the codemod author needs the per-crate breakdown to size effort.

### Open item §4.7-1 (codemod inference success rate)

Line 928 — "Inference success rate for codemod auto-rewrite needs measurement. ... CP3 §9 quantifies the inference success rate against the 7 reverse-deps before committing to auto-rewrite vs manual-marker default."

This is the **right deferral** — measurement needs the codemod prototype, which is CP3 implementation work. Per `feedback_active_dev_mode.md`, the cascade home is named (CP3 §9), and the measurement target is named (against the 7 reverse-deps). No NIT.

### Workspace hygiene items T4 / T5 / T9 (carry-over from CP1 NIT 3)

CP1 review 08e §"Workspace hygiene placement" flagged that T4 (zeroize inline pin), T5 (lefthook doctest mirror gap), T9 (deny.toml layer-ban for nebula-action) had no cascade home named. CP2 does not address these. CP2 ADDS a related concern:

- **T4 expansion**: `nebula-redact` crate (NEW) will need a workspace-pin entry in `Cargo.toml [workspace.dependencies]` — this is a fresh-pin opportunity, not an inheritance issue.
- **T9 expansion**: `nebula-redact` (NEW) joins the list of crates that need an allowed-consumer entry in `deny.toml` `[bans]`.

CP2 §1 / §6 do not surface T4/T5/T9 by name. Per `feedback_active_dev_mode.md`, this remains an **un-homed deferral**. The right move is for CP4 (final ratify) to either pick these up or explicitly document them as out-of-cascade-scope. Not a CP2 ratification blocker, but the deferral chain accumulates.

---

## Required edits (if any)

**Load-bearing nits (4):**

1. **§5.1 dep pin currency — `macrotest = "1.0.13"` is two minor versions stale.** Bump proposal to `macrotest = "1.2"` (caret-resolves to 1.2.1) and verify the §5.5 `expand_args` API citation against 1.2.x docs. If the API renamed, fix the citation inline.
2. **§5.1 workspace-vs-inline pin discipline contradicts existing repo pattern.** `trybuild` has 3 consumers after CP2 (schema, validator, action-macros); the "only consumer = no workspace pin" rationale at line 946 is factually wrong. Either (a) commit to workspace promotion of `trybuild`, or (b) acknowledge the count and defer to a workspace-hygiene sweep with named home.
3. **§5.2 test-matrix shard implication unflagged.** `test-matrix.yml:66` FULL list does not include `"nebula-action-macros"`; CP3 §9 will need to add. Add a flag-line at end of §5.2 naming this CP3 inheritance.
4. **§5.3-1 `nebula-engine` dev-dep on macros crate has build-time cost.** Rust-senior CP2 review should weigh fixture-stub vs real-helper for Probe 6. CI-side: every macros-crate test run pulls in the engine subtree. Acceptable but flag for review.

**Non-blocking observations:**

- **§2.7-1 closure verified at line 438** — parallel flags `unstable-retry-scheduler` + `unstable-terminate-scheduler` committed; resolves CP1 08e NIT 1. Engine forwarding for the Terminate flag is CP3 §9 inheritance (one-line add to `crates/engine/Cargo.toml:21`).
- **§4 macro emission is feature-flag-agnostic** — positive finding. Snapshot fixtures don't fragment by feature combination; saves 4x snapshot-count multiplier in CI time.
- **§6.3 `nebula-redact` scope is reasonable** — single-function crate, narrow surface, single audit point per security-lead 08c. Workspace-member add + deny.toml entry are CP3 §9 inheritance, not CP2 design defects.
- **§6.2.4 codemod fork-points are named** but per-crate auto-vs-manual breakdown is implicit. CP3 §9 needs the breakdown to size effort. Not a CP2 defect, just a CP3 prompt.
- **T4/T5/T9 from CP1 NIT 3 are still un-homed** at CP2. CP2 introduces `nebula-redact` (a fresh workspace-deps + deny.toml opportunity); CP4 should either fold T4/T5/T9 into cascade or document out-of-scope explicitly.

---

## Summary

§5.1 closes the macro-harness gap T1 (Phase 1 audit row "no [dev-dependencies]"). §6.3 adds `nebula-redact` as a NEW dedicated workspace crate with sound rationale (single audit point, layering correctness, review surface). §2.7.2 freezes parallel feature flags `unstable-retry-scheduler` + `unstable-terminate-scheduler` — closes my CP1 NIT 1. Macro emission is feature-flag-agnostic, so `cargo expand` snapshots don't fragment by feature combination — positive CI finding.

**Top 2 CI/dep concerns:** (1) `macrotest = "1.0.13"` is two minor versions stale (current 1.2.1) — the §5.5 `expand_args` API citation may not match 1.2.x; bump to `"1.2"` and verify; (2) §5.1's "macro test harness is the only consumer" claim is wrong — `trybuild` has 3 consumers (schema, validator, action-macros) after CP2 and should be promoted to workspace dep per Phase 1 audit hygiene direction. Both are inline-edit fixes, not design-side defects.

Migration prep for CP3: the hard-removal of `CredentialContextExt::credential<S>()` is sufficiently flagged at §6.2 with codemod fork-points named at §6.2.4. CP3 §9 codemod author has an unambiguous spec. Reverse-dep call-site auto-vs-manual breakdown is implicit and would benefit from per-crate enumeration at CP3 prep time.
