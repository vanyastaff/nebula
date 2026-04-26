# CP3 spec-audit — Strategy Document §6 (nebula-action redesign)

**Auditor:** spec-auditor
**Date:** 2026-04-25
**Document audited:** [`docs/superpowers/specs/2026-04-24-action-redesign-strategy.md`](../../specs/2026-04-24-action-redesign-strategy.md) (540 lines, §6 in scope; §0-§3 audited at [`04a-cp1-spec-audit.md`](04a-cp1-spec-audit.md), §4-§5 at [`05a-cp2-spec-audit.md`](05a-cp2-spec-audit.md))
**Read passes:** structural | consistency | external | bookkeeping | terminology

---

## Audit verdict

**REVISE.** §6 sub-section completeness is intact (8/8 sub-sections, all referenced from §0 reading order + §5 forward-promises). However, **two 🔴 BLOCKERs** (§6.7 includes S-C5 misclassified as deferred-and-paired-with-S-C4, but S-C5 is actually part of the §2.12/§4.4 must-have floor — closes via cancellation-zeroize test, NOT a deferred 🟠 item; §6.2 ADR-00NN+3 names cluster-mode hooks "lease, fence, restart" which contradict §3.1/§3.4/§5.1.5 hook names "IdempotencyKey, on_leader_*, dedup_window") plus **one 🟠 HIGH** (§6 falsely claims discharge of "all 8 forward-promises" in line 518, but at minimum 3 of the 8 — retry-scheduler chosen path, canon §11.2 edit, §2.11 amendment roll-in — are NOT mapped to any §6 sub-section) preclude PASS. Three 🟠 + four 🟡 carry forward.

**Freeze readiness: NO.** S-C5 misclassification is a load-bearing inconsistency (sunset table claims S-C5 is deferred-tracked, but §4.4 line 252 + §2.12 item 4 + 03c §2 item 4 list it as must-have-this-cascade; same finding cannot be both shipped this cascade and deferred ≤2 cycles). Hook-name drift in ADR-00NN+3 will propagate into ADR drafting if not fixed. Single architect revision pass + targeted fixes will close.

---

## §6 completeness check

### ✅ All 8 sub-sections present
- §6.1 Spike → Tech Spec sequencing (line 359)
- §6.2 ADR drafting roadmap (line 367)
- §6.3 Tech Spec checkpoint roadmap (line 382)
- §6.4 Concerns register lifecycle (line 396)
- §6.5 Post-cascade implementation path criteria (line 404)
- §6.6 Cross-crate coordination tracking (line 416)
- §6.7 Sunset commitments for deferred 🟠 findings (line 426)
- §6.8 B'+ contingency activation criteria (line 442)

Headers consistent; numbering monotonic; each cross-linked from §0 reading order ("§6 (post-validation roadmap, CP3)") and from CHANGELOG line 509-517 inventory.

### ✅ §6 scope statement (line 357) is structurally honest
"Strategy decisions remain locked at §1-§5; §6 does not introduce new decisions, only sequencing, ownership, and post-cascade tracking." — verified, all sub-sections are bookkeeping/sequencing/owner-stamping, not new design decisions.

---

## Cross-section forward-ref closure

### ✅ §3.2 B'+ contingency criteria closed by §6.8
§3.2 line 141 ("CP3 §6 (post-validation roadmap) records the contingency activation criteria") → §6.8 records: activation signals (binary OR), rollback path, co-decision rule, sunset commit. Closed.

### ✅ §3.2 condition (2) cascade slot verification closed by §6.6
§3.2 line 146 ("CP2 §4 / CP3 §6 must verify the slot is committed before ratifying any B'+ contingency activation") → §6.6 specifies the slot's three required fields + Phase 8 user-pick gate. Closed.

### ✅ §5.1.4 B'+ activation signal/rollback/sunset → §6.8
§5.1.4 line 286-291 promised CP3 §6 record (a) signal, (b) rollback, (c) sunset commit on bridge — §6.8 covers all three. Closed.

### 🔴 BLOCKER — §4.3.2 retry-scheduler path NOT recorded in §6
**Quote (§4.3.2 line 229):** "Tech Spec §9 picks the path; CP3 §6 records the chosen path in the post-validation roadmap."
**Evidence:** `grep -n 'retry-scheduler' specs/2026-04-24-action-redesign-strategy.md` shows §6 contains zero references to retry-scheduler. The phrase appears at §6.3 line 391 ("CP3 | §9-§13 | Migration, codemod, retry-scheduler chosen path...") but only as the **Tech Spec** CP3 prompt's section list — not as Strategy §6 recording the chosen path.
**Impact:** The §4.3.2 forward-promise that "CP3 §6 records the chosen path" is unfulfilled. Either §4.3.2 should be reworded to defer to Tech Spec §9 only (deleting the §6-recording commitment), or §6 needs a new sub-section (e.g., §6.9 retry-scheduler chosen-path bookmark) cross-pointing to where Tech Spec §9 will land the decision.
**Suggested fix:** Reword §4.3.2 line 229 from "CP3 §6 records the chosen path in the post-validation roadmap" to "Tech Spec §9 picks and records the chosen path; Strategy CP3 §6 cross-links once Tech Spec §9 lands." Or add §6.9 explicitly. Architect picks.
**Severity:** 🔴 BLOCKER — §6 promises something it doesn't deliver, in a section that is supposed to discharge §4 forward-promises.

### 🟠 HIGH — Line 518 claim "all 8 forward-promises mapped to §6 sub-sections" is false
**Quote (CHANGELOG line 518):** "§5 open items — CP3 §6 inventory line 367 promise discharged: all 8 forward-promises mapped to §6 sub-sections."
**Evidence:** §5 line 474 inventory of 8 sub-items:
1. B'+ activation criteria + signal triggers + rollback path + sunset commit → §6.8 ✅
2. retry-scheduler chosen path → ✗ NOT in §6 (see 🔴 above)
3. canon §11.2 edit (if wire-end-to-end picked) → ✗ NOT in §6
4. cluster-mode coordination cascade scheduling commitment → §6.6 partial only (covers credential CP6 slot, NOT cluster-mode coordination cascade slot)
5. `#[trait_variant::make]` separate redesign forward-flag → ✗ NOT in §6 (still only in §3.4 OUT row)
6. path (b) per-cascade budget re-baseline → §6.5 partial (line 411 "per-cascade budgets re-baselined at path selection")
7. spike output → Tech Spec §7 traceability → §6.1 ✅
8. §2.11 amendment roll-in → ✗ NOT in §6 (still tracker-only at "Open items raised this checkpoint" line 473)

**Count:** 2 fully discharged + 2 partial + 4 not addressed = 4/8 either missing or only partially addressed.
**Impact:** CHANGELOG asserts complete discharge; reality is ~50% discharge. CP3 freeze cannot honestly proceed under this claim.
**Suggested fix:** Either (a) add §6 sub-sections covering items 2/3/4/5/8, or (b) reword line 518 to "all 8 forward-promises mapped to §6 sub-sections OR explicitly deferred to Tech Spec §7/§9 with cross-pointer" + audit each promise's deferral location.
**Severity:** 🟠 HIGH — internal contradiction between CHANGELOG claim and §6 actual content.

### 🟡 MEDIUM — §5.1.3 forward-ref "Pin line numbers when CP3 §6 cites the dispatch pattern" not satisfied
**Quote (§5.1.3 line 282):** "Pin line numbers when CP3 §6 cites the dispatch pattern."
**Evidence:** §6 references the dispatch pattern at §6.2 line 378 ("ADRs explicitly cite ADR-0035 as load-bearing") and at §6.1 ("HRTB + `SchemeGuard` + macro emission shape locked"), but no specific credential Tech Spec §7.1 line numbers appear in §6.
**Impact:** Reader following §5.1.3 expects §6 to cite §7.1 line numbers; gets only general references.
**Suggested fix:** Either add a §6 line citing credential Tech Spec §7.1 line numbers (architect to grep for `RefreshDispatcher::refresh_fn` definition site), or reword §5.1.3 line 282 to "Pin line numbers when Tech Spec §7 cites the dispatch pattern."
**Severity:** 🟡 MEDIUM — minor forward-promise non-discharge.

---

## §6.7 sunset table check

### 🔴 BLOCKER — S-C5 misclassified as deferred + paired with S-C4
**Quote (§6.7 line 434):** "**S-C5** credential-keyed lifetime mismatch | Same as S-C4 (paired) | ≤2 release cycles | Same as S-C4"
**Evidence:**
- 02b-security-threat-model.md line 108: "🟡 **MINOR S-C5** — no test asserts that `CredentialGuard::Drop` actually fires when the holding future is cancelled. `guard.rs::drop_zeroizes_inner` covers the *explicit drop* case only. Defense-in-depth gap..."
- 02b line 380 Top-N table: "S-C5 | (no test) | No test asserts zeroize fires when holding future is cancelled | Regression vector"
- 03c §2 item 4 (line 121): "**Add cancellation-zeroize test** (closes S-C5). Test that a `CredentialGuard` held by a cancelled future drops + zeroizes. Pure test addition — no architectural cost."
- Strategy §2.12 item 4 (line 104): "Cancellation-zeroize test — closes S-C5; pure test addition, no architectural cost."
- Strategy §4.4 item 4 (line 252): "Cancellation-zeroize test — closes S-C5; pure test addition, no architectural cost."

**S-C5 is a missing-test finding, NOT a "credential-keyed lifetime mismatch."** The "credential-keyed lifetime" framing is entirely S-C4's territory (03c line 132: "S-C4 — needs `!Send`/`!Sync` or context-keyed lifetime on `CredentialGuard`"). §6.7 conflates S-C5 with S-C4 by mis-naming both the finding and the resolution scope.

Furthermore, §6.7 line 428 frames the table as "Per security-lead 03c §3 deferred-but-tracked items (cited verbatim at §2.12)" but **S-C5 is in 03c §2 must-have hardening (line 121), not 03c §3 deferred-but-tracked (line 127-138)**. 03c §3 lists 6 items (S-W2, S-C4, S-O1/O2/O3, S-I2) plus minors (S-W1, S-W3, S-F1, S-I1, S-U1, S-C1) — **S-C5 is not in that list**.

§2.12 line 106 deferred list: "S-W2 ... S-C4 ... S-O1/S-O2/S-O3 ... S-I2 ... S-W1/S-W3/S-F1/S-I1/S-U1/S-C1 minor" — **S-C5 not in §2.12 deferred either**.

**Impact:** A finding that the must-have floor commits to closing this cascade is duplicated as a deferred-tracked item with a 2-cycle sunset window. Reader cannot tell whether S-C5 ships now or later. Worse, the framing under "Same as S-C4 (paired)" routes the resolution into S-C4's structural-fix track rather than the §2.12 cancellation-zeroize-test track.

**Suggested fix:** **Remove the S-C5 row from §6.7.** S-C5 is a must-have-this-cascade item closed by the §4.4 / §2.12 cancellation-zeroize test addition (per 03c §2 item 4). The §6.7 framing line 428 then matches reality (6 items: S-W2, S-C4, S-O1, S-O2, S-O3, S-I2). If user/orchestrator/security-lead want a 7th item, add the actual deferred minor (e.g., S-W1 `FUTURE_SKEW_SECS` per 03c line 136) — not S-C5.

**Severity:** 🔴 BLOCKER — load-bearing inconsistency: §6.7 says S-C5 is deferred ≤2 cycles; §4.4 + §2.12 + 03c §2 say S-C5 ships in cascade (test addition). Same finding cannot be both.

### 🟠 HIGH — §6.7 framing claim "Per security-lead 03c §3 deferred-but-tracked items (cited verbatim at §2.12)" is inaccurate
**Quote (§6.7 line 428):** "Per security-lead 03c §3 deferred-but-tracked items (cited verbatim at §2.12)..."
**Evidence:** §6.7 table contains 7 rows; 03c §3 has 6 sunset-tracked items + 6 minor items. §2.12 deferred list has 6 items + 6 minor. §6.7 includes the spurious S-C5 row (see 🔴 above) and excludes the minor items (S-W1, S-W3, S-F1, S-I1, S-U1, S-C1).
**Impact:** Reader expects verbatim citation; finds a transformed list.
**Suggested fix:** After removing S-C5 (per 🔴 fix above), §6.7 will be 6 items matching 03c §3 sunset-target rows. Either rename §6.7 to clarify "covers 03c §3 sunset-target items only; minor 03c §3 items (S-W1, etc.) tracked at cascade exit notes per 03c line 137" or add a second sub-table for the minors.
**Severity:** 🟠 HIGH — framing claim doesn't match table content.

### 🟡 MEDIUM — orchestrator's checklist asserted "(S-W2, S-C4, S-C5, S-O1, S-O2, S-O3, S-I2 — 7 items)"
The user's audit checklist instruction stated "S-C5" as one of the 7 items expected. Per the source-of-truth audit above, S-C5 should NOT be in §6.7. Auditor-of-the-auditor note: orchestrator's pre-loaded checklist appears to inherit the §6.7 misclassification rather than independently verify against 03c §3 + §2.12. Flagged here so orchestrator can refresh the inventory.

---

## Freeze readiness

### Internal-consistency state of the §1-§6 freeze artefact

**Carry-forward issues from CP1+CP2 audits — confirmed unchanged in CP3:**
- 🟡 §0 status table line 28: "DRAFT CP2 (this revision)" annotation persists. Frontmatter is `status: DRAFT CP3`. CHANGELOG line 508 acknowledges the move to CP3 row "at next iteration." Persistent annotation drift; was 🟡 in CP2 audit, remains 🟡 in CP3.

**New CP3 issues (this audit):**
- 🔴 §6.2 ADR-00NN+3 hook names "lease, fence, restart" contradict §3.1 component 7 + §5.1.5 + 03-scope-decision §1.7 hook names "IdempotencyKey, on_leader_*, dedup_window".
- 🔴 §6.7 S-C5 row misclassifies a must-have-floor item as deferred.
- 🔴 §4.3.2 retry-scheduler chosen path forward-ref to §6 unfulfilled.
- 🟠 Line 518 "all 8 forward-promises mapped" claim is false (4/8 not mapped).
- 🟠 §6.7 framing "verbatim at §2.12" is inaccurate (S-C5 not in §2.12, minors omitted).
- 🟠 §6.6 covers credential CP6 slot only; cluster-mode coordination cascade slot (per §3.4 row 3 + 05b CP3 hint #4 line 157) is not tracked anywhere despite being in line 474 inventory item 4.
- 🟡 §6.2 ADR-00NN+3 cites "tech-lead 05b CP3 hint line 143" — line 143 is about Tech Spec §7 hook delineation, not ADR drafting; the cluster-mode cascade scheduling hint is at line 157 (CP3 hint #4).
- 🟡 §6.2 ADR-00NN row 1 mis-cites "§3.2 A' selection" (§3.2 = B'+ runner-up; A' selection is §3.1 + §4.1) and "§3.1 component 1" (component 1 is credential vocabulary; ADR-00NN is action `#[action]` macro shape, which is §3.1 component 2).
- 🟡 §5.1.3 line 282 "Pin line numbers when CP3 §6 cites the dispatch pattern" not satisfied in §6.

### Status header check

🟢 Frontmatter `status: DRAFT CP3` matches §0 reading-order line 38 ("§6 (post-validation roadmap, CP3)") and CHANGELOG line 508 ("DRAFT CP2 (iterated 2026-04-24)" → "DRAFT CP3"). Status header is correct.

### CHANGELOG / Handoffs disambiguation check

🟢 All four CHANGELOG/Handoffs blocks qualify with checkpoint label: `### CHANGELOG — CP3 (since CP2 iterated lock)` (line 505), `### Handoffs requested — CP3` (line 520), `### CHANGELOG — CP2 (since CP1 lock)` (line 476), `### Handoffs requested — CP2` (line 498), `### CHANGELOG — CP1 (since initial draft)` (line 526), `### Handoffs requested — CP1` (line 536). CP1+CP2+CP3 all qualified. **Spec-auditor CP2 finding (duplicated headers) addressed; structural integrity of section heading uniqueness restored.**

### Anything in §1-§5 still pointing forward without §6 resolution

- §4.3.2 line 229 retry-scheduler path → 🔴 above.
- §5.1.3 line 282 dispatch-pattern line numbers → 🟡 above.
- §2.11 amendment-pending — §4.3.1 line 218 + §4.4 line 254 mention "§2.11 explicit citation roll-up deferred to CP3." Deferral phrasing acceptable; §2.11 itself is not amended in CP3 ("Open items raised this checkpoint" line 473 keeps it as tracker). NOT a freeze blocker if the doc commits to §2.11 amendment in a CP3 final-iteration; the CHANGELOG line 517 inventory item 8 ("§2.11 amendment roll-in") remains undischarged.

---

## Coverage summary

- **Structural:** REVISE — 2 🔴 (S-C5 row in §6.7; ADR-00NN+3 hook names); 1 🟠 (line 518 false-completion claim); 1 🟡 (§0 status table stale annotation).
- **Consistency:** REVISE — 1 🔴 (§4.3.2 retry-scheduler forward-ref unfulfilled); 1 🟠 (§6.7 framing claim inaccurate); 2 🟡 (§6.2 ADR-00NN+3 cite-line-143; §6.2 ADR-00NN cite-§3.2-A').
- **External verification:** PASS — credential Tech Spec §3.4/§15.7 paths verified; 03c §3 deferred list grepped; 02b S-C4/S-C5 finding text grepped; 05b CP3 hints lines 151-161 confirmed; security-lead 03c §2 item 4 (cancellation-zeroize test closes S-C5) confirmed.
- **Bookkeeping:** REVISE — 1 🟠 (CHANGELOG line 518 claim false); 1 🟠 (cluster-mode coordination cascade slot not tracked in §6.6 despite being in line 474 inventory).
- **Terminology:** PASS-WITH-NIT — option labels stable (A'/B'+/B'/C'); CP6 vocabulary stable; 1 🔴 (cluster-mode hook names drift in §6.2 vs §3.1/§5.1.5/§3.4); 1 🟡 (§5.1.3 forward-promise to "cite dispatch pattern" not discharged in §6).
- **Definition-of-done (PRODUCT_CANON §17):** N/A for Strategy CP3 (DoD applies to Tech Spec / impl phase).

---

## Summary for orchestrator

**Verdict: REVISE.** §6 sub-section completeness PASS (8/8 present). Cross-section closure REVISE with **2 🔴 BLOCKERs**:
1. **§6.7 line 434 — remove the S-C5 row.** S-C5 is a must-have-this-cascade item closed by the cancellation-zeroize test (per §4.4 item 4 + §2.12 item 4 + 03c §2 item 4). Listing it as deferred ≤2 cycles "Same as S-C4 (paired)" creates a contradiction with the must-have floor and routes the resolution to the wrong track.
2. **§6.2 line 376 — fix ADR-00NN+3 hook names.** "lease, fence, restart" → "IdempotencyKey, on_leader_*, dedup_window" per §3.1 component 7 + §5.1.5 + 03-scope-decision §1.7. Current naming is unsourced; will propagate into ADR drafting if not corrected.
3. **§4.3.2 line 229 — reword "CP3 §6 records the chosen path"** to "Tech Spec §9 picks and records; CP3 §6 cross-links once Tech Spec §9 lands" (or add §6.9 retry-scheduler bookmark). Forward-promise unfulfilled.

**Plus 3 🟠** (CHANGELOG line 518 false-completion claim; §6.7 framing "verbatim at §2.12" inaccurate; cluster-mode coordination cascade slot not tracked in §6.6) **and 4 🟡** (§0 status table annotation; §6.2 cite-line-143; §6.2 §3.2-A' mis-cite; §5.1.3 dispatch-pattern line numbers).

**Freeze readiness: NO.** Two 🔴 + structural false-discharge claim block freeze. Single architect revision pass closes all findings; no tech-lead re-litigation needed (all reference/structural/citation, not scope/decision).

**Handoff: architect** for items 1-3 (🔴) + the 3 🟠 + 4 🟡 in single revision pass before CP3 freeze. **No tech-lead intervention required.** **Security-lead** should re-review §6.7 after S-C5 removal to confirm sunset table matches 03c §3 (target: 6 items).
