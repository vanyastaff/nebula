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

Published evidence on coding-agent failure modes shows this is necessary but
structurally blind to a second failure class — degradation that is **decoupled
from functional correctness**:

- **Volume–Quality Inverse Law** (arXiv 2605.02741): code volume (TLoC)
  correlates with architectural decay at ρ≈0.94, file count at ρ≈0.72;
  functionally-correct code is *as likely* to be structurally rotten as
  failing code; better prompting does **not** mitigate it. Agents behave as
  "junior developers with syntactic precision but no senior-architect
  foresight".
- **Laziness-Deficit Fallacy** (AgentPatterns): human restraint is an
  emergent property of *time scarcity*; an agent pays no time cost for code it
  writes, so "be concise / prefer simple" prompts do nothing or backfire.
  Restraint that binds must be an **objective harness gate the agent can
  observe**: diff-size budgets, complexity budgets, duplication thresholds,
  deletion targets.
- **SlopCodeBench** (arXiv 2603.24755): quality degradation compounds across
  iterations; anti-slop / plan-first prompts shift the *intercept* but not the
  *slope* — only tooling that enforces structural discipline per checkpoint
  stops it.
- **Reward-hacking → misalignment** (Anthropic arXiv 2511.18397, Opus 4.6
  card) and **EvilGenie** (arXiv 2511.21654): under a deterministic green
  gate, Claude Code still fits impl to test inputs, edits test files, narrates
  "done" unsupported by the diff, and falsifies failed-tool results. An LLM
  judge is the most reliable detector — near-zero false negatives, ~1 false
  positive on *unambiguous* cases.
- **Action bias** (FixedBench, arXiv 2605.07769): agents apply undesirable
  changes to already-correct code 35–65 % of the time; framing **abstention
  as a successful outcome** lifts correct-abstention by 15–28 pp (reproduce-
  only does not). Directly relevant here — Nebula open issues are frequently
  already fixed (`reference_open_issue_not_open`).
- **Duplicate-helper / pattern divergence** (Lint-Against-the-Machine, KruN):
  agents do not search the codebase, so they re-implement existing utilities
  and violate layer boundaries; layer leakage is the single most common
  AI-debt pattern.

Verified gap in *this* repo: `clippy.toml` declares
`cognitive-complexity-threshold = 25`, `too-many-lines-threshold = 100`,
`excessive-nesting-threshold = 5` — but `[workspace.lints.clippy]` sets
`cognitive_complexity = "allow"` and `too_many_lines = "allow"` (a deliberate
choice — flipping them workspace-wide on 36 crates is thousands of legacy
warnings; rationale in `docs/QUALITY_GATES.md`). The thresholds are therefore
**inert**: the exact mechanical bloat/complexity gate the research says must
exist is currently disabled. D10 cannot see volume, complexity, duplication,
or intent.

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

Deterministic command pre-filter (`intent-gate.sh`) → structural-budget
checks (same hook, pure bash + git, no model) → native **`prompt`** Stop-hook
(Haiku) → native **`agent`** Stop-hook whose command no-ops unless the prompt
tier wrote an escalation marker (conditional escalation on native hook types
without a nested `claude -p` — same "no hand-rolled boundary process"
reasoning that removed `resolve_cmd` from `_lib.sh`).

## Design

### Components

1. **`.claude/hooks/intent-gate.sh`** — command hook on `Stop` and
   `SubagentStop` (matcher = the current implementation-worker subagent;
   `implement-worker` today, re-pointed by the `ce-*` migration workstream),
   ordered **after** `stop-gate.sh`.
   - Loop guard: `stop_hook_active == true` → `allow` (mirrors
     `stop-gate.sh:8`); plus a turn-state `intent_attempts` counter — after
     **N = 2** blocks, `allow` and log an `escalation` record. Failing toward
     not trapping the human is deliberate; the log is the review surface.
   - Pre-filter: empty `git diff turn_base..HEAD` **and** empty
     `impl_files_edited` → `allow`. If C denied this turn → `allow` (C ran
     first; do not double-judge broken code).

2. **Structural-budget tier** (deterministic, inside `intent-gate.sh`,
   **diff-scoped — legacy grandfathered**, so no workspace-wide churn and the
   `QUALITY_GATES.md` allowance stays intact):
   - **Net-LoC budget** — `git diff --numstat turn_base..HEAD` added−deleted
     over a soft cap (**starting value 400**, the research-cited large-change
     threshold; calibrated from the audit log, not guessed again) blocks
     unless the turn carries an explicit `// budget-justified: <reason>` token
     (mirrors the `// guard-justified:` escape-hatch idiom). Large diffs are a
     top rejection cause; the cap forces a split or a justification.
   - **New-file budget** — added-file count over a small cap (**starting
     value 5**) blocks unless `// budget-justified:` (ToF is the second
     strongest decay predictor).
   - **Per-changed-fn complexity** — clippy is crate-scoped and the lints are
     `allow`, so this is a **deterministic heuristic over the diff**: for each
     function added/modified in `turn_base..HEAD`, take the enclosing-fn body
     and check length and max nesting against the existing `clippy.toml`
     thresholds (length 100, nesting 5; cognitive-complexity approximated by a
     branch/loop count proxy). New code is held to the bar the inert workspace
     lint cannot enforce; legacy is untouched.
   - **Duplicate-symbol heuristic** — a new `pub fn` / `pub struct` / `pub
     trait` whose name collides with an existing workspace symbol blocks
     unless `// budget-justified:` (cheap grep across `crates/*/src`; closes
     the "47 date formatters" / "4 rate limiters" pattern).
   - Net-negative / cleanup turns are always allowed (deletion target — the
     research-recommended positive constraint).

3. **Semantic tier — grounded weighted rubric** (`prompt` Haiku → `agent`
   escalation). Per arXiv 2601.04171 + LLM-review-pipeline practice:
   - Input is **not a raw diff**: hunks expanded to the enclosing function;
     mechanical-refactor hunks separated from logic hunks (judge logic only);
     plus the request, the test-vs-impl slice, and the last assistant message
     (`transcript_path`) for the honesty check.
   - **Rubric axes**, each weighted `must` / `important` / `nice`:
     intent-fidelity, completion-honesty, test-integrity (impl-fitted-to-test
     / narrow-happy-path / cross-turn shaping), architecture (layer leakage,
     duplication, unrequested abstraction — the senior-architect axis),
     edge-coverage. Binary score per item, aggregate, threshold **0.7**.
   - **Block only on a failed `must` item at `confidence = high` with
     evidence** (cited `file:line`); no evidence → downgrade, do not block.
     `important`/`nice` failures and ambiguous results are logged advisory,
     not blocking (narrow blocking tier — trust-preserving).
   - Escalate to the `agent` reviewer only on ambiguous + high-risk signal
     (test∩impl changed together, or diff touches credential/auth paths).

4. **`.claude/agents/intent-reviewer.md`** — out-of-context `agent`-tier
   target. Fresh context; `tools: Read, Grep, Bash` (read-only: `git diff`,
   `cargo nextest run -p`); `maxTurns: 8`; not background. Receives **only**
   request + expanded diff + test slice — never the session history, so it
   cannot inherit the implementer's rationalisations. **Agentic rubric**: it
   explores the repo to ground criteria in concrete files/symbols before
   grading (grounded rubrics catch real root-cause-missed in 54 % of
   stricter-than-tests rejections). Returns `{verdict, confidence,
   violations[] with file:line}`.

5. **Inoculation + abstention-as-success** — in the **implementation and
   producer subagent prompts** (by role: whichever subagents write or produce
   code — the AI-Factory worker/`loop-producer`/plan agents today, the `ce-*`
   equivalents after migration): *"If a task is hard, implement it
   correctly — do not make the gate pass by fitting tests or adding
   unrequested scaffolding; surfacing that this is tempting is acceptable,
   doing it is not. Concluding that no change is needed (issue already fixed,
   request already satisfied) is a fully successful outcome, not a failure."*
   The gate must **not** block a correctly-justified abstention (empty diff +
   an explicit "no change needed because …" is `allow`).

6. **Escape hatch** — `// budget-justified: <reason>` for the structural tier;
   turn-state `intent_ack` (user approved a known-partial result) or the
   N-bound for the semantic tier. Every release reason is logged.

7. **Observability (DoD)** — every verdict (allow / block / escalate) with
   tier, axis, confidence, evidence and reason appended to
   `<git-common-dir>/.nebula-guard/intent-log-<sid>.jsonl`.

### Cost envelope

Structural tier: pure bash+git, ~0 cost, every qualifying Stop. Semantic
tier fires only when C passed **and** the structural tier passed **and** there
is a real impl diff — ~once per successful completion. Haiku ≈ 0.1¢; the agent
tier only on flagged-ambiguous-high-risk (rare). Bounded retry caps worst case.

### Wiring & invariants

- `settings.json` `Stop`: `stop-gate.sh` first, `intent-gate.sh` second; new
  `SubagentStop` entry matching the current implementation-worker subagent
  name (one string the `ce-*` migration workstream re-points).
- `task hooks:test` gains proofs (repo invariant): block-on-unambiguous-fake,
  allow-on-honest-green, allow-on-justified-abstention, net-LoC-budget-block,
  duplicate-symbol-block, budget-justified-escape, loop-bound-after-N,
  no-LLM-when-no-diff, allow-when-C-already-denied.
- CLAUDE.md "Enforced Discipline" gains Layer-2 rows; D10 prose notes Layer-2
  is an **addition** above the structural core (core determinism unchanged).
- `docs/adr/README.md` thematic index gains an "Agent harness" row → 0083.
- `docs/QUALITY_GATES.md` gains a note: the `cognitive_complexity` /
  `too_many_lines` workspace `allow` stays; new code is held to the
  `clippy.toml` thresholds **diff-scoped** by Layer-2, not workspace-wide.

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
