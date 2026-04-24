# Strategy CP2 — Spec-Auditor Review

**Date:** 2026-04-24
**Reviewer:** spec-auditor (subagent dispatch)
**Document audited:** `docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md`
**Checkpoint:** CP2 (§0-§5; CP1 ratification edits + §4 decision record + §5 open items)
**Commit basis:** working tree at `d6cee19f` (CP1 wip snapshot)
**Read passes:** structural | consistency | external | bookkeeping | terminology

---

## 1. CP1 ratification edits — landing check

| Edit | Source | Strategy location | Verdict |
|---|---|---|---|
| §0 retabulation N1 | CP1 audit §9.1 | §0 lines 60-67 — verified split: 12 + 6 + 3 + 5 + 1 + 1 = 28 | ✓ LANDED. Math checks out. Categories match `03-scope-decision.md §1/§2/§3`. |
| §0 line 38 §5/§6 swap N3 | CP1 audit §9.2 | §0 line 38 — now reads "§6 post-validation roadmap when CP3 lands; CP2 §5 records open items" | ✓ LANDED |
| §0 explicit pointers for SF-1 + 🟠-13 N2 | CP1 audit §9.6 | §0 line 61 (🟠-13 → "follow-up task post-cascade, issue filed by orchestrator"); §0 line 64 (🟠-10 SF-1 → "devops standalone PR, `03-scope-decision.md §3`") | ✓ LANDED |
| §1.4 line ref `:82` → `:76` (N3) | CP1 audit §9.3 | §1.4 line 121 cites `02-pain-enumeration.md:76` for "every field is referenced..." | ✓ LANDED |
| §1.4 line ref `:79` → `:77` (N4) | CP1 audit §9.4 | §1.4 line 123 cites `02-pain-enumeration.md:77` for "named-args workaround..." | ✓ LANDED |
| §3 line ref `351` → `350` (N5) | CP1 audit §9.5 | §3 line 208 cites `03-scope-options.md` "(line 350)" | ✓ LANDED |
| §2.3 E2 wording | tech-lead E2 | §2.3 line 179 — "**CP2 §4 must extend §3.6 with revoke semantics**" (bolded) | ✓ LANDED |
| §2.4 E1 spike-exit-criteria | tech-lead E1 | §2.4 line 196 — "Spike exit criteria (Phase 4): do NOT include sub-trait fallback... If §3.6 ergonomics or perf fail the spike, escalate to Phase 2 round 2" | ✓ LANDED |
| §2.4 E4 `feedback_active_dev_mode.md` cross-ref | tech-lead E4 | §2.4 line 195 — bullet referencing `feedback_active_dev_mode.md` re: bundled-PR-wave + remove-Auth | ✓ LANDED |
| §3.2 E3 amendment-count parenthetical | tech-lead E3 / CP1 audit §6.3 | §3 line 208 — "(with 2 tech-lead amendments and 3 security-lead amendments tightening the in-scope envelope; all endorsed in the single round...)" | ✓ LANDED |

**Verdict:** all 10 CP1-dispatch edits landed correctly. CP1 audit's previously-flagged 🟠-10 + 🟠-13 silent-absorption gaps are CLOSED at §0 lines 61, 64.

**Deferred items (per CP2 CHANGELOG line 388):**
- spec-auditor §9.7 (`02-pain-enumeration.md:161-162` line range) — explicitly punted to CP3 cleanup. Acceptable.

---

## 2. §4 cross-section consistency

### 2.1 §4.x → §1.x backward refs

| §4.X | Cites | Verified |
|---|---|---|
| §4.1 (trait reshape) | §1.1 (problem), §2.3 (constraint), §3.2 (Option B) | ✓ all three resolve |
| §4.2 (revoke semantics) | §2.3 (constraint surface), §4.9 (observability symmetry), §5.5 (coordination) | ✓ all resolve |
| §4.3 (dispatch mechanics) | §1.1, §2.3, §4.9 | ✓ all resolve |
| §4.4 (engine-fold) | §1.2 (problem), §2.2 (canon §3.5), §3.2 (Option B) | ✓ all resolve |
| §4.5 (file-split) | §1.4 (problem), §2.2 (canon §12.7) | ✓ resolve |
| §4.6 (drain-abort) | §1.5 (problem), §2.2 (canon §12.6 honesty) | ✓ resolve |
| §4.7 (doc rewrite) | §1.3 (problem), §2.4 (`feedback_incomplete_work.md` cited indirectly) | ✓ resolve |
| §4.8 (migration wave) | §2.5 (release posture), §3.1 (Option A blocked), §4.1, §4.7 | ✓ all resolve |
| §4.9 (observability) | §1.6 (problem), §2.4, §4.2, §4.3 | ✓ all resolve |

**Result:** all §4 backward refs resolve. No drift.

### 2.2 §4 forward refs to §5 open items

- §4.4 line 272 → "Open item §5.1 records the conditions under which this revisits" → §5.1 captures the trigger ✓
- §4.2 line 252 → "Open item §5.5 captures this decision-fork" → §5.5 captures the credential-side coordination ✓

Both forward refs resolve.

### 2.3 LOCKED scope per `03-scope-decision.md §4` — alignment

| `03-scope-decision.md §4.X` | Strategy §4.X | Verdict |
|---|---|---|
| §4.1 (Credential reshape verbatim §3.6, NO sub-trait) | §4.1 (per §3.6, NO `AuthenticatedResource` sub-trait, cites `03-scope-decision.md:88-92`) | ✓ aligned |
| §4.2 (parallel dispatch, isolation invariant) | §4.3 (parallel `join_all`, `FuturesUnordered` cap deferred, per-resource isolation, cites `03-scope-decision.md §4.2 lines 94-100`) | ✓ aligned |
| §4.3 (revoke extension — Strategy proposes) | §4.2 (extends §3.6 with separate `on_credential_revoke`) | ✓ aligned |
| §4.4 (observability DoD) | §4.9 (trace + counter + event = DoD) | ✓ aligned |
| §4.5 (warmup must not Scheme::default()) | §4.9 line 330 + §2.4 line 197 | ✓ aligned |
| §4.6 (Daemon/EventSource extraction target — Strategy decides) | §4.4 (engine-fold = option (a)) | ✓ aligned |
| §4.7 (no shims, 5-consumer atomic, frontier) | §4.8 (atomic 5-consumer PR) + §2.5 | ✓ aligned |

**Verdict:** §4 honors LOCKED scope exactly. No divergence.

---

## 3. Claim-vs-source verification (spot-check)

11 file:line citations spot-checked.

| # | Strategy claim | Citation | Verdict |
|---|---|---|---|
| 1 | §4.1: "credential Tech Spec §3.6 (lines 935-956)" | Read 935 → `pub trait Resource {`; 956 → `}` closing trait | ✓ exact match |
| 2 | §4.3: "`03-scope-decision.md §4.2 lines 94-100`" | Read 94-100 → §4.2 header through "Strategy §5 records this." | ✓ exact match |
| 3 | §4.9: cites `phase-2-tech-lead-review.md:90-94` for amendment 2 | Read 90-94 → amendment 2 block | ✓ exact match |
| 4 | §4.9: cites `phase-2-security-lead-review.md:76-82` for B-3 | Read 76-82 → B-3 block | ✓ exact match (B-3 ends at line 81 substantively; 82 is paragraph break) |
| 5 | §4.4: cites `INTEGRATION_MODEL.md:99` for TriggerAction substrate | Read 99 → mid-paragraph mentioning "`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`" | ✓ contains the cited reference |
| 6 | §4.4: "no `nebula-scheduler`/`-worker`/`-background` crate exists" | grep workspace `Cargo.toml` members → no scheduler/worker/background. ✓ load-bearing claim | ✓ TRUE. **But** parenthetical enumeration is incomplete — see finding 4.1 below. |
| 7 | §4.6: cites 🔴-4 from Phase 1 | `02-pain-enumeration.md` line 69 = "1.4 🔴 Drain-abort phase corruption" | ✓ correct row identity |
| 8 | §4.8: cites 5 consumers (action, sdk, engine, plugin, sandbox) | `grep "nebula-resource" crates/*/Cargo.toml` → exactly action, sdk, engine, plugin, sandbox + resource itself + resource/macros (sub-crate) | ✓ all 5 still depend on nebula-resource |
| 9 | §4.6: `runtime/managed.rs:93-102` `set_failed` dead-coded | Read lines 93-102 → matches `#[expect(dead_code, ...)] pub(crate) fn set_failed(...)` | ✓ exact match |
| 10 | §4.6: `manager.rs:1493-1510` Abort path | Read 1493-1510 → matches `DrainTimeoutPolicy::Abort` arm with `set_phase_all(Ready)` on 1507, `Err(DrainTimeout)` on 1509 | ✓ exact match |
| 11 | §4.9: `events.rs` no `CredentialRefreshed`/`CredentialRevoked` (NEW variants) | grep → only `enum ResourceEvent {` declaration found, no matching variants | ✓ correctly labeled NEW |

**11/11 PASS.** No off-by-N or load-bearing drift. Citation hygiene materially improved over CP1 (CP1 had 14/17, with 3 off-by-N drifts).

---

## 4. §4.4 engine-fold pick — evidence-grounding (NOT a design judgment)

Per dispatch instructions, I am NOT auditing whether engine-fold is the right pick — that's tech-lead territory. I AM auditing whether the cited evidence is verifiable and the rationale is internally consistent.

**Architect's three evidence points (§4.4 line 270):**

### 4.1 No precedent for sibling crate

Strategy: "no `nebula-scheduler` / `nebula-worker` / `nebula-background` crate today (workspace grep confirmed: only `nebula-action`/`-api`/`-core`/`-credential`/`-engine`/`-error`/`-eventbus`/`-execution`/`-expression`/`-log`/`-metadata`/`-metrics`/`-plugin`/`-plugin-sdk`/`-resilience` exist)."

**Verification:** workspace `Cargo.toml` `[workspace.members]` lists 31 entries. Strategy lists 15 crates, but the workspace has these top-level crates (excluding `-resource` and macro sub-crates): action, api, core, credential, engine, error, eventbus, execution, expression, log, metadata, metrics, plugin, plugin-sdk, resilience, **schema, sandbox, sdk, storage, system, telemetry, validator, workflow** — 23 distinct top-level non-macro crates.

Strategy enumeration omits 8 crates: `schema`, `sandbox`, `sdk`, `storage`, `system`, `telemetry`, `validator`, `workflow`.

**Load-bearing impact:** the conclusion ("no scheduler-shaped crate exists") is TRUE — none of the 8 omitted crates are scheduler-shaped. But a reader who counts the enumeration to verify the claim will get a count that doesn't match the workspace. **🟡 MEDIUM finding** — the rationale's conclusion holds, but the enumeration is misleading. See finding 5.1.

### 4.2 TriggerAction precedent

Strategy: "engine already dispatches event-driven trigger lifecycles per canon §3.5; EventSource is conceptually a thin extension."

**Verification:** `PRODUCT_CANON.md` line 82: "Action — what a step does. Dispatch via action trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`)." `INTEGRATION_MODEL.md:99` lists the same trait family. ✓ TriggerAction is canon.

**Load-bearing impact:** evidence verified. The "thin extension" framing is an architect/tech-lead judgment, not an audit-substance claim. ✓

### 4.3 `feedback_active_dev_mode.md` (atomic landing)

Strategy: "bundling extraction with the credential reshape (one PR wave) means migrating 5 in-tree consumers' Daemon/EventSource references in the same atomic change — splitting it across two crates' migrations doubles consumer churn for no upside."

**Verification:** `feedback_active_dev_mode.md` is invoked for "atomic landing" — consistent with §2.4 line 195 invocation. Internally consistent.

**Load-bearing impact:** rationale self-consistent. ✓

### 4.4 Internal consistency check

Strategy §0 line 38 says "Sub-spec material (Daemon/EventSource landing site design in engine/scheduler, ...) tracked in §6 post-validation roadmap." But §4.4 *picks* the target (engine-fold). Reading carefully: "landing site **design**" (DaemonRegistry shape, EventSource→Trigger adapter) is post-validation; "landing site **target**" (engine vs sibling crate) is decided in §4.4. The distinction holds — though it's load-bearing on the reader inferring "design vs target" from §0. **🟢 LOW** — could benefit from a single-word clarification ("landing site **design**" vs "target picked in §4.4"); not blocking.

**Verdict on §4.4 evidence-grounding:** the load-bearing claim ("no scheduler-shaped crate exists") is TRUE per workspace evidence. The supporting enumeration is incomplete (8 crates omitted), which is a 🟡 MEDIUM finding for §4.4's parenthetical text but does NOT undermine the conclusion. The other two rationale points (TriggerAction precedent + active-dev atomic landing) are evidence-clean.

---

## 5. Findings

### 5.1 🟡 MEDIUM — §4.4 line 270 workspace-crate enumeration is incomplete

**Claim:** "only `nebula-action`/`-api`/`-core`/`-credential`/`-engine`/`-error`/`-eventbus`/`-execution`/`-expression`/`-log`/`-metadata`/`-metrics`/`-plugin`/`-plugin-sdk`/`-resilience` exist"

**Evidence:** workspace `Cargo.toml` lines 2-37 contain 31 members. Top-level non-macro crates excluding `-resource`: action, api, core, credential, engine, error, eventbus, execution, expression, log, metadata, metrics, plugin, plugin-sdk, resilience, **schema, sandbox, sdk, storage, system, telemetry, validator, workflow**. Strategy lists 15 of 23.

**Impact:** the conclusion ("no scheduler/worker/background crate") is TRUE, so the load-bearing argument holds. But the enumeration as evidence is incomplete — a reader cross-checking against workspace gets a count mismatch and has to re-verify whether any omitted crate IS scheduler-shaped (none are, but it's reader-tax).

**Suggested fix:** either (a) complete the enumeration: "...`-resilience`, `-sandbox`, `-schema`, `-sdk`, `-storage`, `-system`, `-telemetry`, `-validator`, `-workflow` exist" — or (b) replace the enumeration with the load-bearing assertion alone: "workspace `Cargo.toml` lists no `nebula-scheduler`/`-worker`/`-background` crate (verified: 31 members; 0 scheduler-shaped)."

### 5.2 🟡 MEDIUM — §0 line 61 vs §5.2 internal contradiction on AcquireOptions ownership

**Claim (§0 line 61):** "🟠-8 (`AcquireOptions::intent/.tags` → engine #391; CP2 §4 picks interim treatment + cross-ref §5.2)"

**Claim (§5.2):** "Out-of-scope per §3.3. Phase 6 §5 picks: (a) `#[doc(hidden)]`, (b) `#[deprecated(note = ...)]`. Owner: tech-lead in **Phase 6 §5**."

**Contradiction:** §0 line 61 says "**CP2 §4** picks interim treatment"; §5.2 says "**Phase 6 §5** picks." §4 contains no interim treatment decision for `AcquireOptions::intent/.tags`. The reality is §5.2 (Phase 6 picks), not §0's claim.

**Impact:** reader verifying §0's promise against §4 finds no decision; reader following §5.2 sees the actual punt. Minor bookkeeping rot — closer to a typo than a structural issue, but it's the kind of thing CP3 freeze should not preserve.

**Suggested fix:** §0 line 61 → "🟠-8 (`AcquireOptions::intent/.tags` → engine #391; **CP2 §5.2 records as open item; Phase 6 §5 picks interim treatment**)"

### 5.3 🟡 MEDIUM — §5.6 is not really "open"; should be in CP1+CP2 consolidated section as RESOLVED

**Claim (§5.6):** "Phase 4 spike trigger confirmed. CP2 locks: spike runs (trait reshape needs §3.6-shape ergonomic + perf validation per §2.4 exit criteria). Scope + exit already locked in `03-scope-decision.md:145-156`."

**Issue:** §5 is "what CP2 cannot fully resolve and Phase 6 Tech Spec or future cascade must answer" (per §5 header line 340). §5.6 is CONFIRMED, not open — it's a status acknowledgment, not an item awaiting a decision. The header structure ("question / who answers / when / what depends") doesn't fit §5.6 because there is no open question.

**Impact:** semantic drift. A reader scanning §5 for "what's still unresolved post-CP2" gets noise from §5.6.

**Suggested fix:** move §5.6 to the consolidated CP1+CP2 section (lines 360-369) as a RESOLVED item, OR retitle §5 as "open items + status confirmations" if the architect wants the spike-go-no-go visible there.

### 5.4 🟢 LOW — terminology variance: "blue-green pool swap" / "blue-green pattern" / "blue-green swap pattern"

**Locations:**
- §1.1 line 82: "blue-green pool swap"
- §3.2 line 220: "blue-green pattern"
- §4.1 line 240: "blue-green pool swap"
- §4.2 line 248: "blue-green swap pattern"

**Impact:** three slightly different phrasings for the same canonical pattern (credential Tech Spec §3.6 lines 959-993). Reader cannot tell whether the variance is meaningful. Minor readability.

**Suggested fix:** pick one phrasing (recommend: "blue-green pool swap" since it appears in §1.1 + §4.1 and is the most descriptive) and use it consistently across all four sites.

### 5.5 🟢 LOW — `engine-fold` term inconsistency: heading is "engine fold" (no hyphen), body uses "engine-fold"

**Locations:**
- §4.4 line 266 heading: "Daemon + EventSource extraction target — engine fold" (space, no hyphen)
- §4.4 line 270, 272 body: "engine-fold" (hyphenated)
- §5.1 line 342: "engine-fold" (hyphenated)

**Impact:** the term is coined in §4.4 but not formally defined on first use; the heading uses an un-hyphenated form while the body hyphenates. Trivial cosmetic.

**Suggested fix:** change §4.4 heading to "engine-fold" to match body usage. Optionally, define the term parenthetically on first occurrence: "engine-fold (the topology landing site is folded into the engine layer rather than extracted to a sibling crate)".

### 5.6 🟢 LOW — §2.4 cites `phase-2-tech-lead-review.md:80-94` (range covers both amendments)

**Claim:** §2.4 line 196 cites tech-lead amendment 1 at lines 80-94. But amendment 1 is at lines 82-88; amendment 2 is at lines 90-96. Range 80-94 covers both amendments + the "Two bounded amendments" preamble at line 80.

**Impact:** if reader needs to find amendment 1 specifically, the cited range over-targets. Not load-bearing — content is correct.

**Suggested fix:** narrow citation to `:82-88` for amendment 1 only.

---

## 6. §5 open-item bookkeeping

Six open items. Each evaluated for: clear question / owner / when / dependency.

| § | Question | Owner | When | Dependency | Verdict |
|---|---|---|---|---|---|
| §5.1 | When to revisit engine-fold vs sibling? | engine team via Phase 6 §13 | Trigger: Daemon engine code >500 LOC OR ≥2 non-trigger workers | §0 amendment cycle | ✓ Complete |
| §5.2 | Interim treatment for AcquireOptions? | tech-lead in Phase 6 §5 | Phase 6 §5 | engine #391 | ✓ Complete (but see finding 5.2 for §0 contradiction) |
| §5.3 | Runtime/Lease collapse trigger? | future cascade orchestrator | Trigger: any consumer sets `Runtime != Lease` | future cascade | ✓ Complete |
| §5.4 | Convenience method symmetry under NoCredential? | rust-senior + dx-tester via Tech Spec CP2a | Tech Spec CP2a | trait reshape lock | ✓ Complete |
| §5.5 | Credential §3.6 revoke extension dependency or follow-up? | spec-auditor + credential Tech Spec author | (Implicit: before Phase 6 dispatch lands) | §4.2 | ⚠ "When" is implicit — could be tighter |
| §5.6 | (None — status confirmation) | (N/A) | (N/A) | (N/A) | ⚠ Doesn't fit §5 schema — see finding 5.3 |

**Coverage check against `02-pain-enumeration.md` 🟠/🟡:**
- 🟠-8 (AcquireOptions) → §5.2 ✓
- 🟠-11 (Runtime/Lease) → §5.3 ✓
- 🟠-13 (Transport tests) → §0 line 61 (deferred-with-pointer) — not §5 because absorbed elsewhere ✓
- 🟡-16 (AuthScheme: Clone) → §0 line 61 (credential-side cascade) ✓
- 🟡-17 (warmup default) → §4.9 (resolved) ✓
- 🟡-18 (CredentialId import) → §0 line 61 (drive-by) ✓
- 🟡-20 (Resource::destroy default) → §0 line 61 (Phase 4 spike may surface) ✓

No 🟠/🟡 silently absorbed. ✓

**Items that should have been §4 decisions but weren't:**
- §5.4 (convenience method symmetry under NoCredential) — arguably this should be a CP2 §4 decision because it's load-bearing on §4.5 (file-split) and §4.8 (consumer migration). But Strategy explicitly defers to Tech Spec CP2a, which is consistent with §0's "no Tech Spec content" framing (line 32). Acceptable.

---

## 7. Terminology coherence (full pass)

### 7.1 Type / hook names

- `NoCredential` — capitalized consistently, 4 occurrences ✓
- `on_credential_refresh` — used consistently for the new-shape resource hook ✓
- `on_credential_revoke` — used consistently for the new-shape revoke hook ✓
- `on_credential_refreshed` / `on_credential_revoked` — used in §1.1 to describe **current broken** Manager methods (past tense distinguishes) ✓ — deliberate distinction, not drift
- `Auth` / `AuthScheme` / `Credential` / `Scheme` — used in correct contexts (current vs target shape) ✓

### 7.2 Phrasing variance

- "blue-green pool swap" / "blue-green pattern" / "blue-green swap pattern" — 🟢 LOW (finding 5.4)
- "engine-fold" / "engine fold" — 🟢 LOW (finding 5.5)

### 7.3 Acronyms

- "DoD" used in §2.4, §4.9 — defined contextually via `feedback_observability_as_completion.md` reference. Acceptable.
- "RPITIT" used in §2.4 — not expanded; standard project vocabulary. Acceptable.
- "MSRV" used in §2.4 — standard term. Acceptable.

---

## 8. Forward/backward reference resolution

### 8.1 Forward references in §4 to other sections

All §4 forward refs (§5.1, §5.5, Phase 6 Tech Spec §3, §5, §6, §13, Phase 5 ADR, Phase 4 spike) are gracefully framed — Phase 6/5/4 are placeholder targets that don't require resolution within Strategy.

### 8.2 Backward references in §4 to §1, §2, §3

All resolve. Tabulated in §2.1 above.

### 8.3 §5 ↔ §4 ↔ §0 consolidated open-items section

- §0 lines 360-369 (CP1 + CP2 consolidated):
  - "§2.3 revocation extension — RESOLVED in §4.2" — verified ✓
  - "§2.4 warmup semantics — partially resolved in §4.9; exact signature deferred to Phase 6 Tech Spec §5" — verified ✓
  - "§2.1 ADR-0035 amendment — RESOLVED: per tech-lead Ratification answer to ambiguity #2, no new ADR needed" — context-only; can't verify the tech-lead Ratification answer without that artefact, but architect's claim that #2 closed is internally consistent
  - "§3.3 AcquireOptions resurrection trigger — carried to §5.2" — verified ✓

All four CP1 open items have correct status updates. ✓

---

## 9. Audit verdict

**PASS_WITH_MINOR.**

Rationale:
- 0 BLOCKERs.
- 11/11 spot-checked claims verified at exact file:line — citation hygiene materially improved over CP1 (14/17). All previously-flagged off-by-N citations (CP1 §9.3, §9.4, §9.5) are corrected.
- All 10 CP1 ratification edits landed correctly. Previously-flagged 🟠-10 + 🟠-13 silent-absorption gaps are CLOSED at §0 lines 61, 64.
- §4 honors LOCKED scope per `03-scope-decision.md §4` exactly — 9 decisions across 9 subsections, all aligned.
- §4 cross-section consistency is clean (forward refs to §5 resolve; backward refs to §1/§2/§3 resolve).
- §5 open-items bookkeeping is largely complete — 5 of 6 items have full owner/when/dependency. §5.6 doesn't fit the schema (finding 5.3); §5.2 has minor §0 contradiction (finding 5.2).
- §4.4 engine-fold rationale is internally consistent. Three evidence points: (1) workspace enumeration incomplete but conclusion holds; (2) TriggerAction precedent verified; (3) atomic-landing reasoning self-consistent. Auditor does NOT judge whether engine-fold is the right pick.
- Terminology coherence largely holds; minor phrasing variance ("blue-green pool swap" vs "blue-green pattern" vs "blue-green swap pattern"; "engine fold" vs "engine-fold").

**Recommended action:** proceed to CP3 dispatch. Address findings 5.1-5.6 at architect's discretion before CP3 freeze. None block CP3 progression.

---

## 10. Required amendments

**(none — verdict is PASS_WITH_MINOR; no amendments are blockers for CP3 progression)**

---

## 11. Nice-to-have amendments

Listed in priority order. Architect may batch with CP3 work.

### 11.1 §4.4 line 270 — complete or replace workspace enumeration

**Why:** the 15-crate enumeration omits 8 crates from the actual 23. The conclusion ("no scheduler-shaped crate") is TRUE, but the supporting enumeration is incomplete. Reader cross-checking against `Cargo.toml` gets a count mismatch.
**Severity:** 🟡 MEDIUM.
**Fix:** either complete the list (add `-sandbox`, `-schema`, `-sdk`, `-storage`, `-system`, `-telemetry`, `-validator`, `-workflow`) OR replace with conclusion-only: "workspace `Cargo.toml` lists no `nebula-scheduler`/`-worker`/`-background` crate (verified: 31 members; 0 scheduler-shaped)."

### 11.2 §0 line 61 — fix CP2 §4 vs Phase 6 §5 ownership claim for AcquireOptions

**Why:** §0 line 61 says "CP2 §4 picks interim treatment" but §5.2 says "Phase 6 §5 picks." §4 has no interim treatment for AcquireOptions. Reader-misleading.
**Severity:** 🟡 MEDIUM.
**Fix:** §0 line 61 → "🟠-8 (`AcquireOptions::intent/.tags` → engine #391; **CP2 §5.2 records as open item; Phase 6 §5 picks interim treatment**)"

### 11.3 §5.6 — move to consolidated section as RESOLVED, or retitle §5

**Why:** §5.6 is a status confirmation (Phase 4 spike runs), not an open item. Doesn't fit the §5 owner/when/dependency schema.
**Severity:** 🟡 MEDIUM.
**Fix:** either move §5.6 to the CP1+CP2 consolidated section (lines 360-369) as a RESOLVED entry, OR retitle §5 to "Open items + status confirmations."

### 11.4 §1.1 / §3.2 / §4.1 / §4.2 — pick one phrasing for blue-green

**Why:** four sites use three different phrasings. Minor readability cost.
**Severity:** 🟢 LOW.
**Fix:** standardize on "blue-green pool swap" across §1.1 line 82, §3.2 line 220, §4.1 line 240, §4.2 line 248.

### 11.5 §4.4 heading — hyphenate "engine-fold" to match body usage

**Why:** heading is "engine fold" (space); body and §5.1 use "engine-fold". Trivial cosmetic.
**Severity:** 🟢 LOW.
**Fix:** §4.4 line 266 heading → "Daemon + EventSource extraction target — engine-fold".

### 11.6 §2.4 line 196 — narrow citation for tech-lead amendment 1

**Why:** cited range `:80-94` covers both amendments + preamble. Amendment 1 alone is at `:82-88`.
**Severity:** 🟢 LOW.
**Fix:** change citation to `phase-2-tech-lead-review.md:82-88`.

### 11.7 §0 line 38 — clarify "design" vs "target" for Daemon/EventSource

**Why:** §0 line 38 says "Daemon/EventSource landing site **design** ... tracked in §6 post-validation roadmap" but §4.4 picks the **target** (engine-fold). Holds together if reader infers design ≠ target, but a one-word clarification helps.
**Severity:** 🟢 LOW.
**Fix:** §0 line 38 → "Sub-spec material (Daemon/EventSource landing site **internal design** in engine — DaemonRegistry shape, EventSource→Trigger adapter shape; the target was picked in §4.4, ...)"

---

## 12. Coverage summary

- **Structural:** PASS — all forward/backward refs resolve.
- **Consistency:** PASS — §4 honors LOCKED scope; cross-section refs clean.
- **External verification:** 11/11 PASS — no off-by-N drift; load-bearing claims all hold.
- **Bookkeeping:** PASS_WITH_MINOR — 1 §0/§5.2 contradiction (finding 5.2), 1 §5.6 schema-mismatch (finding 5.3); CP1 audit's 🟠-10 + 🟠-13 silent-absorption gaps CLOSED.
- **Terminology:** PASS_WITH_MINOR — load-bearing names clean; minor phrasing variance on "blue-green" + "engine-fold".
- **Definition-of-done (`docs/PRODUCT_CANON.md` §17):** N/A at Strategy level — applies to Tech Spec / implementation, not Strategy §0-§5 framing.

---

## 13. Recommended handoff

- **architect:** address findings 5.1 (§4.4 enumeration) + 5.2 (§0 vs §5.2 ownership) + 5.3 (§5.6 schema) before CP3 freeze. Findings 5.4-5.6 at discretion.
- **tech-lead:** no audit-derived action required. Tech-lead's own ratification of §4 decisions (per CHANGELOG §"Handoffs requested") is independent of this audit.
- **security-lead:** no audit-derived action required. Architect's CHANGELOG handoff to security-lead (verify §4.2 + §4.3 + §4.9 against Phase 2 amendments) is independent of this audit; that's a content-validation handoff, not an audit-finding handoff.
- **orchestrator:** verdict is PASS_WITH_MINOR — proceed with CP3 dispatch. Architect may batch findings 5.1-5.6 with CP3 work.

---

*End of audit.*
