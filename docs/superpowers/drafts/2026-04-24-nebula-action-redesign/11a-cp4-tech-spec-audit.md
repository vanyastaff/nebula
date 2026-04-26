# CP4 Tech Spec audit (§14-§16 + cross-CP final sweep)

**Auditor:** spec-auditor
**Document:** `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` (DRAFT CP4, 2679 lines)
**Companion:** `docs/adr/0039-action-macro-emission.md` (amendment-in-place verification)
**Read passes:** structural | consistency | external | bookkeeping | terminology

---

## Verdict

**REVISE** — three 🔴 BLOCKERs (all mechanical / pin-fix scope; no design or decision rework). After fixes: PASS-WITH-NITS. Freeze-ready: **NO** until 🔴 cleared. Estimate: 15-min architect pass.

---

## §14 cross-reference accuracy

§14.1 ADR matrix (4 rows). ADR-0035 status verified `accepted` (amendments 2026-04-24-B + 2026-04-24-C present in §Status block, lines 5-16). ADR-0038 / 0037 / 0038 statuses all verified `proposed`. Row 3 ADR-0039 status update text "proposed → proposed (amended-in-place 2026-04-25)" matches on-disk ADR §Status line 22.

§14.2 Strategy cross-refs (13 rows). Strategy line ranges spot-checked: §4.2 line 198-206 (table), §6.5 line 408-413 (a/b/c table), §6.6 line 416-426 (cross-crate coordination), §6.8 line 443-461 (B'+ contingency) — all verified.

§14.3 Credential Tech Spec cross-refs (13 rows). Line 2452 (`SlotType` matching pipeline) and line 2456-2470 (`iter_compatible` body) verified on disk. Line 869 (HRTB shape), line 851-863 (`credential_slots(&self)` cardinality), line 3394-3429 (`SchemeGuard` decision) all verified. **However**: see 🔴 #1 below — §9.4 is itself superseded by §15.8 in credential Tech Spec CP5; the "cross-crate authoritative" framing is now stale.

§14.4 CR1-CR11 closure traceability. 11 rows all map to §1 G-handles + §6 / §10 closure points. Internally consistent.

§14.5 Phase 0 evidence. **🔴 BLOCKER** — see below.

## §15.5 ADR-0039 amendment-in-place verification

ADR-0039 on disk verified line-by-line against §15.5.1 enactment claims:

- Line 22 status header: `Proposed (amended-in-place 2026-04-25)` ✓
- Line 24 CHANGELOG block: amendment narrative present, cites Tech Spec CP4 §15.5 + ADR-0035 precedent ✓
- Line 49 inline comment in §1 marks the shape edit ✓
- Line 56 `field_name: "slack"` (renamed from `key`) ✓
- Line 57 `slot_type: SlotType::CapabilityOnly { capability: Capability::Bearer }` (capability folded into variant) ✓
- Line 71-74 `SlotBinding` shape: 3 fields (`field_name`, `slot_type`, `resolve_fn`) ✓
- Line 80-90 three-variant `SlotType` enum present ✓
- §3 / §4 / §5 unchanged per §15.5 "not load-bearing" claim ✓ (lines 104-148 spot-checked)

Amendment **landed correctly**. Modulo 🔴 #1 (terminology in doc comment) — the structural shape is right but the doc comment at ADR-0039 line 86 references the now-removed `capabilities_enabled` field.

## §15.3 / §15.4 soft-amendment flag status

Both flagged-not-enacted per §15 narrative + ADR-0035 precedent. Verbatim "FLAGGED, NOT ENACTED" markers + coordination-owner + sunset-window rows present in §15.3 lines 2270-2282 and §15.4 lines 2284-2307. The credential Tech Spec on disk confirms:

- Probe #7 still shows unqualified `let g2 = guard.clone()` form (line 3756); Tech Spec correctly flags as silent-pass per spike finding #1.
- §15.7 `engine_construct` constructor on disk uses the iter-3 refined form (line 3503-3516) but does NOT have the `engine_construct_with_probe` test variant — Tech Spec §15.4 correctly flags absence.

Both soft-amendment flag rows internally consistent. No enactment performed by Tech Spec, as constraint requires.

## §16.1 (a/b/c) framing alignment

Strategy §4.2 line 200-204 (table) and §6.5 line 408-413 verified. Tech Spec §16.1 line 2375-2381 extends the Strategy table with a "Cascade-final criteria" column — additive, not contradictory. Path (c) NOT VIABLE gate cited correctly: Strategy §6.6 silent-degradation guard + §6.8 architect+tech-lead co-decision.

Path naming: Tech Spec uses `(a) Single coordinated PR` / `(b) Sibling cascades` / `(c) Phased B'+ surface commitment` — matches Strategy §4.2 row labels verbatim. ✓

## §16.3 DoD completeness

7-item DoD checklist:

1. All 11 🔴 closed → traces to §14.4 closure table ✓
2. Security must-have floor 4 items → S-J1 / S-C2 / Display sanitization / cancellation-zeroize ✓
3. Macro test harness → `crates/action/macros/tests/` + 6 probes + Probe 7 + macrotest snapshots ✓
4. Sealed DX adapter pattern → 5 sealed traits + canon §3.5 line 82 revision PR ✓
5. 7 reverse-deps migrated → engine + api + sandbox + sdk + plugin + cli + macros ✓
6. `nebula-redact` workspace integration → 4 atomic edits ✓
7. `deny.toml` positive ban → wrappers-list extension + new positive ban ✓

Plus implicit: ADR-0038 / 0037 / 0038 status moves. ADR-0039 amended-in-place qualifier preserved. ✓

DoD covers must-have floor (4), harness, sealed adapter, reverse-deps, nebula-redact, deny.toml. Complete.

## Cross-CP integrity sweep

§0.1 status table now shows CP4 row as `active` and prior CP1/CP2/CP3 rows as `locked CPN`. Frontmatter `status: DRAFT CP4` matches. ✓

CP4 CHANGELOG (lines 2664-2671) records §14 + §15 + §16 additions, §15.5.1 ADR-0039 enactment. Internally consistent with §15.5.1 narrative. ✓

Cross-CP residual contradictions: see 🟠 issues — `unstable-action-scheduler` typo in §16.4 (CP4-introduced); residual CP1 path bug at line 642 forwarded into §15.1 line 2233 (CP4 row).

## Freeze readiness

**NO** — three 🔴 BLOCKERs require pin-fix before freeze. All are mechanical (terminology / line number / file name corrections); no decision-level rework needed. After fixes: PASS-WITH-NITS, freeze-ready.

---

## 🔴 BLOCKERS

**1. §14.5 cross-references cite WRONG file (`01-current-state.md` instead of `01b-workspace-audit.md`).**

§14.5 line 2182 names `[`01-current-state.md`](../drafts/2026-04-24-nebula-action-redesign/01-current-state.md)` as the citation source. The cited file is **250 lines** and has no §11. The line numbers cited (line 44, line 252-329, line 346-356, line 376, line 379) all live in **`01b-workspace-audit.md`** (382 lines, §1-§11).

Verification:
- `wc -l 01-current-state.md` → 250
- `wc -l 01b-workspace-audit.md` → 382
- §11 grep on `01-current-state.md` → no §11 (top-level §10 = Phase 1 dispatch readiness)
- §11 grep on `01b-workspace-audit.md` → §11 at line 367 ("Ground truth summary"); line 376 = lefthook finding; line 379 = deny.toml finding ✓

§14.5 promises "Every citation below has a corresponding `grep`-able anchor in the cited document at draft time" — the promise is structurally false for **5 of 7** cited lines. Same false citation propagates to §13.4.1 (line 2015 "Phase 0 audit §1 finding 🟠 MAJOR line 44"), §13.4.2 (line 2029), §13.4.3 (line 2037), §16.2 line 2398 ("per Phase 0 §10 line 358").

**Impact:** Future re-pin events (per §0.2 invariant 4) will dereference wrong file; reviewer pass cannot verify line ranges. Load-bearing for §15.7 RATIFIED dispositions (T4 / T9 / nebula-redact all ground in this Phase 0 evidence).

**Fix:** s/`01-current-state.md`/`01b-workspace-audit.md`/g across §14.5 + §13.4.x + §16.2. Verify Phase 0 §9 line 252-329 (reverse-deps inventory) is in `01b` §9 (lines 252-337 ✓). Verify Phase 0 §10 line 346-356 / line 358 in `01b` §10 (337-365 ✓). Verify Phase 0 §11 line 376 + 379 in `01b` §11 (367-382 ✓). One row needs split — Phase 0 §1 line 44 ground in *both* files? Re-pin granularly.

---

**2. §14.3 + §15.5 cite credential Tech Spec §9.4 line 2452 as "cross-crate authoritative — load-bearing for §15.5 ADR-0039 amendment", but §9.4 is now SUPERSEDED by §15.8 in credential Tech Spec CP5.**

Verification:
- credential Tech Spec line 2446 quote: `> **Superseded by §15.8 (CP5 2026-04-24).** ` iter_compatible ` filter body below consults ` cred.metadata().capabilities_enabled.contains(...) ` — plugin-declared field. CP5 canonical form: filter consults ` RegistryEntry::capabilities ` (registry-computed at ` register<C> ` time from sub-trait membership).`
- The `iter_compatible` body at lines 2456-2470 (cited by Tech Spec §14.3 row 7 as "Cross-crate authoritative — load-bearing for §15.5 ADR-0039 amendment") still references `cred.metadata().capabilities_enabled` and `cred.metadata().service_key` — both **plugin-declared** fields per pre-CP5 form.
- Per credential Tech Spec §15.8 (CP5): `capabilities_enabled` field is REMOVED from `CredentialMetadata`; capability authority shifts from plugin metadata to `RegistryEntry::capabilities` (registry-computed at `register<C>` time from sub-trait membership).

ADR-0039 line 86 doc comment also reflects the stale terminology: `Engine matches both service_key + capabilities_enabled` — `capabilities_enabled` is REMOVED in CP5 canonical form.

The **structural shape** of `SlotType` (3 variants: `Concrete` / `ServiceCapability` / `CapabilityOnly`) is preserved across §9.4 → §15.8 ("Same `SlotType::Concrete / ServiceCapability / CapabilityOnly` matching axes"); only the **filter authority source** shifts. So the §15.5 SlotBinding amendment-in-place IS substantively correct, but the citation justification is supersede-stale.

**Impact:** Tech Spec readers following the §14.3 row 7 citation land on a `> Superseded by §15.8` block. The §15.5 enactment narrative cites the superseded body as authority. ADR-0039 line 86 doc comment reflects superseded terminology in the canonical ADR shape. Load-bearing for amendment-justification audit trail.

**Fix:** Three coordinated edits:

(a) Tech Spec §14.3 row 7: re-pin "credential Tech Spec §9.4 line 2452" to "credential Tech Spec §15.8 (CP5 supersession of §9.4)" + add note "matching-axis shape preserved across supersession; capability authority source shifts plugin-metadata → RegistryEntry::capabilities".

(b) Tech Spec §15.5 + §15.5.1: add explicit acknowledgement that §9.4 was superseded by §15.8 in credential Tech Spec CP5; the SlotType shape preservation is authority-source-orthogonal.

(c) ADR-0039 line 86 doc comment: replace `Engine matches both service_key + capabilities_enabled` with `Engine matches by service_key + computed capability set per credential Tech Spec §15.8 RegistryEntry::capabilities` (or equivalent).

Tech Spec §3.1 inline doc comment (lines 624-628) has the same stale-terminology problem (cites `capabilities_enabled.contains(*capability)` per credential Tech Spec §9.4 line 2467-2470). Fix in same pass.

---

**3. §16.4 Layer 1 rollback names the WRONG feature flag — `unstable-action-scheduler` was renamed to `unstable-terminate-scheduler` in CP1 §2.7.2 + Strategy line 432.**

§16.4 line 2423 (verbatim):

> **Layer 1 — Feature-flag gate path (symmetric `unstable-action-scheduler` + `unstable-retry-scheduler`).** If post-cascade soak surfaces issues with `Retry` or `Terminate` end-to-end wiring (e.g., scheduler-integration hook bug, cross-tenant boundary violation slipping through), feature-flag-gate the variant in question via the existing `unstable-retry-scheduler` / `unstable-terminate-scheduler` flags.

The parenthetical names `unstable-action-scheduler` (the **superseded** name); the body text correctly uses `unstable-terminate-scheduler`. Same line is internally inconsistent.

Verification of authoritative naming:
- CP1 CHANGELOG line 2478: "committed feature flag rename: `unstable-action-scheduler` → `unstable-terminate-scheduler` (parallel to `unstable-retry-scheduler`)"
- Strategy line 432-438: feature-flag granularity locked at parallel `unstable-retry-scheduler` + `unstable-terminate-scheduler`.
- §15.2 line 2266: "**Per §0.2 invariant 4**: this Tech Spec freezes the parallel-flag signature; CP3 §9 may amend the *internal scheduler implementation* but cannot rename or unify the public flags without an ADR amendment."

§16.4 introduces the superseded name in CP4-new content, against §0.2 invariant 4 freeze.

**Impact:** Implementer following §16.4 rollback narrative reads conflicting flag names in same paragraph. Could land code with the wrong feature gate. Load-bearing for the rollback-readiness contract DoD lists.

**Fix:** §16.4 line 2423 — replace `(symmetric `unstable-action-scheduler` + `unstable-retry-scheduler`)` with `(symmetric `unstable-retry-scheduler` + `unstable-terminate-scheduler`)`.

---

## 🟠 HIGH

**1. CHANGELOG residual: §3.1 line 642 + §15.1 line 2233 cite `crates/engine/src/registry.rs` but the file is at `crates/engine/src/runtime/registry.rs`.**

Verification: `Glob crates/engine/src/registry*` → no match. `crates/engine/src/runtime/registry.rs` exists (also `crates/engine/src/credential/registry.rs`). The CP1-locked §3.1 paragraph (line 642) carries the wrong path; the CP4 §15.1 row (line 2233) forwards the same wrong path verbatim. CP1 audit instructions said "do not re-audit closed CPs" — but §15.1 line 2233 is CP4-new content forward-referencing a CP1 wrong claim, so this is a CP4 bookkeeping fault.

**Fix:** §15.1 row "§3.1 — engine `ActionRegistry::register*` ... current host is `crates/engine/src/registry.rs`" → re-pin to `crates/engine/src/runtime/registry.rs`. Same pin in §3.1 line 642 if architect can touch (CP1-locked but cite is observably wrong). Same pin in CHANGELOG line 2477.

**2. §14.3 row 11 (`§15.7 — soft amendment for engine_construct_with_probe ...`) cites §6.4.2 and §15.4 but the row's "Status" column says "Soft amendment — flagged, NOT ENACTED" — should be parallel form to row 12 (probe #7) which uses bold form.**

Cosmetic / consistency. Both flagged-not-enacted rows should bold-mark identically for diff-reviewability at re-pin events.

**Fix:** §14.3 line 2155 → align bold "Status" cell wording with line 2156 (probe #7 row).

**3. §16.5 line 2437 cite `[docs/tracking/cascade-queue.md]` — file does not yet exist on disk.**

Verification: `Glob docs/tracking/**` returns only `credential-concerns-register.md`. The file `cascade-queue.md` does not exist. Strategy §6.6 line 424 says "Slot commitment lives in [docs/tracking/cascade-queue.md] (or equivalent — orchestrator picks at Phase 8)" — explicit "or equivalent" hedge. Tech Spec §16.5 should mirror the hedge.

**Fix:** §16.5 line 2437 + §16.1 line 2379 (path (c) cascade-final criteria) — append "(or equivalent location — orchestrator picks at Phase 8 per Strategy §6.6 last paragraph)" to the citation.

---

## 🟡 MEDIUM

**1. §15.1 CP3 carry-forward (lines 2255-2261) row "(a)-(j) line 2253-2263" — line range "2253-2263" is the prior CP3 §15 footer line range, not the current §15 line range. Drift on internal cross-line.**

Mechanical re-pin chore. Not blocking.

**2. §14.5 hygiene-T vs codemod-T disambiguation (lines 2192-2202) — clear and load-bearing, but the disambiguation table format would benefit from a header row marking "Hygiene T (CP1 09e)" vs "Codemod T (Strategy §4.3.3 + Tech Spec §10.2)".**

Pure presentation nit; current form is correct.

**3. §15.5 line 2317 "(folded into `SlotType` enum per credential Tech Spec §9.4 three-variant matching pipeline)" — same supersede-stale citation as 🔴 #2; flag for atomic fix.**

Will close with 🔴 #2 fix.

**4. §15.8 row count is 14 per CHANGELOG line 2669 but actual table has 13 numbered + 4 unnumbered rows = 17 entries. CHANGELOG mis-counts.**

Bookkeeping nit. Mechanical fix.

**5. §16 line 2367 + §16.5 narrative: Phase 8 / Phase 6 / Phase 7 references are not all defined in this Tech Spec — they live in cascade-orchestrator territory (Strategy §6 phases). Consider adding "(see Strategy §6 for Phase definitions)" once at top of §16 to anchor unfamiliar reader.**

Pure DX / readability.

**6. §14.4 row CC1 line 2178: "`semver` consumer-side dep declaration → §10.4 step 1.5 closure (per CP3 iteration 2026-04-24 dx-tester 10d R2)" — verify §10.4 step 1.5 actually closes this. Verified: line 1714 has the step ✓. Could move CC1 row to numbered position for table symmetry, but optional.**

---

## 🟢 LOW

**1. §15.1 Strategy carry-forward row "CP3 §6 inventory bookkeeping (~8 forward-promises)" — claims "All eight sub-promises mapped at Strategy frozen CP3 line 525". Strategy is 549 lines total; line 525 is in CHANGELOG / Open Items area. Spot-verified; load-bearing pass-through, no fix needed.**

**2. §14.1 line 2114 "amended 2026-04-24-B post iter-2; 2026-04-24-C post iter-3" — verifies against ADR-0035 §Status block ✓. No action.**

**3. §16.2 codemod step-counts table re-verified against §10.3 — internally consistent ✓.**

---

## ✅ GOOD

- **§14 cross-reference consolidation discipline.** Five sub-tables (§14.1 / §14.2 / §14.3 / §14.4 / §14.5) cover ADR matrix + Strategy + credential Tech Spec + Phase 1 register + Phase 0 evidence at line-pin granularity. Re-pin obligation paragraph (§14.3 line 2158) explicit.
- **§15.5.1 amendment enactment record.** Discrete bullet list of what changed (shape, status header, CHANGELOG, field rename, cross-section consistency). Verified against on-disk ADR line-by-line. This is exactly the audit-trail discipline that survives future re-pin events.
- **§15.8 deferred-with-trigger registry.** Every row has trigger + owner + scope per `feedback_active_dev_mode.md` discipline. (g) marked ENACTED inline. (f) cross-cascade coordination flagged with sunset window. No silent deferral.
- **§16.5 pre-implementation checklist.** Path (b)/(c) precondition rows correctly conditional on user pick; path (a) explicitly does NOT require credential CP6 cascade slot. Concerns-register-clean check separately listed. Phase 8 user-pick mechanic preserves "Strategy + Tech Spec + orchestrator do not pre-pick" discipline at line 2441.
- **Cross-CP CHANGELOG completeness.** CP1 + CP2 + CP3 + CP4 CHANGELOGs all present with explicit Open-items + Forward-track + Handoff sections. Reviewer pass discipline preserved across 4 CPs.
- **§16.4 rollback layer split.** Layer 1 (feature-flag gate) + Layer 2 (reverse-codemod) + Strategy §6.8 B'+ contingency are three distinct rollback shapes with explicit failure-mode triggers. Modulo 🔴 #3 typo.

---

## Coverage summary

- Structural: PASS (no missing TOC entries; section numbering consistent across §14-§16)
- Consistency: 1 finding (🟠 #1 — CP1 path forwarded into CP4 §15.1)
- External verification: 3 findings (🔴 #1 wrong file; 🔴 #2 supersede-stale; 🟠 #3 missing file with hedge)
- Bookkeeping: 2 findings (🟡 #1 cross-line drift; 🟡 #4 CHANGELOG row miscount)
- Terminology: 1 finding (🔴 #3 superseded flag name)
- Definition-of-done (§17 PRODUCT_CANON): N/A — Tech Spec defines its own §16.3 DoD checklist, which is internally complete

---

## Recommended handoff

- **architect:** all three 🔴 BLOCKERs are mechanical pin-fix scope (file rename in §14.5 / §13.4 / §16.2; supersede-stale citation in §14.3 / §15.5 / §3.1 / ADR-0039 doc comment; flag-name typo in §16.4 line 2423). 🟠 #1 is also pin-fix. 🟠 #3 hedge addition. 🟡 batch-applicable. Estimated 15-min pass.
- **tech-lead:** no decision-level items; no contested calls surfaced. **Ratification gate:** verify 🔴 #2 fix preserves the §15.5 amendment-in-place justification (capability authority source vs structural shape orthogonality is the load-bearing claim).
- **orchestrator:** after architect pin-fix pass, freeze CP4. Before freeze, confirm tech-lead ratification of §15.5 ADR-0039 enactment + §16.3 DoD framing.

---

## Summary (≤150 words)

CP4 §14-§16 are structurally complete and internally coherent. Three 🔴 BLOCKERs are all mechanical pin-fix scope: (1) §14.5 cites `01-current-state.md` as Phase 0 evidence source but the cited line numbers (44, 252-329, 376, 379) live in `01b-workspace-audit.md` instead — propagates through §13.4 + §16.2; (2) §14.3 + §15.5 + ADR-0039 line 86 cite credential Tech Spec §9.4 as authoritative for SlotBinding, but §9.4 was superseded by §15.8 in credential Tech Spec CP5 — structural shape preserved, citation stale; (3) §16.4 line 2423 names superseded flag `unstable-action-scheduler` against §0.2 invariant 4 freeze on parallel-flag signature. Plus 🟠 (CP1 file path drift forwarded to CP4 §15.1) + 🟢/🟡 minor. **Freeze-ready: NO** until 🔴 cleared. Estimate: 15-min architect pin-fix pass; no design rework.

**Top 3 issues:** (1) §14.5 wrong source file; (2) §14.3 + §15.5 + ADR-0039 supersede-stale `capabilities_enabled` terminology; (3) §16.4 flag-name typo against frozen signature.
