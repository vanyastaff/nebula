# Strategy CP1 — Spec-Auditor Review

**Date:** 2026-04-24
**Reviewer:** spec-auditor (subagent dispatch)
**Document audited:** `docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md`
**Checkpoint:** CP1 (§0-§3)
**Commit basis:** `d6cee19f814ff2c955a656afe16c9aeafca16244`
**Read passes:** structural | consistency | external | bookkeeping | terminology

---

## 1. Cross-section consistency findings

### 1.1 §0 description of CP2 / CP3 — ACCURATE
- §0 line 42-44 names CP2 as "§4 decision record + §5 open items" and CP3 as "§6 post-validation roadmap". Matches `03-scope-decision.md §6` artefact-plan row for Phase 3 Strategy ("CP1 §1-§3 → CP2 §4-§5 → CP3 §6 cadence per credential pattern"). ✓
- §0 line 56 reading-order recap is internally consistent with §0 line 42-44 checkpoint path; no content-leak from CP2/CP3 (only structural placeholders).
- §0 line 50 illustrative example "§4.2 said 'parallel dispatch'" refers to content that does not exist yet — clearly framed as a hypothetical clarification example, not a forward reference. ✓

### 1.2 §2.3 ↔ §1.1 (cross-crate contracts ↔ credential rotation surface)
- §2.3 line 165 (Credential Tech Spec §3.6 = "Normative for this redesign") aligns with §1.1 line 73 ("Credential Tech Spec §3.6 designs rotation as a per-resource `on_credential_refresh` method"). ✓
- §2.3 line 170 (revocation extension as open-item for CP2) explicitly cross-references §1.1's gap-narrative without redefining it. ✓

### 1.3 §1.x severity-framing — CONSISTENT
- §1.1-§1.6 each lead with **Symptom / Evidence / Impact** triplet structure. ✓
- Phase 1 severity tags (🔴-1, 🔴-2, 🔴-5, 🔴-6, 🟠-9, 🟠-14, 🟠-15) carried through verbatim from `02-pain-enumeration.md §4` consolidated severity matrix. No re-classification, no severity drift. ✓

### 1.4 §3.1/§3.2/§3.3 ↔ `03-scope-decision.md §1+§2+§3`
- §3.1 (Option A blocked) — agrees with `03-scope-decision.md` framing (security-lead BLOCK on 🔴-1). ✓
- §3.2 (Option B chosen) — agrees with `03-scope-decision.md §4` locked design decisions (§3.6 verbatim, parallel dispatch, observability DoD, warmup amendment, daemon extraction). ✓
- §3.3 (Option C rejected) — agrees with `03-scope-decision.md` deferral pattern (Runtime/Lease, AcquireOptions, Service/Transport merge). ✓

---

## 2. Claim-vs-source verification (spot-check results)

**Attempted:** 17 file:line citations.
**Passed:** 14.
**Failed (off-by-N drift):** 3.

| # | Strategy claim (§ + line) | Citation | Verification | Verdict |
|---|---|---|---|---|
| 1 | §1.1 line 71: `manager.rs:262` reverse-index declared | grep `credential_resources` → line 262 `DashMap<CredentialId, Vec<ResourceKey>>` declaration | ✓ Exact match | PASS |
| 2 | §1.1 line 71: `manager.rs:370` register hardcodes `credential_id: None` | Read line 370 → `credential_id: None,` | ✓ Exact match | PASS |
| 3 | §1.1 line 72: `manager.rs:1378` `todo!()` on refresh | Read line 1378 → `todo!("Implementation deferred to runtime integration")` | ✓ Exact match | PASS |
| 4 | §1.1 line 72: `manager.rs:1400` latent panic on revoke | Read line 1400 → `todo!("Implementation deferred to runtime integration")` | ✓ Exact match | PASS |
| 5 | §1.1 line 71: "write site does not exist anywhere in the codebase" | `grep credential_resources` → 4 matches: declaration (262), init empty (293), 2 reads (1365, 1388). No `.insert()` or `.entry()` call. | ✓ Verified | PASS |
| 6 | §1.2 line 84: `runtime/managed.rs:35` `topology` is `pub(crate)` | Read line 35 → `pub(crate) topology: TopologyRuntime<R>,` | ✓ Exact match | PASS |
| 7 | §1.4 line 108: `manager.rs` is 2101 lines | `wc -l` → 2101 | ✓ Exact match | PASS |
| 8 | §1.4 line 114: `_with` variants at lines 561, 597, 627, 659, 691 | Read each line → `register_pooled_with` (561), `register_resident_with` (597), `register_service_with` (627), `register_transport_with` (659), `register_exclusive_with` (691) | ✓ All five verified | PASS |
| 9 | §1.4 line 115: `register_*` Auth = () bounds at lines 411, 446, 476, 507, 538 | Read each line → all match `R: Resource<Auth = ()>` bound clauses | ✓ All five verified | PASS |
| 10 | §1.5 line 125: `manager.rs:1493-1510` DrainTimeoutPolicy::Abort path | Read 1493-1510 → matches `Abort` branch with `set_phase_all(Ready)` on 1507 then `Err(ShutdownError::DrainTimeout)` on 1509 | ✓ Exact match | PASS |
| 11 | §1.5 line 126: `runtime/managed.rs:93-102` `set_failed` dead-coded | Read 93-102 → `#[expect(dead_code, reason = "callers will land with the recovery-error work")]` + body sets `phase: ResourcePhase::Failed` | ✓ Exact match | PASS |
| 12 | §2.2 line 154: `PRODUCT_CANON.md §3.5` line 80 (Resource definition) | Read line 80 → `**Resource** — long-lived managed object (connection pool, SDK client). Engine owns lifecycle.` | ✓ Exact match | PASS |
| 13 | §2.2 line 156: `PRODUCT_CANON.md §4.5` lines 131-138 | Read 131-138 → §4.5 header at 131, "Public surface exists iff..." at 133, three options at 135-137 | ✓ Range matches | PASS |
| 14 | §2.3 line 174: engine-grep zero hits for `DaemonRuntime/EventSourceRuntime/TopologyTag::Daemon/EventSource` | `grep -rn "DaemonRuntime\|EventSourceRuntime\|TopologyTag::Daemon\|TopologyTag::EventSource" crates/engine/` → zero hits | ✓ Verified | PASS |
| 15 | §1.4 line 112: tech-lead quote "every field is referenced by multiple public methods" cites `02-pain-enumeration.md:82` | Read line 82 → "**Tech-lead priority-call preview:** 'Do NOT split Manager...'" — the cited quote is at **line 76**, not 82 | ✗ Off-by-six | **FAIL** |
| 16 | §1.4 line 114: rust-senior quote "named-args workaround the language now has better answers to" cites `02-pain-enumeration.md:79` | Read line 79 → blank line. The cited quote is at **line 77** | ✗ Off-by-two | **FAIL** |
| 17 | §3 line 197: "Comparison matrix in `03-scope-options.md` §..." line 351 | `grep` → "## Comparison matrix" header is at **line 350**, not 351 | ✗ Off-by-one | **FAIL** |

**Note:** The three off-by-N failures (#15, #16, #17) are all minor line-number drifts pointing at the correct *content* in the right document but at slightly the wrong line. None mislead the reader on substance — the surrounding paragraph context is intact. These are factual mismatches but not load-bearing.

**Strong claims that load-bearing decisions depend on (all verified):**
- 4× `manager.rs` line refs for credential rotation 🔴-1 (262, 370, 1378, 1400) — all PASS
- "write site does not exist" claim — PASS
- `manager.rs` 2101-line count — PASS
- DrainTimeoutPolicy::Abort path 1493-1510 — PASS
- `set_failed` dead-coded helper — PASS
- Engine zero-grep for Daemon/EventSource — PASS

---

## 3. Terminology coherence

### 3.1 `Credential` / `Auth` / `NoCredential` / `AuthScheme` — CLEAN
- §1.1 uses `Auth`, `AuthScheme`, `Credential`, `NoCredential` deliberately to distinguish *current shape* (Auth-based) from *target shape* (Credential-based). Each use is in-context.
- §2.3 line 166: trait shape is `type Credential: Credential` (target). §2.4 line 186: `R::Credential::Scheme::default()` warmup-footgun reference — uses target shape correctly.
- No leftover `Auth` references in places that should describe the **post-redesign** shape. Acceptable. ✓

### 3.2 Daemon / EventSource — TIGHTLY COUPLED
- Every reference (§1.2, §1.4, §2.3, §3.2, §3.3) names them together as a pair. No subsection splits them or treats them differentially. ✓

### 3.3 Blue-green / rotation / refresh / revoke
- §1.1, §2.3 line 169: "blue-green pool swap" (rotation pattern).
- §1.6, §2.3 line 170, §2.4 line 184: "rotation" and "refresh" used synonymously when describing the dispatcher hot path.
- §2.3 line 170: explicit distinction between `on_credential_refresh` and `on_credential_revoke` semantics. Acceptable usage — the doc explicitly flags these as adjacent-but-distinct. ✓

### 3.4 Option A/B/C qualifier consistency
- §3.1 "Option A — Minimal" (§3.1 header).
- §3.2 "Option B — Targeted" (§3.2 header).
- §3.3 "Option C — Comprehensive" (§3.3 header).
- All three qualifiers (Minimal / Targeted / Comprehensive) match `03-scope-options.md` and `03-scope-decision.md`. ✓

### 3.5 Acronyms / abbreviations
- "DoD" (Definition of Done) used in §2.4 line 184, §3.2 line 207. Defined contextually via expansion in `feedback_observability_as_completion.md` reference. Acceptable.
- "RPITIT" used in §2.4 line 179 — not expanded in-doc. **Minor terminology gap** — acceptable since RPITIT is standard Rust 2024 vocabulary in the project; not flagged.
- "MSRV" — used in §2.4 line 178 without expansion. Standard term in this codebase. Acceptable.

---

## 4. Open-item bookkeeping

### 4.1 Strategy §0 line 58 totals — INCONSISTENT WITH `03-scope-decision.md` SOURCE

**Strategy §0 line 58 claims:**
> "Phase 1 surfaced 28 findings in [`02-pain-enumeration.md`](...) §4; **12 are in-scope** per `03-scope-decision.md` §1, **5 are in `03-scope-decision.md` §2 out-of-scope** with explicit pointers, **11 are accepted-as-is**."

**Verified counts in `03-scope-decision.md`:**
- §1 (in-scope) row count: 12 findings (🔴-1 through 🔴-6, 🟠-7, 🟠-9, 🟠-12, 🟠-14, 🟠-15, 🟡-17). ✓ matches "12 in-scope".
- §2 (out-of-scope) row count: **13 rows / 15 findings** (🟠-8, 🟠-11, 🟠-13, 🟡-16, 🟡-18, 🟡-19, 🟡-20, 🟡-21, 🟡-22, 🟡-23, 🟡-24, 🟢-25/26/27 grouped, ✅-28). Strategy claims **5 in §2**. ✗
- §3 SF candidates: 1 finding (🟠-10 = SF-1). Strategy doesn't break this out separately.
- Total claimed: 12 + 5 + 11 = 28 ✓ (matches Phase 1 finding count)
- Total verified per `03-scope-decision.md`: 12 (§1) + 15 (§2) + 1 (§3 SF) = 28 ✓ (also matches)

**Mismatch:** the **5 / 11 split** for §2 doesn't reflect the actual content of `03-scope-decision.md §2`. The §2 table mixes three semantic buckets (deferred-with-pointer, absorbed-into-cascade, accepted-as-is) but Strategy §0 collapses them into "5 out-of-scope / 11 accepted-as-is" without explaining the bucket boundaries. A reader who clicks through to §2 will find 13 rows, not 5, and will be unable to reconcile.

**Severity:** 🟠 HIGH — open-item bookkeeping is a primary purpose of §0 line 58. Numerical mismatches here are exactly what the auditor exists to catch.

**Suggested fix (architect):** either (a) re-tabulate using the actual `03-scope-decision.md §2` taxonomy (13 rows split by treatment: ~6 deferred-with-pointer, ~3 absorbed, ~3 cosmetic, ✅-28 preserve, 🟢-25/26/27 no-action), or (b) note in §0 that the framing is *paraphrased* from the decision doc and reference §2 directly for the canonical breakdown.

### 4.2 Coverage of every Phase 1 🔴/🟠 finding

Cross-check every 🔴 (6) and 🟠 (9) Phase 1 finding to confirm it lands either in Strategy §1 / §3 in-scope or §3 out-of-scope-with-pointer:

| Phase 1 # | Treatment in Strategy CP1 | Verdict |
|---|---|---|
| 🔴-1 (credential seam) | §1.1 (full coverage) + §3.2 (Option B in-scope) | ✓ |
| 🔴-2 (Daemon orphan) | §1.2 (full coverage) + §3.2 (extraction) | ✓ |
| 🔴-3 (doc fabrication) | §1.3 (full coverage) | ✓ |
| 🔴-4 (drain-abort) | §1.5 (full coverage) + §3.2 (absorbed into Option B) | ✓ |
| 🔴-5 (Auth dead weight) | §1.1 (full coverage via §3.6 reshape) | ✓ |
| 🔴-6 (EventSource orphan) | §1.2 (paired with Daemon) + §3.2 (extraction) | ✓ |
| 🟠-7 (manager.rs 2101 L) | §1.4 (full coverage) + §3.2 (file-split in Option B) | ✓ |
| 🟠-8 (reserved AcquireOptions) | §2.2 line 158 mention + §3.3 deferral with pointer | ✓ |
| 🟠-9 (Daemon out-of-canon) | §1.2 + §3.2 | ✓ |
| 🟠-10 (deny.toml) | NOT in §1 or §3 narrative — referenced only via §0 line 58 implicit "11 accepted-as-is". Cited directly in `03-scope-decision.md §3` (SF-1). | ⚠ **GAP** in CP1 narrative — see finding 4.3 below |
| 🟠-11 (Runtime/Lease) | §3.3 (Option C deferred — "Runtime/Lease collapse...future cascade") | ✓ |
| 🟠-12 (register_pooled Auth=()) | §1.4 line 115 + §3.2 (absorbed via §3.6) | ✓ |
| 🟠-13 (Transport tests) | NOT mentioned in §1, §2, or §3 narrative. Only in `03-scope-decision.md §2`. | ⚠ **GAP** — see finding 4.4 below |
| 🟠-14 (rotation observability) | §1.6 (full coverage) + §2.4 line 184 (DoD) | ✓ |
| 🟠-15 (Credential/Auth doc contradiction) | §1.3 line 101 (3-way contradiction) + §3.2 implicit | ✓ |

### 4.3 🟠-10 SF-1 (deny.toml) — INTENTIONALLY OUT OF CP1 NARRATIVE
- Cited by `03-scope-decision.md §3` (standalone-fix candidate, ships independently).
- Strategy §0 line 58 generic framing absorbs SF candidates into "accepted-as-is" — defensible since SF candidates are *outside the cascade scope* by definition.
- Recommendation: 🟢 LOW — would benefit from a one-line mention in §2.3 or §3 explicitly noting "SF-1 ships independently per `03-scope-decision.md §3`" so spec-auditor / reader can confirm the deferral path. But not a blocker.

### 4.4 🟠-13 (Transport tests) — SILENT GAP IN CP1
- `02-pain-enumeration.md:174` flags 🟠-13 explicitly (zero Manager-level Transport tests).
- `03-scope-decision.md:56` resolves it as "Test debt, not structural defect. Follow-up task — issue filed post-cascade."
- Strategy CP1 mentions Transport in §1.4 line 113 (5 of 7 topologies have register_*) and §2.3 lines 172-174 (consumer set), but never names the Transport-test-debt deferral.
- **Severity:** 🟡 MEDIUM — finding decay risk (`feedback_incomplete_work.md` — explicit pointer to where deferred work goes is the discipline). Reader at CP3 freeze may forget 🟠-13 exists.
- **Suggested fix:** §0 line 58 should not silently absorb 🟠-13 into "11 accepted-as-is" without explicit pointer; or §2.3 should note Transport-test-debt as out-of-scope with the cited follow-up location.

### 4.5 §0 "Open items raised this checkpoint" (lines 228-233)
- 4 open items raised. Each has a clear pointer to the section that surfaced it (§2.3, §2.4, §2.1, §3.3). ✓
- §2.3 revocation-extension: clearly raised (line 170) and explicitly punted to CP2. ✓
- §2.4 warmup semantics: raised (line 186) and punted to CP2 §4 + Phase 6 Tech Spec §5. ✓
- §2.1 ADR amendment for TopologyTag scope boundary: raised (line 149) and punted to CP2. ✓
- §3.3 AcquireOptions resurrection trigger: raised (line 218) with explicit evidence-bar. ✓

All four open items are correctly bookkept. ✓

### 4.6 §3.2 "unanimous" framing
- Strategy §3 line 197 says "co-decision body (architect + tech-lead + security-lead) **unanimously picked** Option B in round 1".
- `03-scope-decision.md:25` says "Phase 2 co-decision body is **unanimously aligned** on Option B."
- `phase-2-tech-lead-review.md:12`: "Option B — Targeted, **with two bounded amendments**."
- `phase-2-security-lead-review.md:13-17`: "Option B — ENDORSE WITH AMENDMENTS (3 amendments)".
- **Verdict:** "unanimous" is consistent with `03-scope-decision.md`'s own framing. The Strategy §3.2 line 209 immediately disambiguates: "Tech-lead priority-called Option B with two bounded amendments... Security-lead ENDORSED B with three amendments". The "unanimous" framing refers to **option choice**, not amendment-content alignment. The disambiguation is correct and present. ✓
- See §6 below for the architect's own self-flag on this.

---

## 5. Forward/backward reference resolution

### 5.1 Forward references to CP2/CP3 sections (not yet drafted)
- §0 line 42-44: §4, §5, §6 mentioned as future content. Each marked "(planned)". ✓ acceptable placeholder framing.
- §0 line 56: reading order: "§4 (decisions) + §5 (open items) land in CP2; §6 (roadmap) lands in CP3." ✓
- §0 line 38: "tracked in §5 post-validation roadmap when CP2 lands". ✓ — but **note: §5 is described as "open items" earlier (line 43) and "post-validation roadmap" later (line 38, line 38 says §5; line 56 says §6 is roadmap)**. This is internally consistent if "open items" is the primary §5 framing and "post-validation roadmap" is §6.

**Actually re-reading line 38:** "tracked in §5 post-validation roadmap when CP2 lands". But line 43 + line 56 both say §5 = "open items" and §6 = "post-validation roadmap". **Mismatch within §0.**

- 🟡 MEDIUM finding: §0 line 38 says "§5 post-validation roadmap" but §0 lines 43, 56 establish §5 = open items, §6 = post-validation roadmap. Suggested fix: change line 38 to "§6 post-validation roadmap (CP3)" or similar.

### 5.2 Backward references
- §1.1 references "Phase 1 finding 🔴-5 (`02-pain-enumeration.md:166`)". Verified line 166 = 🔴-5 row. ✓
- §1.2 references "Phase 1 finding 🔴-2 / 🔴-6 (`02-pain-enumeration.md:161-162`)". Lines 161-162 don't perfectly contain the 🔴-2/🔴-6 rows (those are at 163, 167) but the **table-header context** (line 161 = severity-matrix header) is at the cited position. Acceptable approximation, but technically off. 🟢 LOW.
- §1.6 references "(`02-pain-enumeration.md:175`)". Verified line 175 = 🟠-14 row. ✓

### 5.3 Phase 4 spike framing
- §0 line 36: "Phase 4 spike produces those". ✓ clear.
- §2.3 line 171: "Out-of-scope for this cascade per `03-scope-decision.md §2`" (engine #391). ✓
- §0 line 38: "(...) tracked in §5 post-validation roadmap" — see 5.1 above for the §5/§6 confusion.

### 5.4 Dead cross-references
- Searched for "see §X.Y" patterns. None observed pointing into §1.7+ (§1 only has 1.1-1.6) or §3.4+ (§3 only has 3.1-3.3) or §2.6+ (§2 only has 2.1-2.5). ✓
- All in-doc forward refs resolve.

---

## 6. Author's self-flagged ambiguities — my verdict

### 6.1 §2.3 revocation-extension framing (problem-only or decision-requested?)
- §2.3 line 170 reads: "Strategy must extend §3.6 with revoke semantics. Candidate approaches (CP2 to pick): (a) `on_credential_refresh` carries both semantics... (b) separate `on_credential_revoke` method."
- §0 line 230 (open items): "spec-auditor to verify CP2 picks (a)... or (b)... CP2 must also confirm whether this extension warrants a credential-side Tech Spec amendment."
- **Verdict:** unambiguous. CP1 explicitly defers the choice to CP2 with named candidates. No decision is requested *of CP1*. Reader (and CP2 author) knows exactly what work is pending.
- **My verdict on architect's self-flag:** architect was overcautious. The framing is clean. ✅ NO ACTION.

### 6.2 §2.1 ADR-0035 disclaimer positioning
- §2.1 line 149 leads with "**ADR-0035 — Phantom-shim capability pattern**. *For reference, not binding on this redesign.*"
- The disclaimer is positioned **immediately after the ADR title** and before any technical content. The structure is: `ADR title → "Not binding" italic disclaimer → why-cited explanation → boundary statement`.
- **Verdict:** the disclaimer placement is fine. A reader who skims sees "not binding on this redesign" before any of the technical content. It's not buried.
- However: §2.1 is titled "ADR references" and contains exactly 2 bullets (ADR-0035 reference + future ADR candidate). If the auditor's concern is "is the reader going to assume ADR-0035 *binds* this work?" — no, the disclaimer text is sufficient, and §0 line 232 even raises a follow-up open item ("worth a one-line ADR amendment to ADR-0035 noting the scope boundary") that captures the residual risk.
- **My verdict on architect's self-flag:** architect was overcautious. The positioning is acceptable. ✅ NO ACTION.

### 6.3 §3.2 "unanimous" vs amendment-count framing
- §3 line 197: "co-decision body... unanimously picked Option B in round 1".
- §3.2 line 209: explicit reconciliation — "Tech-lead priority-called Option B with two bounded amendments... Security-lead ENDORSED B with three amendments".
- See §4.6 above for full bookkeeping.
- **Verdict:** the "unanimous" wording is technically accurate (round-1 lock with no other option considered), and the immediate next sentence enumerates amendment counts. A reader cannot conclude "no amendments existed" from §3.2 — the amendments are stated in the same paragraph.
- **One sub-concern:** §3 line 197 ("unanimously picked Option B in round 1 of the max-3 protocol") frames "unanimous" at the *option-choice* level. This is consistent with how `03-scope-decision.md:25` uses the word. If the architect is worried that "unanimous" overstates **endorsement-level alignment**, it does not — the immediately-following sentence in §3.2 makes amendment counts explicit.
- **My verdict on architect's self-flag:** architect was overcautious. The framing is precise and the next-sentence disambiguation eliminates the risk. ✅ NO ACTION.

**Summary:** all three self-flagged items are non-issues per evidence. Architect can stop second-guessing these.

---

## 7. Audit verdict

**PASS_WITH_MINOR.**

Rationale:
- 0 BLOCKERs.
- 14 of 17 spot-checked claims verified at exact file:line. The 3 line-number drifts are minor (off-by-1 / off-by-2 / off-by-6) and point at the correct content via near-context — they do not mislead a load-bearing decision.
- Cross-section consistency is clean. No internal contradictions on any load-bearing claim. Type names, severity tags, option qualifiers all carry consistently across §0-§3.
- Forward references to CP2/CP3 are gracefully framed as placeholders.
- Author's three self-flagged ambiguities are non-issues — architect can stop self-flagging and proceed.
- One numerical bookkeeping mismatch (§0 line 58 vs `03-scope-decision.md §2`) is the only finding with audit-substance; it does not block CP2 dispatch but should be reconciled before CP3 freeze.
- Two minor coverage gaps (🟠-10 SF-1 and 🟠-13 Transport-tests) are silently absorbed into "accepted-as-is" without explicit pointers; recommend explicit pointers per `feedback_incomplete_work.md`.

**Recommended action:** proceed to CP2 dispatch. Address minor amendments below at any time before CP3 signoff (architect can batch with CP2 work).

---

## 8. Required amendments

**(none — verdict is PASS_WITH_MINOR; no amendments are blockers for CP2 progression)**

---

## 9. Nice-to-have amendments

Listed in priority order. Architect may defer all of these to CP2 work or batch into a "docs(strategy)" cleanup PR per §0 freeze policy.

### 9.1 §0 line 58 — re-tabulate the 28-finding split
**Why:** the "12 in-scope / 5 out-of-scope / 11 accepted-as-is" framing doesn't match `03-scope-decision.md §2`'s actual 13-row table. A reader who clicks through cannot reconcile. Either (a) re-tabulate using the actual taxonomy from `03-scope-decision.md §2` or (b) replace the inline counts with "see `03-scope-decision.md §2` for canonical breakdown." Severity: 🟠 HIGH — bookkeeping is §0's primary purpose.

### 9.2 §0 line 38 — fix §5/§6 swap
**Why:** line 38 says "§5 post-validation roadmap" but lines 43, 56 establish §5 = open items, §6 = post-validation roadmap. Internal §-name drift. Severity: 🟡 MEDIUM.
**Fix:** change "§5 post-validation roadmap when CP2 lands" → "§6 post-validation roadmap when CP3 lands" or "Strategy §5/§6 post-CP2".

### 9.3 §1.4 line 112 — fix line ref to `02-pain-enumeration.md:82`
**Why:** quoted text "every field is referenced by multiple public methods" is at line 76, not 82. Severity: 🟢 LOW.
**Fix:** change citation to `02-pain-enumeration.md:76`.

### 9.4 §1.4 line 114 — fix line ref to `02-pain-enumeration.md:79`
**Why:** quoted text "named-args workaround the language now has better answers to" is at line 77, not 79. Severity: 🟢 LOW.
**Fix:** change citation to `02-pain-enumeration.md:77`.

### 9.5 §3 line 197 — fix line ref to `03-scope-options.md` line 351
**Why:** "Comparison matrix" header is at line 350, not 351. Severity: 🟢 LOW.
**Fix:** change citation to `03-scope-options.md:350`.

### 9.6 §2.3 or §3 — explicit pointer for SF-1 + 🟠-13
**Why:** Strategy §0 line 58 absorbs 🟠-10 (SF-1) and 🟠-13 (Transport tests) into the generic "11 accepted-as-is" bucket without an explicit pointer to where the deferred work tracks. Per `feedback_incomplete_work.md`, deferred work needs explicit homes. Severity: 🟡 MEDIUM.
**Fix:** add a one-line cross-reference in §2.3 (cross-crate contracts) or §3.3 (Option C rejected) explicitly pointing at `03-scope-decision.md §3` (SF-1) and `03-scope-decision.md §2` row 56 (🟠-13 follow-up).

### 9.7 §1.2 line 86 — broaden cited line range
**Why:** `02-pain-enumeration.md:161-162` cites the severity-matrix header rows, not the 🔴-2 / 🔴-6 rows themselves (which are at 163, 167). Severity: 🟢 LOW.
**Fix:** change citation to `02-pain-enumeration.md:163, 167`.

---

## 10. Coverage summary

- **Structural:** PASS — 0 findings (all forward/backward refs resolve, all section numbering consistent, TOC implicit but coherent).
- **Consistency:** PASS — 0 findings (type names, severity tags, option qualifiers consistent throughout).
- **External verification:** 14/17 PASS, 3/17 FAIL (off-by-N drift, minor; no load-bearing claim refuted).
- **Bookkeeping:** PASS_WITH_MINOR — 1 numerical mismatch (§0 line 58 vs `03-scope-decision.md §2`), 2 silent-absorption gaps (🟠-10, 🟠-13).
- **Terminology:** PASS — 0 findings on load-bearing terms; minor RPITIT/MSRV non-expansions acceptable per project convention.
- **Definition-of-done (`docs/PRODUCT_CANON.md` §17):** N/A at Strategy level — applies to Tech Spec / implementation, not Strategy §0-§3 framing.

---

## 11. Recommended handoff

- **architect:** address §9.1 (re-tabulate 28-finding split) before CP3 freeze; rest of §9 amendments are at architect's discretion. None block CP2 progression.
- **tech-lead:** no action required from this audit.
- **security-lead:** no action required from this audit.
- **orchestrator:** verdict is PASS_WITH_MINOR — proceed with CP2 dispatch. Architect may batch §9 amendments with CP2 work.

---

*End of audit.*
