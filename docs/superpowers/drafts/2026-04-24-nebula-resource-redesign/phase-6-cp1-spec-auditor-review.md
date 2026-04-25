# Tech Spec CP1 — Spec-Auditor Review
**Date:** 2026-04-25
**Reviewer:** spec-auditor
**Subject:** `docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md` (CP1 §0–§3)
**Read passes:** structural | consistency | external | bookkeeping | terminology

---

## 1. Cross-section consistency

### 1.1 §3.6 contradicts §3.1 / §3.2 on the `on_credential_refreshed` signature change — 🟠 HIGH
- **§3.6 line 946** claims the signature change as: "(currently `Result<Vec<(ResourceKey, ReloadOutcome)>, Error>`)" — i.e. it is comparing the new dispatcher to the old `manager.rs:1363` `Result<Vec<(ResourceKey, ReloadOutcome)>, Error>`.
- **§3.2 line 775** new return type is `Vec<(ResourceKey, RefreshOutcome)>` — *not* wrapped in `Result`. Old returned `Result<…, Error>`. New does not. §3.6 narrates the type swap (`ReloadOutcome` → `RefreshOutcome`) but is silent on the dropped `Result` wrapper.
- **Impact:** A migration-impact reader of §3.6 will think the only call-site change is `ReloadOutcome` → `RefreshOutcome`. They will miss the `?` propagation collapse. This is a load-bearing migration cue per §3.6's own framing ("CP3 §13 enumerates the per-consumer migration impact").
- **Fix-class:** §3.6 should explicitly note the `Result` removal, or §3.2 should re-introduce `Result<…, Error>` to match §3.6's narration. Architect's call.

### 1.2 §0.4 reading order omits §0 self-reference — 🟢 LOW
- §0.4 line 62 reads "§0 → §1 → …" — but §0.4 IS itself in §0. The chain reads correctly only if "§0" is interpreted as "the rest of §0 (i.e. §0.1-§0.4)". Minor; not misleading.

### 1.3 §3.5 `RefreshOutcome` vs §3.5 `RotationOutcome` mix — 🟡 MEDIUM
- §3.5 lines 908, 919 define `RefreshOutcome` and `RevokeOutcome` enums.
- §3.5 line 935 then says: "every dispatched outcome emits `ResourceEvent::CredentialRefreshed { credential_id, resources_affected, outcome }` (where `outcome` is `RotationOutcome` summarizing the aggregate)."
- `RotationOutcome` is **never defined in CP1**. Spike defines `RotationOutcome` (`spike/.../manager.rs:63`) but the Tech Spec body introduces only `RefreshOutcome`/`RevokeOutcome`. Reader has no idea what `RotationOutcome` is — Strategy §4.9 line 333 also uses `outcome: RotationOutcome` without defining it.
- Likely intent: `RotationOutcome` is the aggregate type, distinct from per-resource. Architect should either (a) add a `RotationOutcome` enum definition in §3.5, (b) defer to CP2 with explicit "CP2 §7 defines `RotationOutcome`" forward-reference, or (c) drop the unbound term.

### 1.4 §3.2 line 671 declares trait `pub(crate)` — visibility/import contradiction — 🟢 LOW
- §3.2 line 690 declares `pub(crate) trait ResourceDispatcher`.
- §3.1 line 644 imports `Arc<dyn ResourceDispatcher>` and stores it as the `credential_resources` `DashMap` value type at line 662.
- `pub(crate)` is correct (same crate, both files are `crate::manager::*`), but the `DashMap` type now uses an internal type. This is internal-only by design; flag as intent confirmation only.

---

## 2. Claim-vs-source spot-checks (12 attempted, 11 passed)

| # | Claim | Source | Result |
|---|-------|--------|--------|
| 1 | §1.1 cites credential Tech Spec §3.6 lines 935-955 for trait shape verbatim | credential-tech-spec.md:935-955 | **PASS** — `pub trait Resource { type Credential: Credential; …}` block exactly there |
| 2 | §2.1 cites credential Tech Spec §3.6 lines 961-993 for blue-green pool swap pattern | credential-tech-spec.md:961-993 | **PASS** — `PostgresPool` blue-green example exactly there |
| 3 | §2.5 Q5 references "spike's three compile-fail probes" | `spike/.../compile_fail.rs` | **PASS** — exactly 3 `compile_fail` probes (lines 11-95, 96-115, 117-150) |
| 4 | §3.2 cites spike `manager.rs:200-205` (`with_timeout`) | `spike/.../manager.rs:200-205` | **PASS** — `pub fn with_timeout(per_resource_timeout: Duration)` block at exactly those lines |
| 5 | §2.5 Q2 cites spike `manager.rs:227` (`TypeId::of::<R::Credential>()`) | `spike/.../manager.rs:227` | **PASS** — `let opted_out = TypeId::of::<R::Credential>() == TypeId::of::<NoCredential>();` exactly there |
| 6 | §3.5 cites spike `lib.rs:537-578` (`parallel_dispatch_isolates_per_resource_errors`) | `spike/.../resource-shape-test/src/lib.rs:537-578` | **PASS** — test body exactly at 537-578 |
| 7 | §3.5 cites spike `lib.rs:483-531` (`parallel_dispatch_isolates_per_resource_latency`) | `spike/.../lib.rs:483-531` | **PASS** — test body exactly at 483-531 |
| 8 | §2.4 cites spike `lib.rs:583-607` (`parallel_dispatch_crosses_topology_variants`) | `spike/.../lib.rs:583-607` | **PASS** — test body exactly at 583-607 |
| 9 | §3.6 / 🔴-1 resolution cites `manager.rs:1378` `todo!()` panic | `crates/resource/src/manager.rs:1378` | **PASS** — `todo!("Implementation deferred to runtime integration")` exactly there |
| 10 | §3.6 cites `manager.rs:262` reverse-index field declaration | `crates/resource/src/manager.rs:262` | **PASS** — `credential_resources: dashmap::DashMap<CredentialId, Vec<ResourceKey>>` exactly there |
| 11 | §0.1 / §0.2 claim CP1 ratification flips ADR-0036 + ADR-0037 to `accepted` | ADR-0036:198, ADR-0037:140 | **PARTIAL** — see finding 2.1 below |
| 12 | §2.5 Q5 references "spike NOTES.md flagged two production-relevant gaps" (double-registration + NoCredential+real-id) | `spike/NOTES.md:198-202` | **PASS** — both gaps listed there |

### 2.1 ADR-0037 acceptance gate scope — 🟠 HIGH
- **Tech Spec §0.1 line 32** says: "CP1 ratification by tech-lead flips both ADR-0036 and ADR-0037 from `proposed` to `accepted` per their respective acceptance gates."
- **ADR-0037 line 140** specifies its own gate: "this ADR moves to `accepted` when Phase 6 Tech Spec CP1 ratifies the **engine-side landing site** (module layout, primitive name, EventSource→TriggerAction adapter signature) against the target layer recorded above."
- **Tech Spec §2.4 line 519** explicitly defers the engine-side landing site: "Engine-side landing site (module layout, primitive naming, EventSource→TriggerAction adapter signature) is **CP3 §13 deliverable**, not CP1 — CP1 records the enum shrink as a contract, not the engine-side shape."
- **Contradiction:** ADR-0037's gate cannot fire on CP1 (CP1 explicitly defers what ADR-0037 requires). Either (a) ADR-0037's gate language needs amending to say "CP1 records the engine-fold contract; CP3 §13 fills in the landing site", or (b) Tech Spec §0.1 should *not* claim ADR-0037 flips at CP1 — only ADR-0036 should. Same false-claim flagged in §0.2 line 44.
- **Impact:** tech-lead reads §0.1, ratifies CP1, marks ADR-0037 `accepted`. Then someone reads ADR-0037 expecting CP1 to have specified the engine-side landing site, finds CP1 §2.4 defers it. Decision-state contradiction across the doc set.
- **Severity rationale:** load-bearing because ADR status drives downstream gating. Architect to either (i) loosen ADR-0037's gate language, or (ii) qualify §0.1 ("CP1 ratification flips ADR-0036; ADR-0037 ratifies on CP3 §13 landing site"). tech-lead decision needed.

---

## 3. Terminology coherence

### 3.1 `NoCredential` vs `Auth` — coherent. ✅ GOOD
- `Auth` appears 9 times across §0–§3, always as historical reference ("replaces", "was", "ADR-0036 retires"). No drift to current shape.
- `NoCredential` consistently used as the opt-out mechanism. §2.2 line 331 + §2.5 Q1 align on `nebula-credential` location.

### 3.2 `dispatcher` vs `Manager` — coherent. ✅ GOOD
- "Manager" is the type name; "dispatcher" is the per-resource trampoline trait (`ResourceDispatcher`). Used consistently. No leakage between roles.

### 3.3 `RefreshOutcome` vs `ReloadOutcome` — coherent within CP1, mixed against current code. ✅ GOOD (in-doc) / 🟢 LOW (external)
- CP1 introduces `RefreshOutcome`. Current `manager.rs:1363` returns `ReloadOutcome`. §3.6 narrates the rename. Within CP1 there is no drift; against current trunk there is — but CP1 explicitly flags the rename, which is the right pattern.

### 3.4 `RotationOutcome` introduced without definition — see finding 1.3.

### 3.5 "blue-green pool swap" / "blue-green swap" — 🟢 LOW
- Both spellings appear: §2.1 line 204 "blue-green pool swap"; §2.3 line 432 "pool-swap impls". Synonymous. Strategy §4.1 line 244 also uses both forms. Per drift pattern memory entry #7, count = 2 distinct phrasings — flagging as below threshold but worth a consistent passive in CP2.

### 3.6 "engine-fold" / "Daemon and EventSource extraction" — coherent within CP1. ✅ GOOD
- §1.2 line 77 "Daemon and EventSource extraction"; §2.4 explicitly cites ADR-0037 for the fold. ADR-0037 title uses "engine-fold". No mid-doc drift.

### 3.7 `Manager::on_credential_refreshed` (past tense) vs `on_credential_refresh` (imperative) — naming intent. ✅ GOOD
- Manager method ends in `-ed` (event-handler style); Resource trait method ends in imperative form. Consistent throughout: §3.2 uses `on_credential_refreshed`/`_revoked` for Manager; §2.1, §2.3 use `on_credential_refresh`/`_revoke` for trait. This is a deliberate convention; not drift.

---

## 4. Forward-reference integrity

### 4.1 All forward refs to CP2/CP3/CP4 are bounded with explicit "CP2 will / CP3 §X enumerates" framing — ✅ GOOD
Tabulated 18 forward refs:
- CP2 §5 (4 refs) — all bounded
- CP2 §6 (5 refs — security gates) — all bounded
- CP2 §7 (3 refs — observability cardinality) — all bounded
- CP2 §8 (3 refs — test plan) — all bounded
- CP3 §13 (3 refs — engine-side landing + per-consumer migration) — all bounded
- CP4 §15 (1 ref — future-cleanup-when-stable) — bounded

No forward ref leaks content; every forward ref names the deliverable + the CP that owns it.

### 4.2 §1.4 success criterion 6 references §3.6 — resolves correctly. ✅ GOOD
"Phase 1 🔴-1 … explicitly resolved with file:line cross-references (§3.6)." §3.6 indeed contains those file:line refs.

### 4.3 §2.4 line 519 forward-refs CP3 §13 for engine-side landing — but ADR-0037 gates this on CP1. See finding 2.1.

---

## 5. Open-item resolution (5 spike Qs)

Spike NOTES.md §"Open questions for Tech Spec CP1" enumerates exactly 5 questions (lines 156-202).

| Spike Q | CP1 §2.5 resolution | Resolved? |
|---------|---------------------|-----------|
| Q1 NoCredential location | §2.5 Q1 → `nebula-credential` crate | **YES** — clear pick + 3 reasons |
| Q2 TypeId vs sealed-trait marker | §2.5 Q2 → TypeId | **YES** — clear pick + 3 reasons |
| Q3 Box::pin per-dispatch overhead | §2.5 Q3 → acknowledge in observability | **YES** — clear posture |
| Q4 Per-resource timeout config surface | §2.5 Q4 → per-Manager default + per-Resource override | **YES** — clear pick + 3 reasons |
| Q5 Compile-fail probe production gaps | §2.5 Q5 → 4 trait probes; runtime gaps go to §3.1 invariants | **YES** — but see finding 5.1 |

### 5.1 §2.5 Q5 commits "4 trait probes" but the 4th is described, not implemented — 🟡 MEDIUM
- §2.5 Q5 lines 575-579 commits to "four trait probes" — three carry forward from spike, the 4th (`on_credential_revoke` wrong-signature) is **NEW** for production.
- The new 4th probe is not present in `spike/.../compile_fail.rs` (verified: only 3 probes, no symmetric revoke probe).
- This is fine — the 4th probe is a CP1 commitment for production, not a spike artifact. But §2.5 line 575 phrases it as "CP1 commits four trait probes:" which reads as a present-tense claim. Architect could clarify: "CP1 commits to four trait probes — three carry forward from spike; the fourth is new and lands in CP2 §8 alongside the test plan."
- Severity: 🟡 — does not mislead a careful reader (the probe is described as **NEW**), but the framing is mid-sentence ambiguous.

### 5.2 §2.5 Q5 also closes 2 production-relevant runtime gaps from spike — fully resolved. ✅ GOOD
Both runtime gaps (double-registration; NoCredential+real_id) resolved with explicit homes (CP2 §5 / §3.1 + CP2 §8). Per drift pattern memory entry #3 (silent absorption), this is the *opposite* — explicit pointer to where each gap lands.

### 5.3 §2.5 Q4 timeout default "30 seconds" — value justification. 🟢 LOW
- §2.5 Q4 line 557 commits "defaults to 30 seconds — value chosen to accommodate slow blue-green pool builds while still bounding misbehaving impls."
- Spike `manager.rs:193` defaults to **1 second** (different concern: tests need short timeouts). No conflict (production vs test), but the 30s figure has no Strategy / spike grounding. tech-lead should ratify the number explicitly.

---

## 6. Word-cap compliance

Word counts measured by `awk` slice between subsection headers:

| Subsection | Cap | Actual | Compliance |
|------------|-----|--------|------------|
| §2.1 (Resource trait full Rust signature) | ≤1000 | **1103** | **OVER by 103 (10%)** — see finding 6.1 |
| §2.5 (Open question resolutions) | ≤800 | **747** | OK |
| §1.1 Primary goals | ≤600 | 167 | OK |
| §1.4 Success criteria | ≤600 | 170 | OK |
| §3.2 Rotation dispatcher | ≤600 | 586 | OK (just under) |
| §3.6 Resolution of 🔴-1 / 🔴-4 | ≤600 | 249 | OK |

### 6.1 §2.1 over word cap — 🟡 MEDIUM
- §2.1 caps at 1000 words; actual is 1103 (counted between line 109 `### §2.1 …` and line 329 `### §2.2 …`, which includes §2.1.1).
- The Rust code block dominates word count (~700 words of the 1103 are in the trait + impl blocks). If the cap intent is "prose ≤1000 excluding code", §2.1 is fine. If the cap is "all content ≤1000", §2.1 is over.
- Architect to clarify cap intent. If cap excludes code blocks, §2.1 prose is well under (~400 words). If cap includes code, §2.1 needs trimming or the cap needs adjustment.

---

## 7. Verdict

**PASS_WITH_MINOR.**

CP1 is internally coherent on the high-stakes claims:
- All 12 spot-check claims resolve (1 PARTIAL on ADR-0037 acceptance gate; 11 fully verified).
- All 5 spike open questions resolved with rationale.
- Forward-reference integrity is clean (18 forward refs, all bounded with explicit CP ownership).
- Terminology consistent within CP1; the Auth/Credential rename narration honors the rename rather than drifting.
- Phase 1 finding resolutions (🔴-1 + 🔴-4) trace cleanly to §3.6 with verifiable file:line cross-references.

Two findings (2.1 and 1.1) are 🟠 HIGH because they create cross-document or cross-section decision-state ambiguity. Neither is a BLOCKER — both are addressable with localized text changes by architect, no design re-decision required.

CP1 is ratifiable after architect addresses the 🟠 HIGH findings; the 🟡 MEDIUM findings can land in CP2.

---

## 8. Required amendments

### 8.1 🟠 HIGH — must address before CP1 freeze
- **Finding 2.1:** Reconcile §0.1 / §0.2 ADR-0037 acceptance claim with ADR-0037's own gate language. Either soften §0.1 ("CP1 ratifies ADR-0036; ADR-0037 ratifies on CP3 §13") OR amend ADR-0037's gate text to acknowledge the staged ratification pattern. **Tech-lead decision required** — this is a process question, not just a doc fix.
- **Finding 1.1:** §3.6 line 946 narrates the type rename `ReloadOutcome` → `RefreshOutcome` but is silent on the `Result<…, Error>` wrapper being dropped from the new signature. Either restore the `Result` wrapper in §3.2 to match §3.6's narration, OR have §3.6 explicitly call out the `Result` removal as a second migration cue. Architect call.

### 8.2 🟡 MEDIUM — fix before CP2 starts
- **Finding 1.3:** §3.5 uses the term `RotationOutcome` without definition. Either define it in §3.5 (alongside `RefreshOutcome`/`RevokeOutcome`), or add forward-ref "(CP2 §7 defines `RotationOutcome`)", or drop the term.
- **Finding 5.1:** §2.5 Q5 phrases "CP1 commits four trait probes" — clarify that the 4th is NEW (not carrying from spike), with explicit landing-site (CP2 §8 test plan).
- **Finding 6.1:** §2.1 word count 1103 vs cap 1000. Either confirm cap excludes code blocks (then §2.1 is well under), or trim §2.1 prose by ~100 words. Architect to clarify cap intent.

### 8.3 🟢 LOW — defer or batch
- **Finding 1.2:** §0.4 reading order self-reference clarification.
- **Finding 1.4:** §3.2 internal `pub(crate)` visibility — confirm intent only.
- **Finding 3.5:** "blue-green pool swap" vs "blue-green swap" / "pool-swap impls" — pass for consistent phrasing in CP2.
- **Finding 5.3:** §2.5 Q4 30-second default — tech-lead ratify the number.

---

### Coverage summary
- Structural: pass / 0 findings
- Consistency: pass / 4 findings (1 HIGH, 1 MEDIUM, 2 LOW)
- External verification: pass / 1 finding (1 HIGH; 11/12 spot-checks fully passed)
- Bookkeeping (open-items): pass / 1 finding (1 MEDIUM, 1 LOW)
- Terminology: pass / 2 findings (both LOW; coherent overall)
- Word-cap compliance: 5/6 within cap; §2.1 needs clarification (1 MEDIUM)

### Definition-of-done coverage (per `docs/PRODUCT_CANON.md` §17)
- CP1 is a checkpoint, not the full Tech Spec. DoD applies to the full §17 list at CP4 close, not CP1. Within CP1's stated scope (§0–§3), all six §1.4 success criteria are addressed:
  - All five spike open questions resolved (§2.5) — **YES**
  - Full Rust signature for `Resource` trait (§2.1) — **YES**
  - `NoCredential` location decided (§2.2) — **YES**
  - Five topology sub-traits elaborated (§2.4) — **YES** (signatures present)
  - Manager runtime model with reverse-index write path + parallel dispatcher + per-resource timeout (§3.1–§3.5) — **YES**
  - Phase 1 🔴-1 + 🔴-4 explicitly resolved (§3.6) — **YES**

### Recommended handoff
- **architect:** address findings 2.1 (with tech-lead), 1.1, 1.3, 5.1, 6.1. Batch §8.3 LOW findings into CP2 cleanup.
- **tech-lead:** decide finding 2.1 — is ADR-0037 acceptable to ratify at CP1 (gate-text amendment) or does it stage to CP3 (then §0.1 needs amendment)? Also ratify finding 5.3 (30-second default).
- **rust-senior:** no blocking spec-auditor findings on the trait-shape side; proceed with rust-senior review on §2.1 / §2.4 / §3.2 trait-shape ratification. Spec-auditor will not re-audit unless architect amendment introduces new content.
