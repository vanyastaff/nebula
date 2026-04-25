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
| Phase 6 | **DEFERRED** | (Tech Spec — separate continuation session) | Cascade scope completed at design closure (Strategy + Spike + ADRs); Tech Spec drafting deferred per orchestrator context budget management. User decides continuation session. |
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
