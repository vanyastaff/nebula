# CP1 spec-audit — Strategy Document §0–§3 (nebula-action redesign)

**Auditor:** spec-auditor
**Date:** 2026-04-25
**Document audited:** [`docs/superpowers/specs/2026-04-24-action-redesign-strategy.md`](../../specs/2026-04-24-action-redesign-strategy.md) (197 lines, §0–§3 in scope)
**Read passes:** structural | consistency | external | bookkeeping | terminology

---

## Audit verdict

**PASS-WITH-NITS.** Strategy is internally coherent and externally accurate on the load-bearing claims (option pick aligns with scope decision; security must-have floor verbatim from 03c; canon citations resolve to correct sections). Three reference-resolution drifts and two open-item gaps warrant a single architect pass before CP2 begins. None of the findings affects implementer decisions; none requires tech-lead re-litigation.

---

## Cross-section consistency findings

### ✅ §3 option pick aligns with §1 problem framing + §2 constraints
§1 frames four convergent drift patterns (a/b/c/d); §2 enumerates 12 invariants any direction must honor; §3.1 components 1–8 exhaustively map to those drift patterns and honor each §2 constraint. Components in §3.1 line 116-125 are a 1:1 match with [`03-scope-decision.md`](03-scope-decision.md) §1 components 1–8 (cross-checked: credential CP6 vocab, action CP6 adoption, engine wiring, security hardening, Phase 1 solo calls, plugin migration, cluster-mode hooks, workspace hygiene). **No silent additions, no silent drops.**

### ✅ §3.2 B'+ contingency criteria cohere with §3.1 / §3.3
B'+ runner-up framing (§3.2) cleanly distinguishes "scope-contained internal bridge" from B's "structural engine bridge" (§3.3). Contingency activation criteria in §3.2 line 141 explicitly defer to CP3 §6 — consistent with the §0 reading order ("§6 = post-validation roadmap, CP3").

### ✅ §0 status progression cleanly distinguishes amendment vs supersession
§0 line 34 documents the inline-amendment pattern with a credential Strategy §0 precedent; §0 line 36 establishes the authority chain. Both load-bearing for §3.3 C' rejection rationale (which cites supersession bar).

### 🟡 NIT — §1 line 42 mis-classifies a Phase 0 finding's section
**Quote:** "Phase 0 [`01-current-state.md`](...) §1, §2.S1, lines 24, 117 of [`PRODUCT_CANON.md`](...) §4.2"
**Evidence:** `grep -nE "^##? " docs/superpowers/drafts/2026-04-24-nebula-action-redesign/01-current-state.md` shows S1 (`Canon §3.5 / §0.2 drift`) lives at line 94 inside `## 3. Major structural findings (🟠)`, not `## 2. Critical findings (🔴)` (which holds C1–C4).
**Impact:** Reader following `§2.S1` lands in the C1–C4 cluster, not the S1 finding the sentence is describing. Cosmetic but mis-routes the reader.
**Severity:** 🟡 NIT — single character fix (`§2.S1` → `§3.S1`).

---

## Reference resolution findings

### 🟠 REVISE-RECOMMENDED — Strategy §3.3 cites credential Strategy §6.1 for spike iter-3, but §6.1 only documents iter-1 + iter-2
**Quote (Strategy line 155):** "Spike iter-1 ([`2026-04-24-credential-redesign-strategy.md`](...) §6.1, commit `acfec719`), iter-2 (`1c107144`), iter-3 (`f36f3739`) validated the chosen shape compiles..."
**Evidence:**
- `grep -n "f36f3739\|iter-3" docs/superpowers/specs/2026-04-24-credential-redesign-strategy.md` returns no matches.
- `grep -n "f36f3739" docs/superpowers/specs/2026-04-24-credential-tech-spec.md` confirms iter-3 evidence lives in **Tech Spec** §15.7 line 3503 (lifetime-gap refinement), §15.12.3 line 3689 (Gate 3 sub-trait validation), and §0.4 sign-off matrix line 3616.
- Credential Strategy §6.1 line 519-543 explicitly says "Spike outcomes (iter-1 + iter-2 consolidated)" — closes 5 of 5 spike scope questions; iter-3 is post-Strategy-freeze validation living in Tech Spec.
**Impact:** Reader following the Strategy §3.3 citation will not find iter-3 evidence in the cited document. Substantively the iter-3 commit DOES exist and validates as claimed; only the cited location is wrong.
**Suggested fix:** Replace "§6.1" with "§6.1 (iter-1, iter-2) + Tech Spec §15.12.3 (iter-3 Gate 3)" or split the citation: "iter-1/iter-2 per credential Strategy §6.1; iter-3 per credential Tech Spec §15.12.3 line 3689".
**Severity:** 🟠 REVISE-RECOMMENDED — claim is true; citation routes reader to wrong document.

### 🟡 NIT — §1 line 42 PRODUCT_CANON line 24 reference is mis-attributed
**Quote:** "lines 24, 117 of [`PRODUCT_CANON.md`](...) §4.2"
**Evidence:** Line 24 of PRODUCT_CANON.md is `**[L3 Convention]**` in §0.1 Layer legend, not §4.2. Line 117 IS the §4.2 [L1] safety rule (correctly cited).
**Suggested fix:** Drop "24, " — the §4.2 anchor is line 117.
**Severity:** 🟡 NIT.

### ✅ All other PRODUCT_CANON line citations resolve
- §2.1 cites §3.5 line 82 — verified (line 82 is the action trait family bullet).
- §2.2 cites §0.2 line 27 — verified (heading at line 27).
- §2.3 cites §4.5 line 131 — verified (heading at 131; rule at 133, acceptable approximation).
- §2.4 cites §11.2 line 286-298 — verified (heading at 286, retry-status table runs 290–298 with the canonical-retry-surface row at 295).
- §2.5 cites §11.3 line 300 — verified (heading at 300).
- §2.6 cites §12.5 line 386-391 — verified (heading at 386, last bullet at 391).
- §2.7 cites §12.6 line 393-397 — verified (heading at 393, isolation-roadmap bullet ends at 397).
- §2.10 cites §12.1 line 358 — verified (heading at 358).

### ✅ All credential Tech Spec line citations resolve
- §2.8 line 82 cites Tech Spec §2.7 line 486-528 — verified (heading at 485; "Decision — rewrite silently" at 489; example Output ends at 528 right before §2.8 at 530).
- §2.8 line 83 cites Tech Spec §3.4 line 807-939 — verified (heading at 807; §3.4 ends at 939 right before §3.5 at 941).
- §2.8 line 85 cites Tech Spec §15.7 line 3383-3517 — verified (heading at 3383; lifetime-gap refinement at 3503-3516; §15.7 ends at 3518 before §15.8 at 3520).
- §2.8 line 84 references Tech Spec §7.1 without line numbers — architect self-flagged this as CP2-pending in open items (§7.1 heading is at line 1806).

### ✅ ADR-0035 citations resolve
- §2.9 cites ADR-0035 line 65-108 (canonical form) and §2 line 124-135 (Pattern 4 amendment) — verified (canonical form §1 at 65; Pattern 4 amendment 2026-04-24-C at line 134).

### ✅ All sub-report paths resolve
01-current-state, 01a-code-audit, 01b-workspace-audit, 02-pain-enumeration, 02a-dx, 02b-security, 02c-idiomatic, 02d-architectural, 03-scope-decision, 03a/b/c — all 12 referenced files exist at the cited paths.

### ✅ Cited code paths resolve
`crates/credential/src/contract/any.rs`, `crates/action/src/result.rs`, `crates/action/src/context.rs`, `crates/action/src/stateless.rs`, `crates/action/macros/src/action_attrs.rs`, `crates/action/src/lib.rs` all exist. Line ranges (`context.rs:637-643`, `stateless.rs:356-383`, `action_attrs.rs:129-134`, `result.rs:207-219`) verified to contain the claimed text.

---

## Terminology coherence findings

### ✅ "Option A'" / "B'+" / "B'" / "C'" used with consistent semantics
Strategy uses `A'` (chosen), `B'+` (hybrid runner-up), `B'` (action-only fixes, rejected), `C'` (spec revision, rejected) consistently across §0, §1, §2.11, §3.1, §3.2, §3.3. Matches scope decision §1 + §4 + §6 framing exactly. No primed-vs-unprimed drift.

### ✅ "CP6 vocabulary" used consistently
§1 line 42 ("`CredentialRef<C>` typed handle, `AnyCredential` ... `SchemeGuard<'a, C>` ... `SchemeFactory<C>`") and §3.1 line 118 (same enumeration) use identical type names; §2.8 line 82-85 references the exact same vocabulary. No drift.

### ✅ "phantom-shim" naming consistent with ADR-0035
Strategy §2.9 uses "phantom-shim canonical form"; §1 line 60, §3.3 line 155 use "phantom-shim". ADR-0035 itself titled "phantom-shim-capability-pattern". Consistent.

### ✅ "SlotBinding" semantics consistent across §1, §2.8, §3.1
Each occurrence pairs with the HRTB `for<'ctx> fn(...) -> BoxFuture<'ctx, _>` `resolve_fn` clause. No drift.

### 🟡 NIT — "narrow declarative rewriting contract" not yet in glossary
Strategy §3.1 line 119 introduces the term "narrow declarative rewriting contract" (per tech-lead §2 architectural coherence constraint). It does not appear in [`docs/GLOSSARY.md`](../../GLOSSARY.md) — this is acceptable because it is a Strategy-internal phrase, but Tech Spec (Phase 6) will need to surface it as a defined term or replace with concrete signature wording. Flag for CP2 §4 awareness.
**Severity:** 🟡 NIT — pre-glossary term tracking.

### ✅ Acronym discipline
`HRTB` introduced in §1 line 42 alongside spelled-out form `for<'ctx> fn(...) -> BoxFuture`; `RAII` in §2.6, §2.8 paired with explanatory clauses; `DX` (developer experience) used unexpanded but is canonical in `docs/STYLE.md` and `docs/MATURITY.md`.

---

## Open-item bookkeeping

### ✅ Architect self-flagged 2 cross-reference gaps — both tracked as explicit CP2 items
1. **§2.6 — `redacted_display()` crate location:** tracked as "Open items raised this checkpoint" line 183: "The `redacted_display()` helper does not yet exist in `nebula-error` or `nebula-log`; Tech Spec must specify which crate hosts it. Possibly an open glossary item." **Verified absent:** `grep -rn "redacted_display" crates --include="*.rs"` returns zero matches; only doc-level references in 03-scope-decision.md, 03c-security-lead-veto-check.md, and the Strategy itself. Tracked. ✅
2. **§2.8 — Credential Tech Spec §7.1 line numbers:** tracked at line 184: "Credential Tech Spec §7.1 is referenced as the authoritative shape for `RefreshDispatcher::refresh_fn` HRTB pattern; a precise line-number citation requires reading §7 in full ... CP2 should pin §7.1 line numbers when CP2 §4 recommendation cites the dispatch pattern." Tracked. ✅

### ✅ Other open items (4 additional) tracked
- §1 dx-tester time-to-first-compile measurements canonicality (line 182)
- §3.1 component 5 `*Handler` HRTB modernization in-scope vs scoped-out (line 185)
- §3.4 `unstable-retry-scheduler` retire-vs-wire choice (line 186)
- §0 freeze policy CP1/CP2/CP3 vs named-gates (line 187)

All 6 open items are explicit, tracked, and routed to CP2 for resolution. Architect's bookkeeping is complete.

### 🟠 REVISE-RECOMMENDED — §1(a) line 46 raises a CP2-pending item without flagging it
**Quote:** "v2 spec's `ctx.credential::<S>(key)` / `credential_opt::<S>(key)` (per credential Tech Spec §2.7 line 487-516) do not exist (3 alternative methods, none match)."
**Issue:** Tech Spec §2.7 line 487-516 covers `#[action]` macro **rewriting** input/output (Pattern 2 capability-bound + Pattern 1 concrete examples). It does NOT define the `ctx.credential::<S>(key)` / `credential_opt::<S>(key)` ActionContext API; that API surface is sketched in [`03-scope-decision.md`](03-scope-decision.md) §1.2 ("`ctx.credential::<S>(key)` / `credential_opt::<S>(key)` per Tech Spec §2.7") which inherits the same imprecise pointer.
**Impact:** Tech Spec §2.7 line 487-516 is the macro translation contract, not the ActionContext API contract. The two contracts compose, but a reader looking for the latter at §2.7 will not find a `ctx.credential::<S>(key)` signature there — only the macro rewrite.
**Suggested fix:** CP2 should pin the actual `ActionContext::credential` signature location in the Tech Spec (likely §2.6 or §3 — separate from §2.7's macro translation). Add to the open-items list as a third cross-reference gap to resolve in CP2.
**Severity:** 🟠 REVISE-RECOMMENDED — citation imprecision, but resolvable in CP2 without re-litigation; both Strategy and scope-decision share the imprecision.

---

## Nits (non-blocking stylistic)

### 🟡 §2.6 line 76 has nested "§3 of §3" cross-reference
Quote: "(security must-have §3 of [`03-scope-decision.md`](...) §3)" — this resolves correctly (item #3 of section 3) but reads awkwardly. Suggest: "(security must-have §3 of [`03-scope-decision.md`](...) §3.3)" or "[03-scope-decision.md §3 item 3]".

### 🟡 §3.1 line 122 says "Phase 1 tech-lead solo-decided calls (ratified):"
Three calls listed match scope-decision §1 component 5 verbatim. The "(ratified)" parenthetical is fine but a bit terse — `02-pain-enumeration.md` §7 line 232 is the canonical ratification ("Three solo-decided tech-lead calls (...) are ratified as Phase 1 outputs"). Could cite that line for traceability.

### 🟢 Code blocks
Strategy contains no fenced code blocks in §0–§3 (architect uses inline backtick prose only). No language-tag finding applies.

### 🟢 §3.4 table is well-formed
12-row OUT-of-scope table has consistent column structure; sub-spec pointers all resolve.

---

## Summary for orchestrator

CP1 §0–§3 PASSes structural audit with 2 🟠 REVISE-RECOMMENDED findings (both reference-routing imprecisions, neither affects scope/option pick) and 4 🟡 NITs.

**Architect should iterate once before CP2 begins.** Single-pass fix bundle:

1. Strategy §3.3 line 155: split iter-3 citation between credential Strategy §6.1 (iter-1/iter-2) and credential Tech Spec §15.12.3 (iter-3 Gate 3). [🟠]
2. Add a third open item to the §15-equivalent open-items list: pin the `ctx.credential::<S>(key)` / `credential_opt::<S>(key)` ActionContext API location in CP6 Tech Spec (§2.7 covers macro rewriting only, not the context method). [🟠]
3. Strategy §1 line 42: drop "line 24" from the PRODUCT_CANON citation (line 24 is §0.1 layer legend, not §4.2); change `§2.S1` → `§3.S1`. [🟡]
4. Optional: tighten §2.6 nested cross-reference and §3.1 ratification trace. [🟡]

**Handoff: architect** for items 1-3 (single revision pass). **No tech-lead intervention required** — all findings are reference/bookkeeping, not scope/decision.

### Coverage summary
- Structural: PASS (1 nit on §1 §-section mis-attribution).
- Consistency: PASS (§3 components ↔ scope decision §1 1:1 match; option-label terminology stable).
- External verification: PASS WITH NITS (1 🟠 on iter-3 citation; 1 🟡 on PRODUCT_CANON line 24; all other line citations and code paths verified).
- Bookkeeping: PASS WITH NIT (1 🟠 on `ctx.credential::<S>` API location not flagged; 6 open items otherwise tracked).
- Terminology: PASS (option labels, CP6 vocabulary, phantom-shim, SlotBinding all stable; one Strategy-internal phrase pre-glossary, acceptable for CP1).
- Definition-of-done (PRODUCT_CANON §17): N/A for Strategy CP1 — DoD applies to Tech Spec / implementation phase per `docs/PRODUCT_CANON.md` §17 and the cascade prompt scope (CP1 = §1 problem + §2 constraints + §3 options).
