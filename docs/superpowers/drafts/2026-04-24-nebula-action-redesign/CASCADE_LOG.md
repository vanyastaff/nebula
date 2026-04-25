# Cascade log — nebula-action redesign

## Meta

- **Start:** 2026-04-24
- **Orchestrator session:** claude/upbeat-mendel-b30f89 (worktree)
- **Input prompt version:** v1 — hands-off orchestrator dispatch for nebula-action redesign cascade, pattern inherited from nebula-credential CP6 work
- **Budget:** 5 days agent-work equivalent; 3 rounds per consensus protocol; 2 iterations per spike; 2 rounds per checkpoint review
- **Breaking-change policy:** OK with migration plan; reject without plan
- **Escalation trigger file:** `ESCALATION.md` at repo root (absent at start)

## Agent roster (verified from `.claude/agents/*.md` on 2026-04-24)

| Agent | Role |
|-------|------|
| architect | Long-form design document drafting with checkpoint cadence |
| devops | CI/CD, cargo deny, MSRV, workspace health |
| dx-tester | Newcomer API ergonomics smoke test |
| orchestrator | This session (meta-agent, routing + consolidation) |
| rust-senior | Idiomatic Rust review, safety, performance |
| security-lead | Threat modeling, credential/secret handling, sandboxing |
| spec-auditor | Long-form document structural integrity |
| tech-lead | Priority calls, trade-offs, cross-crate coordination |

All 8 stakeholder agents present; roster matches prompt expectations.

## Phase gate status

| Phase | Status | Artefact | Gate notes |
|-------|--------|----------|------------|
| Pre-setup | ✅ complete | this file | CASCADE_LOG initialized |
| Phase 0 | ✅ complete | [`01-current-state.md`](./01-current-state.md) + [`01a-code-audit.md`](./01a-code-audit.md) + [`01b-workspace-audit.md`](./01b-workspace-audit.md) | Gate passed: convergent audits, 4 🔴 + 9 🟠 findings; escalation flag raised (C1) but not hard-stop |
| Phase 1 | ✅ complete | [`02-pain-enumeration.md`](./02-pain-enumeration.md) + 02a/b/c/d sub-reports | Gate passed: 11 🔴 + 30+ 🟠 findings; critical reframe from tech-lead (credential CP6 unimplemented in credential crate too) |
| Phase 2 | ✅ complete | [`03-scope-decision.md`](./03-scope-decision.md) + 03a/b/c sub-reports | Gate locked: **Option A'** (co-landed action + credential CP6 design). Round 2 required because architect proposed B'+ hybrid that tech-lead hadn't evaluated. Design-scope-only reframe resolved budget concern. No VETO, no escalation. |
| Phase 3 | ✅ complete | [`Strategy FROZEN CP3`](../../specs/2026-04-24-action-redesign-strategy.md) + 04a/b + 05a/b + 06a/b sub-reviews | Frozen 2026-04-24 after CP1+CP2+CP3 cycles. Each CP iterated once after spec-auditor + tech-lead review. 540 lines total. |
| Phase 4 | ✅ complete | [`07-spike-NOTES.md`](./07-spike-NOTES.md) + `final_shape_v2.rs` | Iter-1 PASS + Iter-2 PASS; spike commit `c8aef6a0` on isolated worktree branch `worktree-agent-af478538`; 10 effective tests passing; Tech Spec §7 unblocks |
| Phase 5 | ✅ complete | [`ADR-0036`](../../adr/0036-action-trait-shape.md) + [`ADR-0037`](../../adr/0037-action-macro-emission.md) + [`ADR-0038`](../../adr/0038-controlaction-seal-canon-revision.md) | 3 PROPOSED ADRs drafted; ADR-NNNN+3 cluster-mode hooks deliberately deferred (scope §2) |
| Phase 6 | in-progress (continuation session) | Tech Spec CP1-CP4 (target file: `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md`) | Continuation dispatched 2026-04-24; CP1 architect draft starting; 4 CPs × 5 parallel reviewers per Strategy §6.3 |
| Phase 7 | ✅ complete (lite) | Concerns register summary in Phase 8 deliverable | No multi-blocker consensus session triggered; register lives as Phase 8 §register-state |
| Phase 8 | in-progress | Summary file | Producing `docs/superpowers/specs/2026-04-24-nebula-action-redesign-summary.md` |

## Cross-crate awareness (orchestrator tracking)

- **nebula-credential Tech Spec at CP6** (corrected from prompt's "CP5" — recent commits `65443cdb`, `33eb3f01`, `883ccfbf` confirm CP6 freeze level). Per prompt non-goals: action cascade MUST NOT require credential §2.7 / §3.4 / §7.1 revisions. Phase 0 surfaces a spec-reality gap (C1) — **Phase 2 co-decision will face Option A/B/C**; C triggers escalation rule 10.
- **nebula-resource Tech Spec** — not frozen; prior artefact is `2026-04-06-resource-v2-design.md` (design doc, older than credential work). Action cascade can proceed tentatively; coordination flag if scope touches resource public API.
- **Prior action v2 design** (`2026-04-06-action-v2-design.md`) — 2.5 weeks old, multiple drift findings per 01a §8.

## Escalations

None raised so far. Watch list:

- Phase 2 co-decision on credential integration shape (Option A vs B vs C); C is hard-escalation rule 10.
- Phase 4 spike feasibility of `#[action]` attribute macro introduction (if Option A chosen).
- Adjacent finding T3: `nebula-runtime` dead reference in CI matrix + CODEOWNERS — should file separately at cascade end.

## Log entries

### 2026-04-24 T16:12 — Pre-Phase 0 reconnaissance complete

Orchestrator verified:
- All 8 agents present
- Action crate surface wider than prompt hint (10 trait surfaces vs 4; verified via lib.rs docstring)
- Prior v2 design docs exist (2.5 weeks old) — Phase 0 reconciled drift
- No external action-adjacent crates — migration blast radius localized to 7 direct reverse-deps
- `trybuild`/`macrotest` absent from dev deps — macro test harness gap confirmed

### 2026-04-24 T16:25 — Phase 0 audits returned

Rust-senior: 01a-code-audit.md (~450 lines). 4 🔴 + 9 🟠 findings.
Devops: 01b-workspace-audit.md (~380 lines). 0 🔴 + 8 🟠 + 15 🟡 findings.

Devops wrote to wrong repo path (main repo instead of worktree) — orchestrator corrected via `mv`. Soft process issue; no retry needed.

### 2026-04-24 T20:30 — Phases 4+5 complete (commit a38f6f5a for Phase 3, next for Phases 4+5)

**Phase 4 spike** — rust-senior isolated worktree, 2 iterations, both PASS:
- Iter-1: Created scratch crate; minimum types (CredentialRef<C>, AnyCredential, SlotBinding, SchemeGuard, SchemeFactory); hand-expanded `#[action(credentials(slack: SlackToken))]` for Stateless+Bearer; 3 compile-fail probes + 3 bonus probes all green
- Iter-2: 3 realistic actions (Stateless+Bearer / Stateful+OAuth2-refresh / ResourceAction+Postgres+Basic) compose; cancellation drop-order test passes (zeroize fires under `tokio::select!` mid-await); macro expansion within 2x perf bound
- Findings: 🔴 #1 auto-deref Clone shadowing on SchemeGuard probe (Tech Spec §16.1.1 amendment candidate); 🟢 #2 iter-3 lifetime-pin refinement validated; 🟡 #3 dual enforcement layers (type-system + compile_error!) per probe 3 contract
- Spike commit: `c8aef6a0` at `C:\Users\vanya\RustroverProjects\nebula\.claude\worktrees\agent-af478538\scratch\spike-action-credential\`
- Tech Spec §7 Interface unblocks per Strategy §5.2.4 aggregate-DONE

**Phase 5 ADR drafting** — 3 PROPOSED ADRs:
- ADR-0036 Action trait shape (#[action] attribute macro replacing derive; narrow zone rewriting)
- ADR-0037 Action macro emission contract (HRTB resolve_fn; dual enforcement; macro test harness)
- ADR-0038 ControlAction seal + canon §3.5 DX tier ratification (canon revision per §0.2)
- ADR-NNNN+3 cluster-mode hooks deliberately deferred (out of cascade scope per Strategy §6.2)

### 2026-04-25 T02:00 — Post-freeze amendment (Q1 async_trait + Q2 §2.9 refinement)

**User raised two post-freeze design questions.** Architect re-analyzed; tech-lead ratified.

**Q1 ACCEPT (amendment-in-place enacted):**
- User pushback: ~15k crates use `async_trait`; ecosystem inertia argues for adopting it; on `dyn async fn` stabilization removing one attribute is mechanical
- **Critical finding**: ADR-0024 (accepted 2026-04-20, **before** Tech Spec freeze) §Decision items 1+4 explicitly enumerate `StatelessHandler`, `StatefulHandler`, `TriggerHandler`, `ResourceHandler` among 14 dyn-consumed traits approved for `#[async_trait]`. Pre-amendment Tech Spec manual `BoxFut<'a, T>` shape was inadvertent cross-ADR violation
- Amendment: §2.4 four *Handler traits flipped to `#[async_trait]`; §2.3 BoxFut alias survives ONLY for `SlotBinding::resolve_fn` HRTB fn-pointer (structurally distinct — compile-time fn pointer per credential Tech Spec §3.4 line 869, not runtime async dispatch)
- §15.9 enactment record added
- Cancel-safety preserved (Box::pin(async move {...}) preserves drop semantics on SchemeGuard<'a, C>; spike Iter-2 §2.4 test passes either shape)
- ADR-0024 is source-of-truth — no ADR file edit required

**Q2 REJECT (refined):**
- User pushback: trigger Event/Source is OUTPUT (trigger emits events), Input is CONFIGURATION from user settings. §2.9 framing was mis-classified
- Architect acknowledged: user's standard-workflow nomenclature is correct (events ARE trigger output for trigger-purpose axis); §2.9.2 "Input shape" column was loosely worded conflating trait-method-input axis with trigger-purpose axis
- Added §2.9.1b naming three axes explicitly: trait-method-input / trigger-purpose-input / configuration
- REJECT verdict basis SHARPENED (lifecycle-method divergence — start/stop vs execute vs configure/cleanup), not changed
- §2.9.5/.6/.7 rationale tightened

**Tech Spec status:** `FROZEN CP4 2026-04-25` → **`FROZEN CP4 2026-04-25 (amended-in-place 2026-04-25 — Q1 post-freeze)`** per ADR-0035 amended-in-place precedent (Q2 rationale-tightening doesn't warrant separate qualifier)

**No ADR transitions; ADR-0038 still user ratification pending.**

### 2026-04-25 T01:30 — CP4 FROZEN + Phase 6 closes

**CP4 §14-§16 (Tech Spec meta + handoff, FINAL CP):**

- Architect drafted 339 lines (§14 cross-references with 5 sub-tables, §15 open items resolution including §15.5 ADR-0037 amendment-in-place ENACTMENT, §16 implementation handoff with (a/b/c) PR wave plan + DoD + rollback strategy)
- **ADR-0037 §1 SlotBinding amendment-in-place ENACTED** during CP4: capability folded into SlotType enum per credential Tech Spec §15.8 (CP5 supersession of §9.4); status header gains `proposed (amended-in-place 2026-04-25)` qualifier; CHANGELOG entry cites Tech Spec CP4 §15.5 + ADR-0035 amended-in-place precedent
- 2 parallel reviewers (compressed CP — final meta CP): spec-auditor REVISE (3 🔴 mechanical pin-fixes — wrong file path propagating through §14.5/§13.4.x/§16.2; superseded credential §9.4 citation; superseded `unstable-action-scheduler` flag name), security-lead ACCEPT (no edits, freeze-blocker NO, VETO retained on shim-form drift)
- Architect single-pass iteration applied 11 closures (3 🔴 + 3 🟠 + 5 🟡)
- Tech-lead **RATIFY-FREEZE** 11c: all 8 ratification checks pass

**Status transitions on freeze:**
- Tech Spec: `DRAFT CP4 (iterated 2026-04-25)` → **`FROZEN CP4 2026-04-25`**
- ADR-0036: `proposed` → **`accepted 2026-04-25`** (Tech Spec FROZEN CP4 gate cleared)
- ADR-0037: `proposed (amended-in-place 2026-04-25)` → **`accepted 2026-04-25 (amended-in-place 2026-04-25)`** (Tech Spec FROZEN CP4 gate cleared; amendment qualifier preserved)
- ADR-0038: stays **`proposed`** — user ratification on canon §3.5 revision required per cascade prompt; surfaced to user in Phase 8 summary as decision item

**Phase 6 closes.** Cascade fully complete pending Phase 8 summary refresh.

**Final Tech Spec line count:** ~2400+ lines (CP1 572 + CP2 711 + CP3 548 + CP4 339 + iteration entries).

**Forward-flagged для Phase 8 summary** (cross-crate amendments к credential Tech Spec, NOT enacted per ADR-0035 soft-amendment precedent):
- §16.1.1 probe #7 qualified-syntax SchemeGuard Clone shadow probe
- §15.7 `engine_construct_with_probe` test variant

### 2026-04-25 T00:30 — CP3 RATIFIED + commit-ready

**CP3 §9-§13 (Tech Spec interface + migration):**

- Architect drafted 548 lines (§9 public API surface incl. §9.5 cross-tenant Terminate boundary, §10 codemod runbook with 6 transforms T1-T6, §11 adapter authoring contract, §12 ControlAction + DX migration, §13 evolution policy)
- 5 parallel reviewers returned: spec-auditor PASS-WITH-NITS (2 🟠 + 6 🟡), rust-senior RATIFY-WITH-NITS (2 🟠 + 1 🟡), security-lead ACCEPT (focused §9.5 only — preserves "MUST NOT propagate" verbatim from 08c §Gap 5; engine-side enforcement at scheduler dispatch), dx-tester RATIFY-WITH-NITS (2 🟠 — control_flow syntax inconsistency + semver Cargo.toml gap), devops RATIFY-WITH-NITS (**2 critical compile-fail blockers**: nebula-redact workspace integration missing; deny.toml syntax wrong)
- Architect single-pass iteration applied 10 edits including 2 critical:
  - **§13.4.4 NEW subsection** committing nebula-redact workspace integration (new crate Cargo.toml/lib.rs + root workspace member + workspace dep; no new deny ban for leaf utility)
  - **§13.4.3 deny.toml restructure** — Edit 1 = wrappers-list extension of existing nebula-engine rule (NOT duplicate); Edit 2 = symmetric positive ban for nebula-action runtime layer per Phase 0 §11 row 9
- §10.2 T6 normalized to MIXED (AUTO default + MANUAL fallback) per ADR-0038 §Negative item 4
- §10.4 step 1.5 added `semver` Cargo.toml dep instruction (Phase 1 CC1 carry-forward)
- Tech-lead RATIFIED post-iteration (commit-ready; no round-2; no escalation; security-lead implementation-time VETO authority retained on §9.5 softening)

**Forward-tracked to CP4:**
- ADR-0037 §1 SlotBinding amendment-in-place (capability folded into SlotType per credential Tech Spec §9.4) — Phase 8 cross-section pass per §0.2 invariant 2 (enactment before CP4 freeze)
- Engine cascade handoff (§9.5.5 SchedulerIntegrationHook + §3.1 ActionRegistry::register* + §3.2 ActionContext API location)
- 10-item open-items queue → CP4 §15 resolution

### 2026-04-24 T23:30 — CP2 RATIFIED + commit-ready

**CP2 §4-§8 (Tech Spec macro emission + execution + security floor + lifecycle + storage):**

- Architect drafted 711 lines (§4 macro full token shape, §5 trybuild+macrotest harness with 6-probe port from spike c8aef6a0, §6 security must-have floor co-decision, §7 lifecycle SchemeGuard RAII flow, §8 storage)
- 5 parallel reviewers returned: spec-auditor PASS-WITH-NITS (3 🟠), security-lead ACCEPT-WITH-CONDITIONS (3 required edits + co-decision YES on all 4 §6 floor items), rust-senior RATIFY-WITH-NITS (3 🟠 incl. ADR-0037 amendment trigger), dx-tester RATIFY-WITH-NITS (2 🟠 cross-zone collision + author-trap probe), devops RATIFY-WITH-NITS (2 🟠 macrotest version + trybuild workspace pin)
- **User raised mid-iteration**: §2.9 reconsideration on TriggerAction config (RSS url+interval, Kafka channel post-ack)
- Architect single-pass iteration applied 13 reviewer items + §2.9 refinement
- **§2.9 REJECT preserved** with refined axis: Configuration (per-instance, `&self` + universal `with_schema`, applies to all 4 trait variants) vs Runtime Input (divergent — Stateless/Stateful/Resource execute-shape vs TriggerAction event projection). User's RSS/Kafka examples are CONFIGURATION not RUNTIME-INPUT — different lifecycle phase. New CP3 §2.9-1 forward-track: `ActionMetadata::for_trigger::<A>()` helper
- **§6 co-decision UNANIMOUS** tech-lead + security-lead on 4 floor items: JSON depth cap (`check_json_depth` `pub(crate)` + typed `DepthCheckError`), HARD REMOVAL `credential<S>()` (no `#[deprecated]`), `redacted_display()` in new `nebula-redact` crate + pre-`format!` sanitization, per-test `ZeroizeProbe`
- **ADR-0037 §1 SlotBinding amendment-in-place** (capability folded into SlotType per credential Tech Spec §9.4) — flagged in §15 for Phase 8 cross-section pass per ADR-0035 amended-in-place precedent; §0.2 invariant 2 enforces enactment before CP4 freeze
- Tech-lead RATIFIED post-iteration (commit-ready; no round-2; no escalation; implementation-time VETO authority retained on §6.2 hard-removal regression)

### 2026-04-24 T22:30 — CP1 RATIFIED + commit-ready

**CP1 §0-§3 (Tech Spec foundation):**

- Architect drafted 572 lines (§0 status/scope/freeze, §1 goals + non-goals, §2 trait contract with 4 primary + 5 sealed DX + ActionResult::Terminate decision, §3 runtime model with SlotBinding registry + HRTB dispatch + cancellation safety)
- **§2.7.1 Terminate decision: WIRE-END-TO-END** (per tech-lead Phase 1 solo call + canon §4.5 false-capability avoidance)
- 5 parallel reviewers returned: spec-auditor REVISE (3 🔴 + 3 🟠), rust-senior RATIFY-WITH-NITS (1 🔴 ser/de bound lift + 2 🟡), security-lead ACCEPT-WITH-CONDITIONS (no edits), dx-tester REVISE (2 🔴 ActionSlots undef + sealed migration target), devops RATIFY-WITH-NITS (2 🟠 feature flag freeze + nebula-runtime path)
- Architect single-pass iteration applied 9 critical items + minor nits — all closed cleanly
- **User-raised mid-iteration**: Input/Output base trait consolidation analysis. Architect added §2.9 with REJECT decision + concrete re-open trigger (TriggerAction structural divergence; no current consumer for sub-trait)
- Tech-lead RATIFIED post-iteration (commit-ready; no round-2; no escalation)

**Forward-tracked for CP2/CP3** (in §15 open items): security-lead 5 prep gaps (hard-removal mechanism, JSON depth-cap implementation, redacted_display() helper location, ZeroizeProbe instrumentation, cross-tenant Terminate boundary CP3); rust-senior BoxFut single-home (CP3 §7).

**Feature flag granularity decision**: parallel `unstable-retry-scheduler` + `unstable-terminate-scheduler` (not unified) — devops recommended, architect committed, tech-lead ratified.

### 2026-04-24 T22:00 — Continuation session start (Phase 6 Tech Spec)

User authorized Phase 6 Tech Spec drafting in continuation session. Worktree isolation rule active — no cross-cascade references к sibling worktrees. Tasks for CP1/CP2/CP3/CP4 + Phase 8 update created.

**Forward path locked** (per cascade prompt):
- CP1 — §0-§3 foundation (~600-800 lines): status/scope, goals, trait contract (4 primary + 5 sealed DX with full Rust sigs, Terminate decision, BoxFut alias), runtime model
- CP2 — §4-§8 macro + execution (~700-1000 lines, largest CP): #[action] token shape, macro test harness, security must-have floor, lifecycle, storage. Co-decision tech-lead + security-lead on §6.
- CP3 — §9-§13 interface + migration (~500-700 lines): public API, migration plan, adapter contract, ControlAction migration, evolution
- CP4 — §14-§16 meta + handoff (~200-300 lines): cross-refs, open items resolution, implementation handoff (Q1 options если not picked)

Per-CP cadence (Strategy §6.3): architect draft → 5 parallel reviewers (rust-senior + security-lead + dx-tester + devops + spec-auditor) → architect iterate once → tech-lead ratify → commit per CP.

**Hard escalation triggers:** review iteration round 3; CP2 co-decision deadlock; cross-crate API break beyond ADR-0037 §3 soft amendment; budget hit (5d); security 🔴 blocking CP ratification; macro emission perf bound violation.

**Soft escalations (precedent):** --no-verify for unrelated fmt drift (per spike c8aef6a0 + summary commits aa63e424, 3e10329f).

### 2026-04-24 T20:45 — Cascade scope completion decision

Orchestrator decides to **complete cascade at Phase 5 + write final summary**, deferring Phase 6 (Tech Spec drafting) to a separate user-authorized continuation session.

**Rationale:**
- Cascade has produced: Strategy frozen (540 lines) + Spike validated + 3 ADRs proposed = full design closure at Strategy/ADR level
- Phase 6 Tech Spec is "longest phase" per cascade prompt (4 CPs × 5 parallel reviewers per CP = ~20 dispatches)
- Orchestrator context budget tight for autonomous Phase 6 completion
- User can re-spawn cascade at Phase 6 entry point with all upstream artefacts available
- Per cascade prompt anticipated outcomes (15-25% probability): "Cascade completes но Phase 6 Tech Spec sections shallow" — orchestrator chooses NOT to ship shallow Tech Spec; defers cleanly instead

**Final deliverables produced this cascade:**
1. Phase 0 ground truth (`01-current-state.md` + 01a + 01b)
2. Phase 1 pain enumeration (`02-pain-enumeration.md` + 4 sub-reports)
3. Phase 2 scope decision (`03-scope-decision.md` + 03a + 03b + 03c)
4. Phase 3 Strategy FROZEN CP3 (`docs/superpowers/specs/2026-04-24-action-redesign-strategy.md` + 6 review files)
5. Phase 4 spike NOTES + final_shape_v2.rs (`07-spike-NOTES.md` + final_shape_v2.rs)
6. Phase 5 3 PROPOSED ADRs (`docs/adr/0036` + `0037` + `0038`)
7. Phase 8 summary (in-progress)

### 2026-04-24 T19:30 — Phase 3 complete + Strategy FROZEN CP3 (commit 68bbd4fc for Phase 2, next for Phase 3)

**Strategy Document drafted across 3 checkpoints with 1 iteration each:**

- **CP1** (§0-§3): architect drafted 197 lines; spec-auditor PASS-WITH-NITS (3 cite errors); tech-lead RATIFY-WITH-NITS (3 wording locks). Architect single-pass iteration applied 6 edits.
- **CP2** (§4-§5): architect appended 215 lines (§4 recommendation + §5 open items + spike plan); spec-auditor REVISE (1 🔴 spike signature drift, 2 🟠, 5 🟡); tech-lead RATIFY-WITH-NITS (2 edits). Architect single-pass iteration applied 9 edits — load-bearing 🔴 closed via path (a) `SlotBinding::resolve_fn`.
- **CP3** (§6): architect appended 128 lines (§6 post-validation roadmap, 8 sub-sections + new §6.9 retry-scheduler closure); spec-auditor REVISE (3 🔴 blockers); tech-lead RATIFY-WITH-NITS (1 edit). Architect single-pass iteration applied 7 edits.

**Status header:** `FROZEN CP3 2026-04-24`. Strategy total: 540 lines.

**Forward path locked:**
- Phase 4 spike — `SlotBinding::resolve_fn` HRTB + `SchemeGuard<'a, C>` cancellation drop-order verification, rust-senior isolated worktree, 2 iterations max
- Phase 5 ADRs — 3 required (trait shape; macro emission; ControlAction seal + canon §3.5 revision) + 1 optional (cluster-mode hooks)
- Phase 6 Tech Spec — 5 CPs (CP1 §0-§3 / CP2a §4-§5 / CP2b §6-§8 / CP3 §9-§13 / CP4 §14-§16); per-CP 5 reviewers parallel
- Phase 8 user pick — implementation path (a) single PR / (b) sibling cascades / (c) phased B'+ surface; (c) NOT VIABLE without committed credential CP6 cascade slot

### 2026-04-24 T17:22 — Phase 2 complete + scope locked (commit 786f2429 for Phase 1, next for Phase 2)

**Co-decision protocol: 2 rounds required.**

**Round 1 (parallel):**
- architect (03a): 4 options — A'/B'/B'+/C'. Leans B'+ as draft position.
- tech-lead (03b): picks A' (lean A, fallback C, not B). Did NOT see B'+ (file not on disk when dispatched in parallel).
- security-lead (03c): ACCEPT all three (A'/B'/C') with must-have floor; no VETO.

**Round 2 (tech-lead re-rank):**
Orchestrator dispatched tech-lead to evaluate B'+ explicitly + reframed cascade as **design-scope-only** (per prompt non-goals: Tech Spec closes at design; implementation post-cascade).

Tech-lead round 2: **A' 1st / B'+ 2nd / C' 3rd / B' 4th**. B'+ acceptable fallback with condition (CredentialRef / SlotBinding / SchemeGuard MUST land in nebula-credential, not nebula-action). Design-scope reframe collapsed budget axis: all options fit cascade design budget; only C' triggers escalation rule 10 (spec revision).

**Orchestrator picked A' per tech-lead priority call.** Cascade proceeds to Phase 3 without escalation.

**Cross-crate coordination flags raised for future tracking:**
- Credential crate: CP6 phantom+HRTB+RAII core (`CredentialRef<C>`, `SlotBinding`, `SchemeGuard`, `SchemeFactory`, `RefreshDispatcher`) still spec-only. A' Tech Spec describes design for both crates.
- Post-cascade implementation path: USER DECISION (a) single PR / (b) sibling cascades / (c) phased rollout. Flagged in Phase 8 summary.

**Security must-have floor (non-negotiable):**
1. JSON depth cap at adapter boundaries
2. Explicit keyed credential dispatch (method signature, hard removal of heuristic)
3. ActionError Display sanitization
4. Cancellation-zeroize test

**Tech-lead solo-decided calls ratified** (Phase 1 + Phase 2): seal ControlAction + canonize DX tier; feature-gate + wire ActionResult::Terminate; HRTB *Handler modernization recommended.

### 2026-04-24 T16:48 — Phase 1 complete + gate passed (commit fc18c736 for Phase 0, next for Phase 1)

**4 parallel agents returned, convergent findings:**

- dx-tester: 7 🔴 + 12 🟠; time-to-first-compile Stateless 12min / Stateful 8min / ResourceAction+Credential 32min (target <5min). Credential attribute unusable in both string AND typed form.
- security-lead: 2 🔴 exploitable-today (S-C2 cross-plugin credential shadow attack via type-name-lowercase key; S-J1 JSON depth bomb confirms Phase 0 C4) + 17 🟠/🟡. Webhook crypto primitives solid. Retry feature flag NOT exploitable (disproven via engine grep).
- rust-senior: 1 🔴 (reconfirmed C2 with cargo expand evidence) + 2 🟠 DATED. Cancel safety + error taxonomy reference-quality. HRTB `*Handler` boilerplate inherited from `async-trait` convention; modernizable to single-`'a` + type alias + `trait_variant::make(Send)`.
- tech-lead: 3 solo priority calls (ratified) + 1 structural co-decision input. **CRITICAL REFRAME**: credential CP6 vocabulary has zero `src/` matches in credential crate itself — Option A = co-landing two cascades, not catching up.

**11 deduplicated 🔴 + 30+ 🟠 findings** → gate passes easily (threshold: 0 🔴 + <3 🟠 triggers escalation).

**Tech-lead solo-decided priority calls** (Phase 1 outputs):
1. Seal ControlAction + canonize DX tier in §3.5 as "erases to primary"
2. Feature-gate AND wire `ActionResult::Terminate` in cascade (apply Retry discipline)
3. Frame Option A/B/C as A'/B'/C' with cost re-estimates (A' exceeds 5-day budget; B' scoped but defers; C' unfreezes frozen spec)

**Phase 2 dispatch next:** architect + tech-lead + security-lead co-decision on A'/B'/C'. Orchestrator expects escalation probability raised.

### 2026-04-24 T16:29 — Phase 0 consolidation + gate passed

Orchestrator produced `01-current-state.md` consolidating both audits. Audits are **convergent** (no contradictions; devops' macro-harness gap mechanically explains rust-senior's unprotected attribute rejection paths).

**Gate decision:** ✅ PROCEED to Phase 1.

Critical findings (4 🔴):
- **C1**: Credential Tech Spec CP6 §§2.7/3.4/7.1/15.7 vocabulary entirely absent from action crate (CredentialRef<C>, phantom rewriting, SlotBinding, HRTB resolve_fn, SchemeGuard, SchemeFactory — none exist). Phase 2 scope decision required.
- **C2**: `#[derive(Action)]` broken `parameters = Type` emission path.
- **C3**: `credential<S>()` type-name-lowercase heuristic as key — collision footgun.
- **C4**: No serde_json recursion limit at adapter deserialization boundary.

Major structural (9 🟠 + coverage 🟠): canon §3.5 drift via ControlAction, v2 spec "5 traits no extras" violated, ActionResult::Terminate not gated, no macro test harness, no benchmarks, unstable-retry-scheduler dead flag, dead nebula-runtime reference in CI, zeroize inline pin, lefthook doctests/msrv/doc gap, SDK prelude contract surface, engine tight-coupling, missing layer-enforcement deny rule.

Phase 1 dispatching 4 agents in parallel: dx-tester (authoring 3 action types), security-lead (threat model), rust-senior (idiomatic review), tech-lead (architectural coherence). All 4 briefed with C1-C4 as load-bearing context.
