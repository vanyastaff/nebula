# Tech Spec CP4 — Tech-Lead FINAL Ratification

## Verdict

**RATIFY → FROZEN.** All four architect decisions confirmed, all six CP3 edits verified landed, register flips spot-checked 22-of-22, §14 cross-refs resolve, §15.1-§15.5 close cleanly, §16.2-§16.5 implementation handoff is sufficient. No bounded edits required at CP4.

Tech Spec status flips: CP4 draft → **FROZEN**. Phase 6 complete. Cascade Phases 0-8 implementation foundation locked at: Strategy §4 + ADR-0036 + ADR-0037 + Tech Spec CP1+CP2+CP3+CP4 + register (22 rows `decided`).

---

## §15.2 deprecated `AcquireOptions::intent/.tags` — confirm

**Confirmed.** `#[deprecated(note = "engine integration ticket #391 not yet wired; field is reserved but does not affect acquire dispatch")]` is the right call. §15.2 lines 2540-2546 enumerate the three options correctly.

- `(c) retain` violates canon §4.5 (false-capability) and `feedback_incomplete_work.md` (reserved-but-unused public API is exactly what the redesign closes — the Phase 1 🟠-8 finding tagged R-051).
- `(a) #[doc(hidden)]` hides from rustdoc but leaves the field construct-able with no caller-side warning. Silent drop is the precise anti-pattern Phase 1 found.
- `(b) #[deprecated]` actively warns at compile time, pairs with the eventual #391 resolution path, and degrades to removal-in-follow-up if #391 dies.

Verified against trunk: `crates/resource/src/options.rs:36-40` `AcquireIntent::Critical` already documents "Will allow callers to bypass queues or skip throttling once engine integration lands; today it is informational only (#391)" — `#[deprecated]` makes the doc-comment promise enforceable. R-051 register flip to `decided → §15.2` is well-grounded.

## §16.1 atomic single-PR — confirm

**Confirmed.** Single atomic PR per Strategy §4.8 + ADR-0036 + Phase 2 security-lead BLOCK on Option A. §16.1 lines 2617-2634 are correct.

- Strategy §4.8 line 319 commits to "Single PR wave migrates all 5 in-tree consumers atomically" with three reasons stacked: `feedback_no_shims.md`, `feedback_hard_breaking_changes.md`, security-lead BLOCK on partial-state.
- The phased 2-3 PR alternative (Strategy §6.2 Phase C "default unless review surfaces a separable concern") is properly rejected — CP4 surfaces no separable concern. Splitting forces a half-migrated-trunk state that contradicts Strategy §0 freeze policy.
- §16.1 PR contents enumeration is complete: trait reshape, file-split, reverse-index + dispatcher, Daemon/EventSource extraction, observability wiring, doc rewrite, 5 consumer migrations, register flips, compile-fail probes. No daemon-related work omitted.

The "review-narrative" sub-ordering note (action/sdk → plugin/sandbox → engine within the same commit-set) is helpful for the actual PR review without violating atomicity. 2am-test passes: nothing in this plan would wake someone up.

## §15.6 register flips — verified 22 of 22

Cross-checked register `tech-spec-material` rows against §15.6 table:

| Source | Count | Match |
|--------|-------|-------|
| Register (line 31-92, all rows tagged `tech-spec-material`) | 22 IDs | R-001/002/003/004/005/010/011/012/020/021/022/023/030/031/032/033/034/035/043/051/053/060 |
| Tech Spec §15.6 table (line 2584-2607) | 22 rows | identical IDs, set-equal |

Symmetric difference: empty. All 22 rows have section pointers; pointers spot-checked:
- R-001 → §2.1 + §11.2 + ADR-0036 ✓ (CP1 trait declaration is at §2.1)
- R-002 → §3.1 + §5.3 + §14.4 row 🔴-1 ✓ (reverse-index write path is §3.1, tainting is §5.3)
- R-021 → §9.3 + §10.2 ✓ (file-split has registration submodule at §9.3, dual helpers at §10.2)
- R-051 → §15.2 ✓ (the `#[deprecated]` decision)
- R-060 → §6 + §6.5 DoD gate ✓ (rotation observability + DoD gate)

Register §"Lifecycle rules" rule 2 ("tech-spec-material items must be addressed before CP4 freeze") satisfied.

## CP3 edits verification (6 edits — all landed)

| Edit | Verification |
|------|--------------|
| **E1** ResourceDispatcher visibility | `pub(crate) trait ResourceDispatcher` at line 699 + `pub use manager::ResourceDispatcher as __internal_ResourceDispatcher;` at line 1785. Oxymoron framing replaced with clean shape. |
| **E2** wait_for_drain cross-pointer | §9.7 line 1704 ("NOT in `gate.rs` — moved per §9.6") + §9.6 line 1691 explanatory paragraph both present. |
| **E3** register_* total = 11 | §10.2 line 1819-1821 explicitly states "Total `register*` public surface = 11 methods" with the 10 dual + 1 type-erased breakdown. |
| **DX-1** §11.1 imports | `RegisterOptions`, `PoolConfig`, `AcquireOptions` added to import list (verified §11 imports section). |
| **DX-2** §11.2 illustrative-shape | `rust,ignore` annotations present (lines 1976, 2100, 2155); spike `lib.rs:125-156` (MockKvStore) + `lib.rs:158-200` (MockHttpClient) cited as compile-checked baseline (lines 2513-2514). |
| **DX-3** §11.5/§11.6 walkthrough framing | `rust,ignore` on hypothetical `PostgresCredential`/`build_pool_from_scheme` blocks present. |

All six edits took.

## §14 cross-ref integrity

§14.1 Strategy refs (9 rows): every Strategy §4.x → Tech Spec §X mapping resolves; spot-checked §4.4 → §10.1+§12, §4.6 → §5.5+§7.2, §4.8 → §16.1+§16.2 — all sections exist. §14.2 ADR refs: ADR-0036 §Decision → §2.1+§3.1+§3.2 verified; ADR-0037 amendment record cites the 2026-04-25 amended-in-place CP1 calibration correctly. §14.3 credential cross-spec one-way dependency consistent with §15.5 closure rationale and Strategy §4.2 footnote. §14.4 Phase 1 finding map covers all 6 🔴 + 9 🟠 in-scope findings; deferred 🟠-13 (transport tests) properly flagged as post-cascade. §14.5 + §14.6 register and spike artefact links resolve.

## §15.1-§15.5 Strategy closures

Each §5.x closes against a CP-section-anchored resolution:
- §15.1 (Strategy §5.1 Daemon revisit) → trigger-1 (~500 LOC) + trigger-2 (≥2 non-trigger workers) concretized; §12.1 `daemon/` rejection of pre-naming `scheduler/` aligns.
- §15.2 (Strategy §5.2 intent/tags) → `#[deprecated]` per options.rs landing site cited.
- §15.3 (Strategy §5.3 Runtime/Lease collapse) → trigger framing held through Phase 4-6; spike + Tech Spec evidence cited (`Lease = Runtime` everywhere); R-050 future-cascade pointer correct.
- §15.4 (Strategy §5.4 NoCredential symmetry) → resolved by §10.2 dual-helper; CP3 ratification record cited.
- §15.5 (Strategy §5.5 revoke spec extension) → closed at Strategy §4.2 footnote (CP2 E4) + reaffirmed via CP1 Q5 + CP2 §5.3; cross-spec one-way dependency consistent.

## §16.2-§16.5 implementation handoff

- **§16.2 per-consumer migration sequence** covers all 5 consumers (action/sdk/engine/plugin/sandbox). Mechanical verification commands (`rg "type Auth = "`) are concrete. Engine-side substantive work (493+75 LOC) appropriately scoped.
- **§16.3 rollback** — targeted-fix preferred, feature-flag NOT used (correct per `feedback_no_shims.md` + Strategy §4.8 atomicity), full-revert as last resort with mechanical `git revert <merge-commit>` + Phase 2 round 2 restart. Soak failure thresholds (counter `errors` >0.1%, structural panic, regression) are concrete.
- **§16.4 DoD checklist** — CI gate (5 checks) + 7 invariants (todo!() absence, atomic reverse-index, per-resource isolation, scheme-not-held-across-await, no `Scheme::default()` at warmup, observability triple, drain-abort phase=Failed). Verifier commands and §-pointers are concrete. Sufficient for cascade-completion declaration.
- **§16.5 MATURITY transition** — matches Strategy §6.4 line 396-407 transition criteria exactly: zero 🔴 in counter, register zeroed, consumer tests green, doc surface clean. Bump-as-separate-PR posture preserves `MATURITY.md` review cadence (per `feedback_no_shims.md` and the `feedback_active_dev_mode.md` discipline I called in CP3 ratification — bump is *separate* from migration PR).

## FROZEN gate — cleared

| Gate | Status |
|------|--------|
| All four architect CP4 decisions confirmed | CLEARED |
| Six CP3 edits landed in spec body | CLEARED |
| 22 register tech-spec-material rows flipped to `decided` with section pointers | CLEARED |
| §14 cross-references resolve | CLEARED |
| §15.1-§15.5 close all five Strategy §5 open items | CLEARED |
| §16.4 DoD checklist sufficient for cascade-completion | CLEARED |
| §16.5 MATURITY transition matches Strategy §6.4 | CLEARED |
| ADR-0036 + ADR-0037 unchanged at `accepted` (CP4 is internal milestone, not ADR gate) | CLEARED |

**Tech Spec FROZEN. Phase 6 complete. Cascade implementation foundation locked.**

The migration PR (per §16.1) now unblocks; merge unblocks soak (§16.3, 1-2 weeks per Strategy §6.3); soak + 7-invariant verification unblocks the maturity bump proposal (§16.5). The cascade arc closes when `MATURITY.md` records `nebula-resource = core`.

---

## Architect handoff

Architect can flip Tech Spec status from `CP4 draft — awaiting spec-auditor + tech-lead review` to `FROZEN` and append the Phase 6 closure entry to the Changelog. No further architect iteration on §14-§16 required.

Recommended Changelog entry shape (architect drafts the final wording):
> 2026-04-24 CP4 ratified (architect; tech-lead RATIFY → FROZEN + spec-auditor [pending]). Status flipped to `FROZEN`. Phase 6 complete; cascade implementation foundation locked at Strategy §4 + ADR-0036/0037 + Tech Spec CP1-CP4 + register (22 rows `decided`). Migration PR per §16.1 unblocked.
