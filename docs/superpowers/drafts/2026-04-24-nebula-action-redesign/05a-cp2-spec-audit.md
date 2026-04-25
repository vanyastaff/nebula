# CP2 spec-audit — Strategy Document §4 + §5 (nebula-action redesign)

**Auditor:** spec-auditor
**Date:** 2026-04-25
**Document audited:** [`docs/superpowers/specs/2026-04-24-action-redesign-strategy.md`](../../specs/2026-04-24-action-redesign-strategy.md) (398 lines, §4–§5 in scope; §0–§3 audited at [`04a-cp1-spec-audit.md`](04a-cp1-spec-audit.md))
**Read passes:** structural | consistency | external | bookkeeping | terminology

---

## Audit verdict

**REVISE.** §4 + §5 are largely coherent and load-bearing decisions are well-grounded. **However**, one 🔴 BLOCKER (spike target signature drift — Strategy quotes the wrong dispatcher's HRTB signature for `SlotBinding::resolve_fn`), three 🟠 HIGH structural integrity issues (duplicated CHANGELOG/Handoffs headers, missing §3.4 OUT-row promised in §4.3.1, two §4 forward-references claim §2.11 amendments that the CHANGELOG itself documents as deferred), and several 🟡 MEDIUM citation drifts warrant a single architect revision pass before CP2 lock.

The spike-signature 🔴 is critical because the spike plan is the thing the Tech Spec gates on; an inaccurate signature in the spike target will either propagate into the spike scratch crate or force the spike to silently rewrite its own goal.

---

## §4 cross-section consistency findings

### ✅ §4.1 locked decision A' aligns with §3.1 components 1-8 + scope decision §1
§4.1 line 191 ("A' is the chosen direction... ratified at Phase 2") cites scope decision §1 — verified. §4.1 line 193 explicitly defers component re-derivation to §3.1. No silent additions or drops.

### ✅ §4.2 paths (a/b/c) align with §3.2 B'+ contingency conditions
§4.2 path (c) line 203 explicitly cites "B'+ surface commitment" + "scope-contained internal bridge" + "deleted when credential CP6 internals land" — coheres with §3.2 line 137 distinction between B'+ scope-contained vs B structural. §4.2 line 205 "Activation precondition for path (c)" cites both §3.2 conditions ((1) `nebula-credential` placement; (2) committed cascade slot) verbatim. **No drift.**

### ✅ §4.4 security must-have floor — exact verbatim match with §2.12
Items 1-4 in §4.4 lines 248-251 are character-by-character identical to §2.12 lines 101-104, including the parenthetical attributions (`feedback_no_shims.md`; security-lead 03c §1 VETO on shim form), the `redacted_display()` helper reference, the closes-S-C5 framing on item 4. **Verbatim claim holds.**

### ✅ §4.3.1 cited file paths verified in code
- `stateless.rs:313-322` — verified (HRTB execute fn with `'life0/'life1/'a` lives at exactly 313-322).
- `stateful.rs:461-472` — verified (HRTB execute fn at 461-472).
- `trigger.rs:328-381` — verified (start at 328-335, stop at 346-353, handle_event at 373-381).
- `resource.rs:83-106` — verified (configure at 83-91, cleanup at 98-106).

### 🔴 BLOCKER — §5.2.1 spike target signature drifts from cited authority
**Quote (line 310):** "Does `SlotBinding::resolve_fn: for<'ctx> fn(&'ctx CredentialContext<'ctx>, &'ctx CredentialId) -> BoxFuture<'ctx, Result<RefreshOutcome, RefreshError>>` (per credential Tech Spec §3.4 line 869) compile..."

**Evidence:**
- Credential Tech Spec line 869 states the `SlotBinding::resolve_fn` signature is: `for<'ctx> fn(&'ctx CredentialContext<'ctx>, &'ctx SlotKey) -> BoxFuture<'ctx, Result<ResolvedSlot, ResolveError>>`.
- Credential Tech Spec lines 1835-1838 give `RefreshDispatcher::refresh_fn` as: takes `&'ctx CredentialId`, returns `Result<RefreshOutcome, RefreshError>`.

The Strategy spike target conflated the two HRTB signatures: it claims to validate `SlotBinding::resolve_fn` and cites Tech Spec §3.4 line 869, but writes the **`RefreshDispatcher::refresh_fn`** parameter type and return type. The two patterns share the *shape* (HRTB fn-pointer + `BoxFuture`) but have different parameter types (`SlotKey` vs `CredentialId`) and different return types (`ResolvedSlot/ResolveError` vs `RefreshOutcome/RefreshError`).

**Impact:** The spike target is the load-bearing artefact for Phase 6 Tech Spec writing. A signature mismatch between the spike target and the cited authority means: (a) the spike scratch crate may end up validating the wrong HRTB shape; (b) when Tech Spec §7 writers cite the spike output as authority for `SlotBinding::resolve_fn`, they will encounter contradictions vs credential Tech Spec §3.4. CP1 spec-auditor verified the §6.1 / §15.12.3 reference split — this is a different drift in the same family, not a re-emergence of CP1 finding.

**Suggested fix:** Replace line 310's signature with the §3.4-line-869 verbatim form: `for<'ctx> fn(&'ctx CredentialContext<'ctx>, &'ctx SlotKey) -> BoxFuture<'ctx, Result<ResolvedSlot, ResolveError>>`. If architect's intent is for the spike to validate **both** dispatchers (resolve + refresh), split into two questions: Q1 SlotBinding::resolve_fn (resolve-site), Q2 RefreshDispatcher::refresh_fn (refresh-site), each citing its respective Tech Spec line.

**Severity:** 🔴 BLOCKER — load-bearing artefact (spike target) misaligned with cited authority; will propagate to scratch crate or force silent goal rewrite.

### 🟠 HIGH — §4.3.1 line 219 promises a §3.4 OUT-row that doesn't exist
**Quote:** "`#[trait_variant::make]` adoption (rust-senior 02c §6 line 362-380) is **scoped-out** as a separate Phase 3 redesign decision... **Out-of-scope row added to §3.4 in CP2 CHANGELOG.**"

**Evidence:**
- §3.4 OUT-of-scope table (lines 168-181) is **unchanged** from CP1: 12 rows, none mentioning `#[trait_variant::make]` or `trait_variant` adoption.
- CHANGELOG line 371 states: "§3.4 OUT row to be added in CP3 if not pre-empted." — i.e., the row is explicitly NOT in CP2.

**Impact:** §4.3.1 forward-promises a §3.4 row that the document's own CHANGELOG documents as deferred. Reader following §4.3.1's pointer to §3.4 will not find the trait_variant row.

**Suggested fix:** Either (a) add the row to §3.4 in this CP2 revision (2 lines: "`#[trait_variant::make(Handler: Send)]` adoption | Phase 3 handler-redesign cascade | Per rust-senior 02c §6 line 362-380; would break public `*Handler` trait surface"), or (b) reword §4.3.1 line 219 from "Out-of-scope row added to §3.4 in CP2 CHANGELOG" to "Out-of-scope row deferred to CP3 §3.4 amendment per CHANGELOG" to match the CHANGELOG's own deferral.

**Severity:** 🟠 HIGH — internal contradiction between §4.3.1 promise and §3.4 actual content + CHANGELOG.

### 🟠 HIGH — §4.3.1 + §4.4 forward-promise §2.11 amendment that's actually deferred
**Quote 1 (§4.3.1 line 217):** "Tech-lead's CP1 review (line 39-42) flagged `feedback_idiom_currency.md` as 'mandatory if CP2 §4 picks up the HRTB modernization as in-scope' — **§2.11 amendment in CP2 CHANGELOG cites it explicitly**."

**Quote 2 (§4.4 line 253):** "Per `feedback_observability_as_completion.md` (typed error + trace span + invariant check are DoD), items 2-3 in particular must ship with trace spans + invariant checks, not as follow-up. **§2.11 amendment in CP2 CHANGELOG cites this.**"

**Evidence:**
- §2.11 (lines 91-97) lists exactly five feedback memories: `feedback_hard_breaking_changes`, `feedback_boundary_erosion`, `feedback_active_dev_mode`, `feedback_adr_revisable`, `feedback_no_shims`. **Neither `feedback_idiom_currency` nor `feedback_observability_as_completion` is cited there.**
- CHANGELOG line 375 states: "§2.11 amendment-**pending** — `feedback_idiom_currency.md` cited as load-bearing for §4.3.1 HRTB modernization; `feedback_observability_as_completion.md` cited as load-bearing for §4.4 security must-have floor items 2-3. **CP3 may roll into §2.11 explicit citations.**"

**Impact:** Both §4.3.1 and §4.4 confidently claim "§2.11 amendment in CP2 CHANGELOG cites it explicitly" as if the citation lands in §2.11. The CHANGELOG itself flags this as "amendment-pending" deferred to CP3. Reader following the §2.11 pointer expects to find the two new feedback memories cited there; they are not. Tech-lead 04b line 101 also flagged this exact citation as needing to land in §2.11.

**Suggested fix:** Either (a) actually amend §2.11 in CP2 by adding two bullet points for `feedback_idiom_currency.md` (cited from §4.3.1) and `feedback_observability_as_completion.md` (cited from §4.4), then update CHANGELOG line 375 to "§2.11 amended" rather than "amendment-pending"; or (b) reword both §4.3.1 line 217 and §4.4 line 253 to match the CHANGELOG's own framing: "§2.11 amendment-pending; CP3 to roll citation into §2.11 explicitly."

**Severity:** 🟠 HIGH — two §4 forward-promises do not resolve to claimed content; document contradicts itself across §4 / §2.11 / CHANGELOG.

### 🟠 HIGH — Duplicated `### CHANGELOG` and `### Handoffs requested` headers create structural ambiguity
**Quote (lines 366, 384):** Two identically-titled `### CHANGELOG (since previous checkpoint)` headers; (lines 377, 394) two identically-titled `### Handoffs requested` headers.

**Evidence:**
- Lines 366-376 = CP2 CHANGELOG (newest, listed first, distinguishable only by leading "CP2 single-pass append" body line)
- Lines 384-393 = CP1 CHANGELOG (older, listed second, "CP1 single-pass iteration" body line)
- Lines 377-383 = CP2 Handoffs (newest)
- Lines 394-398 = CP1 Handoffs (older)

**Impact:** TOC-equivalent navigation (any tool that lists section headers) cannot distinguish between the two `### CHANGELOG` blocks; readers scrolling to "Handoffs requested" hit CP2 first and may not realize CP1 handoffs are also archived below. Per CP1 audit precedent, the credential Strategy uses single CHANGELOG with chronological subsections — the action Strategy is the outlier here.

**Suggested fix:** Rename the two CHANGELOG headers to `### CHANGELOG (CP2 — since CP1 lock)` and `### CHANGELOG (CP1 — since initial draft)`; same convention for `### Handoffs requested (CP2)` / `### Handoffs requested (CP1)`. Alternative: collapse archived CP1 CHANGELOG into a single `### CHANGELOG history` section listing both.

**Severity:** 🟠 HIGH — structural integrity issue; section heading uniqueness is a basic invariant.

### 🟡 MEDIUM — §0 status table line 27 still labels DRAFT CP1 as "(this revision)"
**Quote (line 27):** "**DRAFT CP1** (this revision) | §0–§3 only | ..."
**Evidence:** Frontmatter line 2 says `status: DRAFT CP2`. The "(this revision)" annotation was correct at CP1 lock but should have moved to line 28 ("DRAFT CP2") at the CP2 append.
**Impact:** Reader inspecting §0 to determine current state encounters CP1 marked as current, contradicting the frontmatter.
**Suggested fix:** Move "(this revision)" annotation from line 27 to line 28.
**Severity:** 🟡 MEDIUM — readability; not load-bearing for any decision.

### 🟡 MEDIUM — §4.3.1 line 215 attributes 30-40% LOC figure + "Phase 3 redesign" quote to wrong 02c sub-section
**Quote:** "rust-senior 02c §6 (line 386: 'Phase 3 redesign decision with clear LOC payoff') quantifies the change at ~30-40% LOC reduction across..."

**Evidence:**
- The "30-40%" figure lives at 02c line **439** (§8 Top-N findings table, row 2 reasoning column: "Single `'a` lifetime + type alias (`BoxFut<'a, T>`) would tighten ~30-40% of boilerplate"). 02c line 386 (§6 summary) says "8 lines per handler trait" + "~40-60 lines across the handler family" — never quotes a percentage.
- The "Phase 3 redesign decision with clear LOC payoff" quote at 02c line 386 refers to **`#[trait_variant::make]` adoption** (which §4.3.1 explicitly scopes OUT — line 219). Applying this quote to the in-scope single-lifetime change inverts 02c's framing: 02c says single-lifetime + type-alias is tightenable "without touching semver" (i.e., NOT a Phase 3 redesign), while `trait_variant` adoption IS a Phase 3 redesign.

**Impact:** Reader following §4.3.1's citation to 02c §6 line 386 finds the "Phase 3 redesign decision" quote attached to the OUT-of-scope item; the IN-scope LOC figure is at line 439 in §8. Substantively the LOC figure is real and the in-scope decision is correct; only the citation routing is misleading.

**Suggested fix:** Split the citation: "rust-senior 02c §8 line 439 (~30-40% LOC reduction) + 02c §6 line 358 (cuts 8 lines per handler trait, dyn-safety preserved). Note that §6 line 386's 'Phase 3 redesign decision with clear LOC payoff' quote refers to `#[trait_variant::make]` adoption, which §4.3.1 scopes OUT."

**Severity:** 🟡 MEDIUM — citation imprecision; the underlying claim is correct.

### 🟡 MEDIUM — §4.3.1 line 215 "per 02c line 39" / "per 02c line 357" off-by-one
**Quote:** "Rust 1.95 elision rules accept the single-lifetime form (per 02c line 39); dyn-safety is preserved (per 02c line 357)..."
**Evidence:**
- 02c line 39 is empty (between paragraph break and "What *is* dated, however..." at line 40). The elision claim ("Rustc's elision rules accept this since 1.51 (RFC 2115)") lives at 02c line 55.
- 02c line 357 is empty (between code block end at 356 and "passes dyn-safety checks" at 358).
**Suggested fix:** Update to `per 02c line 55` (elision) and `per 02c line 358` (dyn-safety).
**Severity:** 🟡 MEDIUM — off-by-one citation drift, both correct after adjustment.

### ✅ §4.3.2 retry-scheduler principle aligns with §2.4 + §2.3 + tech-lead Phase 1 ratification
§4.3.2 line 223-228: gates Retry+Terminate symmetrically, no parallel retry surface (cites §2.4), applies tech-lead Phase 1 solo-decided call (cites 02-pain-enumeration §7). Verified — §2.3 line 70 cites the `Retry` discipline; §2.4 line 72 prohibits parallel retry surface; 02-pain-enumeration §7 ratifies the solo-decided calls. **No drift.**

### ✅ §4.3.3 codemod transform 1 cites verified line range
§4.3.3 transform 1 cites `02c-idiomatic-review.md §2 line 134-141` for `parameters = Type` broken `.with_parameters` finding. Verified — 02c lines 134-141 contain the `🔴 WRONG` finding for the broken emission with the exact code block referenced.

---

## §5 open-items closure verification

### ✅ All 5 carried-forward open items have explicit resolution slots
- **§5.1.1** ActionContext API location → "before Tech Spec §7 (Interface section) writing begins." Owner: architect + credential Tech Spec author. Resolution slot: real (Tech Spec §7 is the explicit interface section; can be physically resolved).
- **§5.1.2** `redacted_display()` hosting crate → "Phase 6 Tech Spec §4 (Security section)." Owner: architect + security-lead. Resolution slot: real.
- **§5.1.3** Credential Tech Spec §7.1 line numbers → "CP3 draft." Owner: architect. Resolution slot: real (CP3 §6 will cite the dispatch pattern).
- **§5.1.4** B'+ activation signal enumeration → "CP3 §6 (post-validation roadmap)." Owner: architect (draft) + tech-lead (ratify). Resolution slot: real.
- **§5.1.5** Cluster-mode hooks final shape → "Tech Spec §7 Interface section." Owner: architect (Tech Spec §7) + tech-lead (ratify). Resolution slot: real.

**No "TODO" placeholders.** Every item has named owner + named resolution location + named deadline.

### ✅ Source citations on each carry-forward item
- §5.1.1 cites "Spec-auditor's CP1 finding 2" + "Was CP1 §1(a)" — verified at 04a line 117-122 (the 🟠 REVISE-RECOMMENDED for `ctx.credential::<S>` API location).
- §5.1.2 cites "was CP1 §2.6" — verified at 04a line 106 ("§2.6 — `redacted_display()` crate location").
- §5.1.3 cites "was CP1 §2.8" — verified at 04a line 107 ("§2.8 — Credential Tech Spec §7.1 line numbers").
- §5.1.4 cites "Tech-lead CP1 review pre-load hint" — verified at 04b (tech-lead CP1 review, condition on B'+ contingency activation signals).
- §5.1.5 cites "tech-lead CP2 hint" — verified at 04b line 113-115 (must-lock items for CP2).

### ✅ §5.1 closed-by-CP2 inventory matches §4 sub-decisions
Line 261: "§3.1 component 5 `*Handler` HRTB scope (§4.3.1 in-scope); §3.4 `unstable-retry-scheduler` retire-vs-wire (§4.3.2 principle locked, Tech Spec §9 picks path)" — verified, both items closed in §4.3.

### 🟡 MEDIUM — §5.1 line 261 closed-by-CP2 includes §1 dx-tester item without cross-link
**Quote:** "§1 dx-tester figure citation (tech-lead CP1 review confirmed acceptable)."
**Evidence:** 04b (tech-lead CP1 review) does discuss dx-tester evidence at line 38 area but doesn't have an explicit "32-min figure acceptable" sentence with a clean line anchor. This is a minor traceability gap; the closure claim is plausible from tech-lead's overall RATIFY-WITH-NITS verdict.
**Suggested fix:** Either cite 04b line N where tech-lead acceptance is explicit, or rephrase: "no objection raised at CP1 lock per tech-lead 04b review."
**Severity:** 🟡 MEDIUM — cosmetic; the closure is real.

### ✅ §5 "Open items raised this checkpoint" list (lines 358-364) is complete
Seven bullets covering §4.3.2, §4.3.3, §5.1.1, §5.1.2, §5.1.4, §5.1.5, §5.2 — each cross-linked to the body section. **No body-section open question is missing from this list** (the 🟠 promise-doesn't-resolve issues in §4.3.1 / §4.4 are not raised here as open items, but they are *internal contradictions* not open questions).

---

## Spike plan structural findings

### ✅ All four required components present
- **Iter-1** (§5.2.2 lines 313-321) — minimum compile shape, 3 compile-fail probes named, DONE criteria explicit.
- **Iter-2** (§5.2.3 lines 323-333) — composition + cancellation + perf, 3 actions named (Stateless+Bearer / Stateful+OAuth2 / Resource+Postgres), DONE criteria explicit.
- **Aggregate DONE** (§5.2.4 line 337) — explicit aggregation of iter-1 + iter-2 criteria + cancellation + expansion perf.
- **Budget** (§5.2.4 line 341) — "max 2 iterations per cascade prompt"; failure mode is non-blocking; surfaces narrowing of §4.2 path choice.

All four required CP2 spike plan components named.

### ✅ Spike scope ties to §4.3 sub-decisions
§5.2 line 304 ("HRTB fn-pointer + `SchemeGuard<'a, C>` cancellation drop-order") explicitly addresses §4.3.1 (HRTB modernization in-scope drives need for HRTB compile validation) + §4.4 item 4 (cancellation-zeroize test, security must-have floor). Iter-2 §5.2.3 Action B (StatefulAction + OAuth2 + refresh) probes RefreshDispatcher integration that §4.3.3 codemod transform 2 implicitly assumes. **Tied.**

### ✅ §5.2.5 spike → Tech Spec interface lock plan complete
Three named output artefacts (`NOTES.md`, `final_shape_v2.rs`, test artefacts), each with role-in-Tech-Spec-§7 locked. Sequencing line 352 ("Spike runs in Phase 4 (parallel with CP3 drafting); Phase 6 Tech Spec writing does not begin until spike DONE criteria met") gates Tech Spec writing on spike completion. **Cleanly structured.**

### ✅ Iter-1 + Iter-2 mirror credential Strategy §6.1 pattern (with one nuance)
§5.2 follows credential Strategy §6.1's iter-1/iter-2 + perf-bench + worktree-isolated pattern. **Note:** §5.2.4 line 339 says "Pattern follows credential Strategy §6.1 spike iter-1/2/3 worktree pattern." Per CP1 audit (which validated this same drift), iter-3 is in credential **Tech Spec** §15.12.3, not credential Strategy §6.1. The reference in CP2 §5.2.4 retains the same minor drift CP1 already flagged — flag for consistency with 04a's recommended split-citation pattern.

### 🟡 MEDIUM — §5.2.4 "iter-1/2/3 worktree pattern" inherits CP1's iter-3 citation drift
**Quote (line 339):** "Pattern follows credential Strategy §6.1 spike iter-1/2/3 worktree pattern."
**Evidence:** Credential Strategy §6.1 documents iter-1 + iter-2 only; iter-3 evidence lives in credential **Tech Spec** §15.12.3 (per 04a CP1 audit finding 1, which architect already addressed in §3.3 by splitting the citation).
**Suggested fix:** Apply the same fix CP1 received: "Pattern follows credential Strategy §6.1 spike iter-1/2 + credential Tech Spec §15.12.3 iter-3 worktree pattern."
**Severity:** 🟡 MEDIUM — inherited drift; same fix shape as CP1 §3.3.

---

## Forward-ref bookkeeping to CP3 §6

### ✅ CP3 §6 references are marked deferred, not dangling
- §4.3.2 line 228: "CP3 §6 records the chosen path in the post-validation roadmap." — Deferral explicit.
- §4.4 + CHANGELOG: §2.11 amendment "CP3 may roll into §2.11 explicit citations." — Deferral explicit (but see 🟠 HIGH finding above on the §4 in-body promises being inconsistent with this deferral).
- §5.1.3 line 281: "Pin line numbers when CP3 §6 cites the dispatch pattern." — Deferral explicit.
- §5.1.4 line 287: "CP3 §6 (post-validation roadmap) must record..." — Deferral explicit, with three sub-items enumerated.
- §3.2 line 141 (pre-existing from CP1): "CP3 §6 records the contingency activation criteria." — Deferral explicit.
- §3.2 line 146: "CP2 §4 / CP3 §6 must verify the slot is committed before ratifying any B'+ contingency activation." — Deferral explicit.
- "Open items raised this checkpoint" line 358 + 362: explicit CP3 §6 routing.

**All seven CP3 §6 forward-references are tagged as deferred with explicit handoff to CP3 author. None dangling.**

### 🟡 MEDIUM — CP3 §6 promise count grows to ~8 sub-items; CP3 will need disciplined inventory
Forward-promises now span: B'+ contingency activation criteria; B'+ activation signal enumeration; B'+ rollback path; B'+ sunset commit; retry-scheduler chosen path; cluster-mode coordination cascade scheduling commitment; possibly §2.11 amendment roll-in.
**Suggested fix:** CP3 author should treat §6 as a checklist mapping each promise to a specific sub-section before drafting prose. Not a defect in CP2 — flag for CP3 setup hygiene.
**Severity:** 🟡 MEDIUM — proactive bookkeeping flag, not a CP2 defect.

---

## Status header consistency

### 🟡 MEDIUM (already flagged above) — Frontmatter `DRAFT CP2` vs §0 status table line 27 stale "(this revision)" annotation
Documented in §4 cross-section findings. Single character fix (move "(this revision)" parenthetical from CP1 row to CP2 row).

---

## Coverage summary

- **Structural:** REVISE — 1 🔴 (spike target signature), 1 🟠 (duplicated CHANGELOG/Handoffs headers), 1 🟡 (status table stale annotation).
- **Consistency:** REVISE — 2 🟠 (§3.4 OUT-row promise doesn't land; §2.11 amendment promise doesn't land), 2 🟡 (citation off-by-one to 02c lines 39/357; LOC-figure §-attribution).
- **External verification:** PASS WITH NITS — code paths in §4.3.1 verified; §4.4 verbatim claim against §2.12 verified character-by-character; cited credential Tech Spec sections all resolve to claimed content; 02c LOC figure verified at line 439 (just attributed to wrong sub-section); 02-pain-enumeration §7 ratification verified.
- **Bookkeeping:** PASS WITH NIT — 5 carried-forward open items all have real resolution slots; 1 🟡 (§1 dx-tester closure citation); 7 CP3 §6 forward-refs all marked deferred.
- **Terminology:** PASS — option labels stable; CP6 vocabulary stable (`CredentialRef<C>`, `SlotBinding`, `SchemeGuard<'a, C>`, `SchemeFactory<C>`); "narrow declarative rewriting contract" used consistently with §3.1 component 2 and §5.2.2 probe 3.
- **Definition-of-done (PRODUCT_CANON §17):** N/A for Strategy CP2 (DoD applies to Tech Spec / impl phase).

---

## Summary for orchestrator

**Verdict: REVISE.** §4 + §5 are structurally well-organized and all 5 carried-forward open items + spike plan components present. **Iterate before CP2 lock.** Three findings drive the verdict:

1. **🔴 BLOCKER §5.2.1 line 310** — spike target writes `RefreshDispatcher::refresh_fn` signature (`&CredentialId` / `RefreshOutcome`) while citing credential Tech Spec §3.4 line 869 which defines `SlotBinding::resolve_fn` (`&SlotKey` / `ResolvedSlot`). Spike output is the input to Tech Spec §7 — drift here propagates.
2. **🟠 §4.3.1 + §4.4 forward-promises don't land** — both claim "§2.11 amendment in CP2 CHANGELOG cites it explicitly"; CHANGELOG itself documents the amendment as "pending — CP3 may roll into §2.11." Plus §4.3.1 promises a §3.4 OUT-row that doesn't exist.
3. **🟠 Duplicated `### CHANGELOG` and `### Handoffs requested` headers** — section heading uniqueness violated; rename with CP1/CP2 disambiguators.

**Iterate-yes.** Single architect pass; no tech-lead re-litigation needed (all findings are reference/structural, not scope/decision).

**Handoff: architect** for items 1-3 + the 4 🟡 MEDIUM nits in single revision.
