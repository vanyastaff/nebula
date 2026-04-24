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
| Phase 1 | in-progress | `02-pain-enumeration.md` | Dispatching 4-agent parallel review (dx-tester + security-lead + rust-senior + tech-lead) |
| Phase 2 | blocked | `03-scope-decision.md` | Blocked by Phase 1 |
| Phase 3 | blocked | Strategy CP3 | Blocked by Phase 2 |
| Phase 4 | blocked | Spike NOTES.md | Conditional on Phase 3 scope |
| Phase 5 | blocked | ADR(s) | Blocked by Phase 3 |
| Phase 6 | blocked | Tech Spec CP4 | Blocked by Phases 4+5 |
| Phase 7 | blocked | Concerns register | Conditional on Phase 1 severity |
| Phase 8 | blocked | Summary file | Blocked by Phase 6 |

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
