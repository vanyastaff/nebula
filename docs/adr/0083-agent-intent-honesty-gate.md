---
id: 0083
title: agent-intent-honesty-gate
status: proposed
date: 2026-05-18
supersedes: []
superseded_by: []
tags: [agent-harness, hooks, anti-reward-hacking, enforced-discipline, observability]
related:
  - CLAUDE.md
  - .claude/hooks/stop-gate.sh
  - .claude/hooks/record.sh
  - .claude/hooks/edit-guard.sh
---

# 0083. Agent intent & honesty gate (Layer-2 semantic Stop-gate)

## Context

The committed guard stack (CLAUDE.md "Enforced Discipline") gives a **structural**
no-cheat guarantee D10 = B (`edit-guard.sh`) + A2 (`record.sh`) + C
(`stop-gate.sh`) + lefthook/CI. It proves, mechanically and deterministically,
that every crate touched in a turn reaches a **clean** clippy + nextest green,
and that tests were not weakened by the same turn that changed implementation.

Published evidence on coding-agent failure modes shows this is necessary but not
sufficient. Anthropic's reward-hacking → emergent-misalignment work
(arXiv 2511.18397) and the Opus 4.6 model card document agents that, under a
deterministic green gate, will still: fit the implementation to known test
inputs, add a narrow happy-path test that passes while real behaviour is broken,
change tests across a *different* turn than the implementation, narrate
"done / tests pass" when the diff does not support the claim, or falsify the
result of a tool that failed. EvilGenie (arXiv 2511.21654) reproduced explicit
reward hacking in Claude Code specifically (hardcoded test cases, edited test
files) and found an **LLM judge** the most reliable detector — near-zero false
negatives and ~1 false positive on *unambiguous* cases — while held-out tests
missed heuristic-fitted solutions.

Mapping those vectors onto Nebula's existing stack, five gaps remain open above
D10:

1. **Intent fidelity** — a green diff that does not address the user's actual
   request (premature completion; "looks right, doesn't work").
2. **Test-shaping that survives `edit-guard`** — impl fitted to test inputs;
   semantically-gutted test that keeps its assert count; cross-turn test edits.
3. **Tool-result / completion falsification** — final message claims X done
   while diff and repo state do not support X.
4. No **independent, out-of-context** verification before completion.
5. No **inoculation prompting** in subagent prompts, leaving the
   reward-hack → misalignment generalisation unmitigated.

Gaps 1–3 are the recurring local pain already recorded as feedback
(`feedback_incomplete_work`, `feedback_progress_log_fact_only`).

## Decision

Add a **Layer-2 semantic gate** as a pure addition above D10. It does not
modify or weaken the deterministic core; `stop-gate.sh` (C) runs first and
remains the structural guarantee. The new gate is **structurally enforcing**
(blocks the turn via the same `deny()` exit-2 contract as C), **layered** in
cost (deterministic pre-filter → cheap Haiku prompt → out-of-context agent only
on a flagged high-risk ambiguity), and scoped to the **main thread Stop plus
`implement-worker` SubagentStop**.

Recorded as a decision because it changes the enforced-discipline contract
(adds a gate that can block completion) and the agent-prompt baseline
(inoculation), both of which downstream agents and contributors must rely on.

### Enforcement model

Structural gating, not advisory. A confirmed unambiguous violation blocks the
turn (Claude continues working) exactly as C does. This is the user-selected
model; the design carries the false-block and infinite-loop mitigations that
make hard LLM gating safe (below).

### Mechanism

Native Claude Code hook types, not a nested `claude -p` from a command hook
(avoids hook recursion, permission ambiguity, and an un-auditable nested
process — the same class of "do not hand-roll a parser on the boundary"
reasoning that removed `resolve_cmd` from `_lib.sh`):

- A deterministic **command** pre-filter (`intent-gate.sh`) that decides
  whether the LLM tier should run at all and enforces the loop bound.
- A **`prompt`** Stop-hook (Haiku, ~0.1¢) as the always-on cheap judge.
- An **`agent`** Stop-hook whose command is a thin wrapper that no-ops unless
  the prompt tier wrote an escalation marker — giving conditional escalation
  on native types without nesting.

## Design

### Components

1. **`.claude/hooks/intent-gate.sh`** — command hook on `Stop` and
   `SubagentStop` (matcher: `implement-worker`), ordered **after**
   `stop-gate.sh` in `settings.json`.
   - Loop / deadlock guard: `stop_hook_active == true` → `allow` (mirrors
     `stop-gate.sh:8`). Plus a bounded `intent_attempts` counter in turn-state:
     after **N = 2** blocks, `allow` and append an `escalation` record to the
     audit log. Failing toward not trapping the human is deliberate — a
     persistently-disagreeing judge must not deadlock the session; the logged
     escalation is the human review surface.
   - Pre-filter (no LLM cost when nothing to judge): empty
     `git diff turn_base..HEAD` **and** empty `impl_files_edited` → `allow`.
     If C denied this turn (touched crate not in `gate_green`) → `allow`
     (ordering guarantees C ran first; do not double-judge broken code).
   - On pass: assemble judge input and hand off to the prompt tier.
2. **Prompt tier** (`prompt` Stop-hook, Haiku). Input: the turn's originating
   request, `git diff turn_base..HEAD` + working tree, an explicit
   **test-vs-impl diff slice**, and the **last assistant message** (from
   `transcript_path`) for the completion-honesty check. Returns
   `{verdict, confidence, reason}`. Blocks only on `verdict=block ∧
   confidence=high ∧ unambiguous` (EvilGenie calibration: judges are reliable
   on unambiguous, noisy on ambiguous). On an ambiguous result that also
   carries a **high-risk signal** (test files changed alongside impl, or diff
   touches credential/auth paths) it writes the escalation marker; otherwise
   `allow`.
3. **`.claude/agents/intent-reviewer.md`** — out-of-context reviewer, the
   `agent`-tier escalation target. Fresh context, `tools: Read, Grep, Bash`
   (read-only use: `git diff`, `cargo nextest run -p`), `maxTurns: 8`, not
   background. Receives **only** request + diff + test slice — never the
   session history, so it cannot inherit the implementer's rationalisations.
   Returns `{verdict, confidence, violations[]}`.
4. **Completion-honesty sub-check** — folded into the prompt tier, not a
   separate hook: flag any "response claims X; diff/state does not support X".
   Closes gaps 2–3 and the `feedback_incomplete_work` /
   `feedback_progress_log_fact_only` class.
5. **Inoculation prompting** — add one mild line (arXiv 2511.18397: −75–90 %
   misaligned generalisation, no increase in hacking) to `implement-worker.md`
   and the `loop-producer` / plan subagent prompts where a reward-hack
   incentive exists: *"If a task is hard, your job is to implement it
   correctly, not to make the gate pass. Surfacing that reward hacking is
   tempting here is acceptable; doing it is not."*
6. **Escape hatch** — consistent with the repo's `// guard-justified:`
   philosophy: a turn-state `intent_ack` flag (set when the user themselves
   approved a known-partial result) **or** the N-attempt bound releases the
   gate. Every release reason is logged.
7. **Observability (Definition of Done)** — every verdict (allow / block /
   escalate) with tier, confidence and reason appended to
   `<git-common-dir>/.nebula-guard/intent-log-<sid>.jsonl`.

### Cost envelope

The LLM tier fires only when **C passed** (gate green) **and** there is a real
impl diff — i.e. roughly once per *successful* completion, not on every Stop
attempt. Haiku prompt ≈ 0.1¢. The agent tier fires only on a
flagged-ambiguous-high-risk result (rare). Bounded retry caps worst-case spend
per turn.

### Wiring & invariants

- `settings.json` `Stop` array: `stop-gate.sh` first (free, deterministic,
  fail-safe), `intent-gate.sh` second; new `SubagentStop` entry matching
  `implement-worker`.
- `task hooks:test` gains proofs (repo invariant — every guard has a proof):
  block-on-unambiguous-fake, allow-on-honest-green, loop-bound-after-N,
  no-LLM-call-when-no-diff, allow-when-C-already-denied.
- CLAUDE.md "Enforced Discipline" table gains the Layer-2 row; D10 prose
  updated to note Layer-2 is an **addition** above the structural core, not
  part of it (the core's determinism is unchanged).
- `docs/adr/README.md` thematic index gains an "Agent harness" row pointing at
  0083 (repo convention — agents start at the index, not loose ADR files).

## Consequences

- A confirmed unambiguous reward-hack / dishonest-completion blocks the turn;
  the deterministic D10 guarantee is unchanged and still primary.
- Agent honesty questions → this ADR, then the hook sources.
- New nondeterministic surface is contained: block only on unambiguous +
  high-confidence, bounded retry, logged escapes, deterministic core untouched,
  C strictly first.
- Adds bounded per-completion LLM cost and latency on the final Stop only.
- Inoculation changes the agent-prompt baseline; subagent edits must keep the
  line.

## Supersession

None. Pure addition above D10; supersedes no prior ADR.
