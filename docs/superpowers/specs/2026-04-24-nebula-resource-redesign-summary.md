# nebula-resource Redesign Cascade — Summary

**Date:** 2026-04-24 (initial); 2026-04-25 (continuation completed)
**Status:** **CASCADE COMPLETE** — All 9 phases ratified (initial cascade 2026-04-24 covered Phases 0-3, 5, 7, 8; continuation 2026-04-25 completed Phase 4 spike + Phase 6 Tech Spec CP1-CP4)
**Branch:** `claude/vigilant-mahavira-629d10`
**Worktree:** `.worktrees/nebula/vigilant-mahavira-629d10`
**Orchestrator:** main session (flat coordination; hands-off cascade per user's paste-in-session prompt of 2026-04-24)

---

## Headline

The cascade established ground truth, enumerated 28 concrete pain findings (6 🔴 / 9 🟠 / 9 🟡 / 3 🟢 / 1 ✅), converged on Option B (targeted redesign) in a single co-decision round, produced a **frozen Strategy Document + 2 accepted ADRs + 35-row living concerns register**, validated the §3.6 trait shape via Phase 4 spike (PASSED iter-1), and elaborated the design to a **FROZEN 27,827-word Tech Spec across 16 sections + 4 ratified checkpoints**. **Implementation foundation now complete.** Implementation PR wave can begin per Tech Spec §16.1 (atomic single-PR plan).

## Continuation 2026-04-25 outcomes (Phase 4 + Phase 6)

| # | Phase | Status | Commit | Artefacts |
|---|-------|--------|--------|-----------|
| 4 | Spike (iter-1) | ✅ PASSED | `262665f8` | `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/spike/` (11 files including `NOTES.md` + 4 mock Resource impls + 6 integration tests) |
| 6 CP1 | Tech Spec §0-§3 (foundation) | ✅ RATIFIED | `1e416b91` | Tech Spec §0-§3; ADR-0036 + ADR-0037 flipped to `accepted` |
| 6 CP2 | Tech Spec §4-§8 (execution + storage) | ✅ RATIFIED | `e0f49536` | Tech Spec §4-§8; security amendments B-1/B-2/B-3 honored |
| 6 CP3 | Tech Spec §9-§13 (interface + migration) | ✅ RATIFIED | (final commit) | Tech Spec §9-§13; manager file split function-level cuts; adapter contract spec |
| 6 CP4 | Tech Spec §14-§16 (meta + handoff) | ✅ FROZEN | (final commit) | Tech Spec §14-§16; all 22 register tech-spec-material rows flipped to `decided`; PR wave plan locked atomic |

### Spike outcome (Phase 4)

- All 7 exit criteria met: cargo check/test/clippy clean; 6/6 integration tests pass; 3/3 compile-fail probes resolve as expected
- `<Self::Credential as Credential>::Scheme` ergonomics workable
- `type Credential = NoCredential;` opt-out clean
- Parallel rotation dispatch with per-resource isolation demonstrated under latency (3s sleep, 250ms budget) AND error (one resource Err, siblings Ok)
- Reverse-index write path implemented (resolves manager.rs:262/370 Phase 1 finding 🔴-1)
- 5 open questions resolved in Tech Spec CP1 §2.5

### Tech Spec ratification track (Phase 6)

- **CP1:** spec-auditor PASS_WITH_MINOR + rust-senior RATIFY_WITH_EDITS + tech-lead RATIFY_WITH_EDITS — 6 bounded edits applied including ADR-0037 amended-in-place gate text
- **CP2:** tech-lead RATIFY_WITH_EDITS + security-lead ENDORSE_WITH_AMENDMENTS — security B-1/B-2/B-3 verbatim honored; 6 edits applied
- **CP3:** tech-lead RATIFY_WITH_EDITS + dx-tester ENDORSE_WITH_AMENDMENTS — 6 edits applied (3 spec-hygiene + 3 §11 adapter walkthrough fixes)
- **CP4:** tech-lead RATIFY → FROZEN. **First CP this cascade requiring zero edits.**

### Locked design decisions (canonical)

- **`Resource::Credential` adoption** per credential Tech Spec §3.6 verbatim (ADR-0036)
- **`NoCredential`** lives in `nebula-credential`, re-exported by `nebula-resource`
- **`TypeId`-based opt-out** detection (over sealed-trait)
- **`ManagerConfig::credential_rotation_concurrency = 32`** soft cap default
- **30s rotation timeout** (Manager default) + per-`RegisterOptions` override
- **Revocation default-hook = option (b):** Manager unconditionally flips `credential_revoked` atomic post-dispatch
- **Blue-green pool swap = `Arc<RwLock<Pool>>`** (async-write exclusion across `.await`)
- **`warmup_pool` two-method split:** credential-bearing + `warmup_pool_no_credential` (B-3 type-level guard)
- **Manager file-split: 7 submodules** (mod / options / registration / dispatch / rotation / shutdown / gate)
- **Daemon + EventSource → engine-fold** at `crates/engine/src/daemon/` (ADR-0037)
- **`register_*` dual-helper API: 5 topologies × 2 = 10 helpers + 1 type-erased = 11 total methods**
- **`AcquireOptions::intent/.tags`: `#[deprecated]`** (option b per Strategy §5.2)
- **PR wave: atomic single-PR** (Strategy §4.8 + ADR-0036 + security atomicity invariant)
- **MATURITY transition: frontier → core** post-soak (Strategy §6.4)

---

## Phases completed

| # | Phase | Status | Gate | Artefacts |
|---|-------|--------|------|-----------|
| 0 | Documentation reconciliation | ✅ PASSED | Audits consistent — no architect mediation needed | `01-current-state.md` (reconstructed after filesystem event; per-agent audits lost) |
| 1 | Pain enumeration | ✅ PASSED (easily) | 6 🔴 / 9 🟠 far exceed "nothing to redesign" escalation threshold | `02-pain-enumeration.md` (7920 words, canonical) |
| 2 | Scope narrowing co-decision | ✅ PASSED (round 1, unanimous) | Option B + 5 merged amendments | `03-scope-options.md` + `03-scope-decision.md` + `phase-2-tech-lead-review.md` + `phase-2-security-lead-review.md` |
| 3 | Strategy Document | ✅ FROZEN (CP3) | 3 checkpoints, each ratified by architect + spec-auditor + tech-lead | `docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md` (7920 w / 453 L / 6 sections) |
| 4 | Spike | ⏸ DEFERRED | Per Strategy §5.5 trigger — spike runs at implementation PR draft time | Not dispatched this cascade |
| 5 | ADR drafting | ✅ COMPLETE (primary ADR) | ADR-0036 accepted-pending-Tech-Spec-CP1-ratification | `docs/adr/0036-resource-credential-adoption-auth-retirement.md` (2203 w / 201 L) |
| 6 | Tech Spec | ⏸ DEFERRED | Single session cannot execute 4-5 CPs × multi-reviewer cascade; Strategy + ADR + Register give sufficient implementation runway | Not dispatched this cascade |
| 7 | Register + consensus | ✅ COMPLETE (register only; no consensus session) | Phase 1 convergence was unanimous — no 3-stakeholder consensus session needed | `docs/tracking/nebula-resource-concerns-register.md` (35 rows) |
| 8 | Final consolidated summary | ✅ COMPLETE | This file | `docs/superpowers/specs/2026-04-24-nebula-resource-redesign-summary.md` |

---

## Commits landed

| Commit | Message | Phase |
|---|---|---|
| `12d0d79a` | `docs(resource): cascade Phases 0-1 — ground truth + pain enumeration` | 0, 1 |
| `d903e470` | `docs(resource): cascade Phase 2 — scope LOCKED on Option B + amendments` | 2 |
| `5cf00a24` | `docs(resource): cascade Phase 3 — Strategy FROZEN at CP3` | 3 |
| *(this commit)* | `docs(resource): cascade Phases 5/7/8 — ADR + register + summary` | 5, 7, 8 |

---

## Register state

From `docs/tracking/nebula-resource-concerns-register.md`:

| Label | Count |
|---|---|
| strategy-blocking | **0** ← no residual scope blockers |
| tech-spec-material | **22** ← Phase 6 Tech Spec owns these |
| sub-spec | 0 |
| standalone-fix | **1** (R-040 SF-1 `deny.toml` wrappers rule) |
| post-cascade | 5 |
| future-cascade | 4 |
| invariant-preservation | 3 |
| **Total** | **35** |

All 6 Phase 1 🔴 findings are addressed in Strategy §4 + ADR-0036 + register. All 9 🟠 findings are either in-scope (5), standalone-fix (1), post-cascade (2), or future-cascade (1).

---

## Escalations raised during cascade

### Soft escalation — filesystem loss of per-agent findings files

**2026-04-24 T+~45min** — after Phase 1 consolidation, Edit tool reported "File does not exist" for `01-current-state.md` and `CASCADE_LOG.md`. Inspection showed only `02-pain-enumeration.md` survived the Phase 1 agent dispatches. Per-agent findings files (`phase-0-code-audit.md`, `phase-0-manifest-audit.md`, `phase-1-{dx-tester,security-lead,rust-senior,tech-lead}-findings.md`) were also lost.

**Hypothesis:** Agent subagents ran in isolated worktrees per `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` teammate mode; when those worktrees were cleaned up, the untracked files they created may have been swept. The exact mechanism remains unclear; main-session writes were affected too.

**Recovery:** `02-pain-enumeration.md` preserved the canonical consolidated findings; `01-current-state.md` and `CASCADE_LOG.md` were reconstructed from orchestrator context. No material cascade progress lost — only raw agent-level evidence.

**Mitigation applied:** commit after every phase gate (Phases 2, 3 committed immediately on completion). This worked — Phase 2 and Phase 3 artefacts persisted.

**Post-cascade:** worth investigating the cleanup mechanism. If it's systemic, future cascades should commit per-CP rather than per-phase.

### No hard escalations

No `ESCALATION.md` was written. The cascade did not hit:
- Phase 0 reconciliation contradiction (audits agreed)
- Phase 1 "nothing to redesign" threshold (6 🔴 / 9 🟠 vastly exceeded)
- Phase 2 co-decision deadlock (round 1 unanimous)
- Budget hit (~130 min agent-effort; 5-day envelope untouched)
- Production data requirement
- External stakeholder input requirement
- Breaking change exceeds migration budget (5 in-tree consumers, MATURITY = frontier)

---

## Two distinct sets of open items (do not confuse)

The cascade left two different question-sets with different lifecycles. Reader should not treat them as overlapping:

| Set | Where | Lifecycle | Examples |
|---|---|---|---|
| **Strategy §5.1-§5.5** | `2026-04-24-nebula-resource-redesign-strategy.md` §5 | **Design questions** that surface during implementation; Phase 6 Tech Spec / Phase 4 spike resolves them | NoCredential convenience symmetry, AcquireOptions interim treatment, Runtime/Lease collapse trigger |
| **Summary Q1-Q5** | This document below | **Process questions** about cascade pacing + scope (when to dispatch what) | Spike timing, Tech Spec scope, ADR-0037 dispatch, SF-1 PR, MATURITY transition awareness |

The two sets do **not** overlap. Strategy §5 records design ambiguity; Summary Q records process choice.

---

## Open questions awaiting user decision

### Q1 — Phase 4 spike: dispatch now (separate session), or roll into implementation PR drafting?

**Context:** Strategy §5.5 triggers a spike before implementation. The spike validates §3.6 trait shape ergonomics + perf against current pool-acquire. Per Phase 3 CP1 E1 amendment, spike failure escalates to Phase 2 round 2 — no mid-flight shape change.

**Options:**
- **(a)** Dispatch Phase 4 spike in a separate focused session (2 iterations, isolated worktree). Exit criteria clear. ~30-60 min wall.
- **(b)** Roll spike into implementation PR preamble — first PR in wave is effectively the spike; if it reveals §3.6 ergonomics issues, escalate then.

**Orchestrator recommendation:** (b). Spike artefacts (`NOTES.md`, `final_shape_v2.rs`) are more useful embedded in a PR's commit history than stashed in a worktree. Strategy is frozen; spike is validation not design.

### Q2 — Phase 6 Tech Spec: when + how?

**Context:** Tech Spec is the implementation-bridge artefact with full trait signatures, file-split cuts, consumer-migration enumeration. Strategy §4 has the shape; Tech Spec §3-§13 would have the exact code.

**Options:**
- **(a)** Dispatch a dedicated multi-session Tech Spec cascade (4-5 CPs × multi-reviewer). Budget: ~10-15 hours agent-effort.
- **(b)** Skip Tech Spec entirely — implementation PR wave uses Strategy §4 + ADR-0036 + Register as the design input. Phase 6 Tech Spec is template for Nebula but may be overkill given engine fold + credential Tech Spec §3.6 already exists as the concrete signature source.
- **(c)** Dispatch a minimal Tech Spec CP1 only (§0-§3 scope + contract + runtime model) to anchor the implementation PR wave; defer CP2-CP4 to post-merge.

**Initial orchestrator recommendation (REVISED 2026-04-25):** previously (c). User push-back with concrete counter-evidence from credential cascade overrides:

> Credential CP4→CP5 surfaced 8 compile-time amendments + 4 runtime checks via 3-agent consensus session. Strategy was frozen at CP3 — what caught those was Tech Spec CP4→CP5 reviews when Phase 1 sec-lead findings (N1-N10) were re-elaborated against actual Tech Spec sections. Without Tech Spec, sub-trait split (capability silent-downgrade fix) and sensitivity dichotomy would not have surfaced. ADR-0036 currently has gaps Tech Spec would close: conceptual `<Self::Credential as Credential>::Scheme` lacks ergonomic alternatives evaluation; default `on_credential_revoke` returning `Ok(())` lacks mechanism specification; dispatcher concurrency cap "~32" mentioned in tech-lead Phase 2 review not confirmed in ADR; 5-consumer migration not concretized to function-level diffs.

**REVISED recommendation:** (a) **full multi-CP Tech Spec cascade in a follow-up session.** "Strategy §4 dense enough" justification rejected — that logic skipped credential cascade CP4→CP5 amendments and would have shipped a silent-downgrade vulnerability there.

**Fallback if budget tight:** (c) minimal CP1 as preamble to first implementation PR — first PR commits CP1-only Tech Spec alongside code; remaining sections grow as implementation PRs land. Less rigorous than full multi-session cascade but cheaper.

**Avoid (b)** — skipping Tech Spec entirely is the same anti-pattern as the credential cascade would have hit if it had stopped at Strategy.

### Q3 — ADR-0037 for Daemon/EventSource engine-fold?

**Context:** Strategy §4.4 picked engine-fold for Daemon + EventSource extraction. Architect flagged this as deserving its own ADR (separate from ADR-0036 which covers `Resource::Credential` adoption).

**Options:**
- **(a)** Draft ADR-0037 before implementation. ~1-2 hours agent-effort.
- **(b)** Skip — Strategy §4.4 is detailed enough; implementation PR commit message references Strategy §4.4 directly.

**Orchestrator recommendation:** (a) if time allows, (b) otherwise. ADR-0037 captures the §5.1 revisit trigger condition (Daemon code >500 LOC or non-trigger workers >2) — useful historical artefact, not blocking.

### Q4 — SF-1 (deny.toml wrappers rule): dispatch now or at implementation?

**Context:** Security-lead Phase 2 flagged this as standalone-fix PR. Mechanical change. Devops agent can handle in minutes.

**Options:**
- **(a)** Dispatch devops agent now for SF-1 PR.
- **(b)** Wait for implementation PR wave start.

**Orchestrator recommendation:** (a). SF-1 is truly independent, low-risk, and tightens layer containment today regardless of redesign pace.

### Q5 — MATURITY.md transition timing

**Context:** Strategy §6.4 describes `frontier` → `core` transition post-merge. Transition criteria: no 🔴 in rotation counter over soak period, register shows no unresolved concerns.

**Options:** None — this is automatic per Strategy §6.4. Just confirm user is aware the MATURITY row should update at cascade completion.

---

## Recommended next steps (ordered, REVISED 2026-04-25 per user direction)

**LANDED 2026-04-25:**
- ✅ SF-1 `deny.toml` wrappers rule — commit `bb66537a` `fix(deny): add nebula-resource layer-enforcement wrapper rule` — devops verified 5 consumers (action, engine, plugin, sandbox, sdk) and `cargo deny check bans` passes.
- ✅ ADR-0037 Daemon/EventSource engine-fold drafted (1628w, status: Proposed pending Tech Spec CP1) — `docs/adr/0037-daemon-eventsource-engine-fold.md`. ADR-0036 frontmatter backlink added.

**REMAINING decisions (per user direction 2026-04-25):**
1. **Q2 Tech Spec — full Phase 6 cascade (option a) recommended** (not minimal CP1). User push-back vs orchestrator's initial recommendation accepted — see Q2 above for credential cascade evidence. Dispatch in dedicated follow-up session; budget ~10-15 hours agent-effort.
2. **Q1 Phase 4 spike — roll into first implementation PR (option b)** — confirmed. First PR is effectively the spike; spike artefacts embed in PR commit history. Note: spike-as-first-PR has full security/rust-senior review on real code, higher signal but slower iteration than throwaway worktree spike.
3. **Q5 MATURITY transition — automatic** per Strategy §6.4 trigger condition; tracked in register. No action.

**Implementation start:**
4. **(when ready)** Open multi-session work stream for trait-reshape PR wave. Strategy §4 as design input, ADR-0036 + ADR-0037 as decision records, register as scope tracker. Consumers migrate atomically per Strategy §4.8. Observability gate per Strategy §4.9.
5. **(post-merge)** Soak period per Strategy §6.3 (1-2 weeks). Watch `nebula_resource.credential_rotation_attempts` counter + `ResourceEvent::CredentialRefreshed` event stream.
6. **(post-soak)** MATURITY.md `frontier` → `core` transition per Strategy §6.4. Register closes. Cascade formally complete.

---

## Artefact index (complete)

### Canonical outputs (committed)

| Path | Role |
|---|---|
| `docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md` | **Strategy — FROZEN CP3** (7920 w / 453 L / 6 sections) |
| `docs/superpowers/specs/2026-04-24-nebula-resource-redesign-summary.md` | **This summary** |
| `docs/adr/0036-resource-credential-adoption-auth-retirement.md` | Primary ADR (2203 w / 201 L) |
| `docs/tracking/nebula-resource-concerns-register.md` | Living concerns register (35 rows) |
| `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/CASCADE_LOG.md` | Append-only cascade log |
| `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/01-current-state.md` | Phase 0 ground truth (reconstructed) |
| `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md` | **Phase 1 canonical findings** (6 🔴 / 9 🟠) |
| `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-options.md` | Phase 2 architect 3-option draft |
| `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md` | **Phase 2 LOCKED scope** |
| `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-2-tech-lead-review.md` | Phase 2 priority-call |
| `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md` | Phase 2 security-gate |
| `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-3-cp1-spec-auditor-review.md` | CP1 audit |
| `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-3-cp1-tech-lead-ratification.md` | CP1 ratification |
| `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-3-cp2-spec-auditor-review.md` | CP2 audit |
| `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-3-cp2-tech-lead-ratification.md` | CP2 ratification |

### Lost to filesystem event (recovery: 02-pain-enumeration.md preserves consolidated content)

- `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-0-code-audit.md` (rust-senior Phase 0)
- `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-0-manifest-audit.md` (devops Phase 0)
- `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-1-dx-tester-findings.md`
- `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-1-security-lead-findings.md`
- `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-1-rust-senior-findings.md`
- `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-1-tech-lead-findings.md`
- `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/scratch/probe-*.md` (dx-tester probes)

---

## Budget accounting

| Phase | Agent-effort |
|---|---|
| Phase 0 (2 parallel agents) | ~13 min |
| Phase 1 (4 parallel agents) | ~40 min |
| Phase 2 (architect + 2 reviewers) | ~10 min |
| Phase 3 (3 architect + 4 reviewer dispatches across 3 CPs) | ~50 min |
| Phase 5 (1 architect dispatch, ADR-0036) | ~5 min |
| Phase 7 (orchestrator direct, no dispatch) | 0 agent-effort |
| Phase 8 (orchestrator direct, no dispatch) | 0 agent-effort |
| **Total agent-effort** | **~118 min** |
| Wall-time (incl. orchestrator consolidation) | ~140 min |

Envelope was 5 days (7200 min). **Budget used: ~2% of envelope.** Phase 4 spike + Phase 6 Tech Spec would double-to-quadruple this — well within remaining envelope if user dispatches a follow-up session.

---

## Closing

The cascade established that **nebula-resource needs a targeted redesign driven by the credential×resource seam**, that **Option B is the right scope**, and that **§3.6 trait reshape + Daemon/EventSource engine-fold + Manager file-split + atomic 5-consumer migration** is the implementation path. The frozen Strategy, ADR-0036, and living register provide the design foundation. **User returns to a state where implementation can begin** — not to an incomplete design exercise.

Two deferrals (Phase 4 spike, Phase 6 Tech Spec) are clearly scoped with orchestrator recommendations for how to handle each. Neither blocks implementation start.

**The cascade did not proceed to "paper-design output" territory** — every decision traces to Phase 1 evidence, every deferral has a trigger condition, every 🔴 is either addressed in Strategy or has an explicit pointer in the register.
