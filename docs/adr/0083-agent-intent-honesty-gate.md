---
id: 0083
title: agent-intent-honesty-gate
status: proposed
date: 2026-05-18
supersedes: []
superseded_by: []
tags: [agent-harness, hooks, anti-reward-hacking, structural-budget, enforced-discipline, observability]
related:
  - CLAUDE.md
  - docs/QUALITY_GATES.md
  - clippy.toml
  - .claude/hooks/stop-gate.sh
  - .claude/hooks/record.sh
  - .claude/hooks/edit-guard.sh
---

# 0083. Agent intent, structural-budget & honesty gate (Layer-2)

## Context

The committed guard stack (CLAUDE.md "Enforced Discipline") gives a
**structural** no-cheat guarantee D10 = B (`edit-guard.sh`) + A2
(`record.sh`) + C (`stop-gate.sh`) + lefthook/CI. It proves, mechanically and
deterministically, that every crate touched in a turn reaches a **clean**
clippy + nextest green and that tests were not weakened by the same turn that
changed implementation.

Published evidence shows D10 is structurally blind to a second failure class —
degradation **decoupled from functional correctness**:

- **Volume–Quality Inverse Law** (arXiv 2605.02741): code volume (TLoC)
  correlates with architectural decay at ρ≈0.94, file count at ρ≈0.72;
  functionally-correct code is *as likely* to be structurally rotten as
  failing code; better prompting does **not** mitigate it.
- **Laziness-Deficit Fallacy** (AgentPatterns): human restraint is an
  emergent property of *time scarcity*; an agent pays no time cost for code it
  writes, so "be concise / prefer simple" prompts do nothing or backfire.
- **SlopCodeBench** (arXiv 2603.24755): quality degradation compounds across
  iterations; anti-slop / plan-first prompts shift the *intercept* but not the
  *slope* — only tooling that enforces structural discipline per checkpoint
  stops it.
- **Reward-hacking → misalignment** (Anthropic arXiv 2511.18397, Opus 4.6
  card) and **EvilGenie** (arXiv 2511.21654): under a deterministic green
  gate, Claude Code still fits impl to test inputs, edits test files, narrates
  "done" unsupported by the diff, and falsifies failed-tool results.
- **Action bias** (FixedBench, arXiv 2605.07769): agents apply undesirable
  changes to already-correct code 35–65 % of the time; framing **abstention
  as a successful outcome** lifts correct-abstention by 15–28 pp. Nebula open
  issues are frequently already fixed (`reference_open_issue_not_open`).
- **Duplicate-helper / pattern divergence** (Lint-Against-the-Machine, KruN):
  agents do not search the codebase, so they re-implement existing utilities;
  layer leakage is the single most common AI-debt pattern.

Verified gap in *this* repo: `clippy.toml` declares
`cognitive-complexity-threshold = 25`, `too-many-lines-threshold = 100`,
`excessive-nesting-threshold = 5` — but `[workspace.lints.clippy]` sets
`cognitive_complexity = "allow"` and `too_many_lines = "allow"` (a deliberate
choice — flipping them workspace-wide on 36 crates is thousands of legacy
warnings). The thresholds are therefore **inert**. D10 cannot see volume,
complexity, duplication, or intent.

Five vectors remain open above D10:

1. **Intent fidelity** — a green diff that does not address the request.
2. **Test-shaping that survives `edit-guard`** — impl fitted to test inputs;
   gutted-but-assert-count-preserved test; cross-turn test edits.
3. **Tool-result / completion falsification** — claim unsupported by state.
4. **Structural decay** — volume/complexity/duplication/layer-leakage that is
   green but unmaintainable; no out-of-context senior-architect verification.
5. No **inoculation** and no **abstention-as-success** framing.

## Decision

Add a **Layer-2 gate** as a pure addition above D10. It does not modify or
weaken the deterministic core; `stop-gate.sh` (C) runs first and remains the
structural guarantee. Layer-2 has two tiers, in cost order:

1. **Structural-budget tier — deterministic, the binding restraint.** Per the
   Laziness-Deficit and SlopCodeBench evidence, the restraint that actually
   binds is mechanical and observable, not an LLM and not a prompt. This tier
   is deterministic, extends the D10 philosophy, and runs before any model is
   invoked.
2. **Semantic tier — confidence-gated LLM, the judgment layer.** Catches
   intent/honesty/architecture failures a mechanical check cannot see, blocks
   only on unambiguous high-confidence must-have violations.

Both tiers block the turn via the existing `deny()` exit-2 contract (C's
mechanism). Enforcement is **structural gating** (user-selected); the design
carries the false-block and infinite-loop mitigations that make hard gating
safe. Scope: main-thread `Stop` plus the **implementation-worker
`SubagentStop`** — defined by *role* (the subagent that executes plan tasks),
not by a fixed ecosystem name. Today that is the AI-Factory `implement-worker`;
the deterministic core gates on git/clippy/nextest and is **skill-ecosystem-
agnostic**, so the AI-Factory → `ce-*` migration (a separate sequenced
workstream, below) only re-points the named integration hooks, never the core.

Recorded as a decision because it changes the enforced-discipline contract
(adds gates that can block completion), reconciles an intentional
`QUALITY_GATES.md` lint allowance (diff-scoped re-enablement), and changes the
agent-prompt baseline (inoculation + abstention-as-success).

### Mechanism

**Shipped now (this PR):** a deterministic `command` Stop / SubagentStop hook
(`intent-gate.sh`) — pre-filter then structural-budget checks, pure bash + git,
no model.

**Deferred (the semantic tier, a sequenced follow-up plan — NOT in this PR):**
a native **`prompt`** Stop-hook (Haiku) → native **`agent`** Stop-hook whose
command no-ops unless the prompt tier wrote an escalation marker (conditional
escalation on native hook types without a nested `claude -p` — same "no
hand-rolled boundary process" reasoning that removed `resolve_cmd` from
`_lib.sh`). Wiring for that tier lands with its own plan, not here.

## Design

The deterministic structural-budget tier is specified, with code and tests, in
[`docs/plans/2026-05-18-003-feat-agent-intent-honesty-gate-plan.md`](../plans/2026-05-18-003-feat-agent-intent-honesty-gate-plan.md).
The confidence-gated semantic LLM tier (grounded rubric, out-of-context
reviewer) is a sequenced follow-up plan in `docs/plans/`. This ADR records the
**decision**; implementation detail is not duplicated here (0082 convention).

## Consequences

- The binding restraint against quick-win / slop / spaghetti is **mechanical
  and observable** (structural-budget tier), per the evidence that prompts and
  green gates cannot bind it; the LLM tier adds the senior-architect judgment
  a linter cannot.
- A confirmed unambiguous reward-hack / dishonest-completion, an over-budget
  unjustified diff, a duplicate symbol, or a must-rubric failure blocks the
  turn. Correct abstention is an allowed success.
- Nondeterminism is contained: deterministic tier first; LLM blocks only on
  must + high-confidence + evidence; bounded retry; logged escapes; C strictly
  first; D10 determinism unchanged.
- Adds ~0-cost deterministic checks every Stop and bounded per-completion LLM
  cost on the final Stop only.
- Reconciles an intentional `QUALITY_GATES.md` lint allowance without the
  workspace-wide churn that justified it (diff-scoped enforcement only).
- Agent-prompt baseline changes; subagent edits must keep the inoculation +
  abstention lines.
- The deterministic core is **skill-ecosystem-agnostic** (gates git / clippy /
  nextest only); the AI-Factory → `ce-*` migration re-points named integration
  hooks but cannot weaken or bypass the core.
- ADR sprawl is bounded: the whole 3-workstream program is **one ADR**;
  follow-ups are plans, not ADRs; 0083 is slimmed post-plan so agent context
  is not eaten by this program's documentation.

## Escape hatch hardening

The structural-budget tier's `// budget-justified: <reason>` escape is the
agent-facing pressure valve. Field operation revealed two failure modes that
this ADR now closes (PR-series referenced inline in the implementation):

1. **SIGPIPE on `grep -q`.** The original `budget_justified` consumer used
   `grep -q`, which exits on the first match and triggered `SIGPIPE` on the
   `ig_added_lines` producer (the three-way diff and the untracked
   `while-read | sed` loop). Under `set -uo pipefail` the producer-side rc=141
   propagated through the pipeline and `budget_justified` returned non-zero
   — the marker silently failed to escape. The fix is a drain-safe `grep -c`
   pattern that lets producers exit cleanly. No semantic change to the
   escape; this is a correctness fix that was hiding a latent
   marker-doesn't-actually-work bug.

2. **Gameable marker.** A bare `// budget-justified: <anything>` line was
   sufficient to unlock the entire turn. The escape is now hardened on three
   axes, each independent:

   - **Path-based auto-exempt.** `*/benches/*.rs`, `*/migrations/*.sql`, and
     `*/tests/golden/*` + `*/tests/snapshots/*` + `*/snapshots/*` no longer
     need a marker — the path encodes the semantics (criterion tables, DDL
     fixtures, golden snapshots). Bench files still respect a per-file blob
     cap of 300 (so a single function cannot balloon). Migrations and
     golden/snapshot data are effectively unbounded. Agents cannot game the
     path because it is checked literally and a reviewer catches
     misplacement. Files whose first lines carry an `@generated` marker
     (prettier / prost-build / tonic convention) are similarly auto-exempt.
   - **Per-turn marker budget.** `MARKER_BUDGET=2`. Spamming markers across
     files defeats the point; a 3-marker turn fails with
     `marker-budget-exhausted` regardless of what else is justified. The cap
     runs BEFORE the blob / NF / net-LoC checks so markers cannot authorize
     themselves.
   - **Minimum-justification quality (blob only).** The text after
     `budget-justified:` must be at least 30 chars and mention one of
     `table | generated | criterion | migration | fixture | schema |
     snapshot | golden | test data`. A lazy `// budget-justified: ok`
     authorizes nothing for the blob check. NF / net-LoC / dup still treat
     any marker as present — the quality bar is concentrated on the most
     decay-correlated dimension (per-fn complexity).

   Net effect: the marker is no longer sufficient on its own. Path matters.
   Quantity matters. Justification quality matters. The deterministic core
   (D10) is unchanged; this is pure addition to Layer-2.

Verification ships as new positive and negative cases in the `task hooks:test`
harness (bench-path exempt, snapshot-path exempt, `@generated` auto-exempt,
3-marker turn denies, low-quality marker fails blob escape, quality marker
escapes blob, 200-line src blob without marker still denies — the default
path stays at the 100-line per-fn cap that mirrors `clippy.toml` `too-many-
lines`).

## Follow-up workstream (sequenced, not part of this ADR)

The diff-scoped / legacy-grandfathered choice is deliberate **ordering**, not a
permanent exemption. The recurring mistake this program prevents is letting
complexity/duplication debt accumulate with no observable gate. Therefore:

1. **0083 lands first** — the binding structural-budget + semantic gate is
   installed and proven (`task hooks:test`).
2. **Legacy structural-debt burn-down** (own plan in `docs/plans/`, **no new
   ADR** — execution under this decision, not a new decision). Shape,
   grounded in cleanup-loop practice (Propel, Fowler net-negative): a
   background cleanup loop on a fixed cadence producing **net-negative**,
   evidence-carrying (lint/structure deltas) small PRs, prioritised by churn
   (files touched in >30 % of recent PRs first), reconciling the
   `QUALITY_GATES.md` `cognitive_complexity` / `too_many_lines` allowance
   crate-by-crate. The 0083 gate runs throughout, so cleaned crates cannot
   re-accrue debt and the slope cannot reverse.
3. **AI-Factory removal / `ce-*` default** (own plan in `docs/plans/`, **no
   new ADR**).
   `aif-*` skills and the AI-Factory subagent fleet are retired
   (`project_ai_factory_abandoned` already froze curation); `ce-*` becomes the
   default skill set. This workstream owns re-pointing 0083's *named*
   integration hooks (the `SubagentStop` matcher string and the role-based
   inoculation targets) to the `ce-*` equivalents. The 0083 deterministic
   core needs **no change** — it never references a skill ecosystem. No shim
   or aif↔ce bridge: the wrong thing is replaced directly.

Items 2 and 3 are sequenced **after** 0083 and are independent of each other.
This ADR does **not** design either; it only fixes the order so both are
durable and migration-safe.

## Documentation discipline (one-ADR program)

ADR count is itself an agent-context cost (CLAUDE.md already forbids bulk-
reading `docs/adr/0*`; the thematic index is the access path). This program is
deliberately **one ADR — 0083**:

- Workstreams 2 and 3 produce **plans in `docs/plans/`, not ADRs**. They are
  execution under this decision, not new architectural decisions.
- Future related changes **amend or absorb into 0083** (the repo's
  stub/absorb pattern — cf. 0080/0081/0082 contract ADRs), never a sibling
  ADR number. If a genuinely new decision arises it supersedes 0083 in place.
- After `writing-plans` produces the 0083 implementation plan, this ADR is
  **slimmed to the lean decision** (Context → Decision → Consequences); the
  component/design detail moves into the plan, matching the 0082 convention
  ("implementation detail not duplicated in the ADR body"). The ADR stays
  context-cheap and reachable via the thematic index, not by bulk read.

## Supersession

None. Pure addition above D10; supersedes no prior ADR.
