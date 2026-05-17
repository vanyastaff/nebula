# Agent Discipline & `.claude/` Curation — Design

- **Date:** 2026-05-16
- **Status:** Approved design (pre-implementation). Next step: implementation plan.
- **Scope:** `.claude/` agent configuration only. No product code, no CI workflow
  changes, no new subagents.
- **Owner:** vanyastaff

## 1. Motivation

Distilled from the Bun "Rewrite Bun in Rust" port (oven-sh/bun#30412):
discipline that lives only in prose (AGENTS.md, skills) is rationalized away by
the agent over time — it drifts to the path of least work, weakens tests to get
green, suppresses lints, and claims "done" without verifying. The only thing
that held in Bun's setup was **harness-enforced hooks, hardened against the
agent's own evasion, committed to the repo** (their `pre-bash-zig-build.js`
literally parsed around redirect/pipe evasion; comment: "Claude is a sneaky
fucker"). Their skills were symptom-keyed context; their success came from a
falsifiable per-step oracle, not a clever prompt.

Nebula today is the inverse of Bun: 18 predefined subagents + ~23 generic
`aif-*` skills, **zero discipline hooks**. The real quality gate (`lefthook`)
fires only at `git commit`/`git push` and checks fmt/clippy/deny — never "did
you weaken a test", "is this a costyl", "did you verify before claiming done".
Between commits the agent is unconstrained.

Owner decision (locked): zero tolerance for cheating. Soft warnings on
cheat-symptoms are exploited over time → **hard-deny, presume evasion**. Soft
warnings are acceptable only for *quality/idiom judgment* (not cheating), and
only where a linter cannot already catch it.

## 2. Goals / Non-Goals

**Goals**

1. Move discipline from prose into harness-enforced, committed hooks.
2. Make cheating mechanically infeasible where a symptom is detectable
   (test-weakening, lint-suppression, `--no-verify`, costyl markers).
3. Make "done" falsifiable: the turn cannot end claiming completion unless the
   real gate (clippy + nextest for touched crates) was observed green.
4. Remove friction the owner explicitly flagged (permission prompt-walling,
   per-commit full-workspace clippy, the Windows `cargo fmt --all` footgun).
5. Curate `.claude/` toward Rust + strictness + thoughtfulness: trim dead
   weight skills/subagents, Rust-ify the keepers, land it durably.

**Non-Goals**

- No new subagents (Bun did 1M LOC with zero; the lever is not "more agents").
- No CI workflow edits. CI stays authoritative; `lefthook` mirrors it.
- No mechanical detection of "architectural costyl vs proper rework" — that is
  irreducible judgment, carried by `review-sidecar` + a symptom skill, not a
  hook. This limit is stated honestly and is by design.

## 3. Locked Decisions

| # | Decision | Source |
|---|----------|--------|
| D1 | Enforcement posture: hard-deny on cheat symptoms; Stop-gate on done-claims; soft-warn only for non-cheat idioms | Owner |
| D2 | Anti-cheat scope: test-integrity **+ costyl-proxies** | Owner |
| D3 | lefthook pre-commit clippy → changed-crate scoped; full-workspace clippy → pre-push | Owner |
| D4 | Replace ad-hoc `permissions.allow` with a curated, committed `.claude/settings.json` | Owner |
| D5 | aif-skill curation: Rust-ify + tighten + delete dead weight; edits land in `.ai-factory/skill-context/` (Rule 7 — base files are overwritten on update) | Owner + audit |
| D6 | Subagent curation: delete 9 orphan `loop-*`, merge 5 sidecars → 1, keep 4 load-bearing, trim dead MCP grants | Owner + audit |
| D7 | Remove BridgeSpace integration (in-repo only; owner removed the out-of-repo product directly) | Owner |
| D8 | Invert doc canon: **CLAUDE.md is canonical**, AGENTS.md becomes a thin pointer to it (non-standard; owner chose it over AGENTS.md-as-canon with the trade-off shown) | Owner |
| D9 | Hook runtime = **bash + jq** under `.claude/hooks/*.sh` (supersedes the Node `.mjs` draft — owner rejected a new Node dependency; repo already requires bash. **Location revised `scripts/guard/` → `.claude/hooks/*.sh`** — owner 2026-05-17, per Claude Code's documented hooks convention + the cited Bun `.claude/hooks/` precedent this design emulates (§12); `.claude/hooks/` is git-tracked, so the committed/structural guarantee is unchanged). ~~Hook A is fail-closed.~~ **Fail-closed-A clause SUPERSEDED by D10.** Runtime decision (bash+jq) stands. | Owner |
| D10 | **Security boundary relocated to the oracle.** 5 adversarial rounds proved a hand-rolled bash shell-parser on a security boundary is an un-winnable arms race. The no-cheat guarantee is carried structurally by **B** (edit-guard: no test-weakening) + **A2** (recorder, now self-protecting: refuses to record a green gate if the clippy command suppressed lints via `-A`/`--allow`/`RUSTFLAGS=…-A`) + **C** (Stop-gate: no "done" without a recorded clean gate) + lefthook/CI. `resolve_cmd`/`normalize_argv0` are **deleted**; **hook A is demoted** to a cheap **fail-open** advisory substring tripwire (its parser correctness is no longer security-critical). Guarantee preserved — *relocated*, not weakened. B and C remain hard. **(C's crate-set sourcing refined by D11.)** | Owner |
| D11 | **Governing principle (3rd recurrence — make it canon, stop re-deriving per round): never rest a guarantee on one fragile detector; source it from ground truth.** Adversarial review of B found the no-cheat guarantee resting on B's bash-regex `impl_files_edited` recording, which Rust's idiomatic colocated `#[cfg(test)] mod tests` in `src/*.rs` silently poisons (edit logic next to the test mod ⇒ not recorded ⇒ C blind ⇒ cheat). Fix is structural, not regex-patching: **C (Stop-gate) derives the touched-crate set from `git diff --name-only` ground truth** (modified `crates/*/src/**` ⇒ a clean recorded gate is required for that crate), with B's `impl_files_edited` as *corroborating* belt-and-suspenders only. Test-weakening is then structurally defeated by A2-clean-gate + CI even if B's weaken-deny misses a shape; **B's weaken-deny + costyl checks are demoted to early edit-time advisory** (still fix CRIT-1 recording-by-path, CRIT-2/3 weaken coverage, per-occurrence `// guard-justified:` — but B need not be perfect). Guarantee = C-via-git-diff + A2-clean-gate + CI (three ground-truth layers). | Owner |

## 4. Architecture — Enforcement Layers

Hook runtime: **POSIX `bash` + `jq`** (D9). Rationale: the repo **already
requires bash** (lefthook runs `bash scripts/pre-commit-fmt-check.sh`; the
worktree flow uses `bash scripts/worktree.sh`), so bash hooks add **zero new
toolchain**, whereas a Node runtime would be a new project-level dependency the
owner explicitly rejected. `bash 5.2` (git-bash) and `jq 1.8` are present and
verified on the dev machine; git-bash is proven to work here (lefthook). Hook
scripts live in **`.claude/hooks/*.sh`** — Claude Code's documented hooks
directory + the Bun `.claude/hooks/` precedent this design emulates (§12),
not the repo's `scripts/` dir; wiring in **committed `.claude/settings.json`**
(`command`-type, invoking `bash "$CLAUDE_PROJECT_DIR/.claude/hooks/<x>.sh"`).
Hook arrays concatenate across settings scopes; `settings.local.json` carries
no hooks after §7. Add
`"$schema": "https://json.schemastore.org/claude-code-settings.json"`. `jq` is
the only new (tiny, ubiquitous, already-installed) dependency; if `jq` is
absent a hook degrades **fail-closed for A** / fail-open elsewhere (see below).

**Prerequisite toolbelt (declared, not implicit):** `bash ≥ 4` (associative
arrays / `[[ =~ ]]` — `sh`/dash is intentionally NOT targeted: the fail-closed
`normalize_argv0` tokenizer needs arrays and would be far more bypass-prone in
POSIX sh, and the repo already mandates bash via lefthook), `jq`, `git`,
coreutils. Allowed additions must be small single-purpose binaries of this
class (e.g. `rg`), declared here, and degrade safely if absent — **never** a
runtime/ecosystem (Node/Python). **Relocation-safe by construction:** no
absolute paths in committed hooks/settings — wiring uses `$CLAUDE_PROJECT_DIR`,
each hook self-locates via `$(dirname "${BASH_SOURCE[0]}")`, turn-state via
`git rev-parse --git-common-dir`; cloning/moving the repo needs no edits.

Hook contract: blocking via **`exit 2` + stderr reason** (uniform across
PreToolUse/Stop — no JSON emission needed; simpler and robust in bash). `jq`
parses stdin (`tool_input.command`, `file_path`, `session_id`,
`stop_hook_active`, …). **Failure-mode split (the load-bearing design choice):**
a hook that hits its *own internal error* fails **open** (`exit 0`) so a broken
guard never wedges the session — EXCEPT hook **A**, which is fail-**closed** on
input it **cannot confidently normalize**. `resolve_cmd` removes *balanced*
quotes exactly as the shell concatenates them (`ca'rg'o`→`cargo`,
`-m "fix: x"`→`-m fix: x`) and matches deny rules on that resolved form, so
benign quoted commands (`git commit -m "msg"`, `gh pr create --title "X"`) pass
and obfuscated ones (`ca"rg"o`, `--"no-verify"`) are still caught; only
genuinely unanalyzable input — shell substitution/chaining/metachars,
*unbalanced* quotes, or `env --split-string` — denies. Scoped fail-closed: an
over-block on the narrow unanalyzable set is a safe annoyance, an under-block is
a security hole; the guard does **not** cripple normal workflow. This makes a
deliberately *simple, conservative* bash matcher strictly safer than a clever
tokenizer that can be evaded (the
Node draft had exactly such a bypass, caught in review). Hooks complete < 2 s.

> **D9/D10 supersession note (read this before §4.A above's preamble):** the
> preceding paragraph's `resolve_cmd`/fail-closed-A/"scoped fail-closed"
> narrative is **SUPERSEDED by D10** and retained only as design history. Per
> D10: `resolve_cmd`/`normalize_argv0` are **deleted**; **hook A is fail-open**
> (advisory tripwire, not a security boundary); the no-cheat guarantee is B +
> A2 (lint-suppression-aware recorder) + C + lefthook/CI. Every hook is a
> `.claude/hooks/<role>.sh` bash script (jq for JSON, `exit 2`+stderr to block;
> fail-**open** on internal error for *all* hooks including A — only B and C
> are hard-deny on their conditions). The authoritative A/A2 behavior is the
> two subsections immediately below (already D10-correct); the implementation
> **plan** carries the exact bash code.

### A. PreToolUse / Bash — `.claude/hooks/bash-deny.sh` (matcher `Bash`) — D10: demoted to a fail-open advisory tripwire

**Not a security boundary** (D10 — that role moved to B+A2+C). No shell parser,
no `resolve_cmd`/`normalize_argv0` (deleted). A cheap best-effort substring
check that **fails open** (any doubt / no jq / non-Bash / unreadable → `allow`;
C is the real guard). It still denies the *blatant literal* cases as a helpful
early nudge (a rare false-deny on e.g. a doc commit message is acceptable —
it's advisory, the agent rewords; it is no longer the guarantee):

- raw command literally contains `git commit` **and** `--no-verify` /
  `--no-gpg-sign` / `core.hooksPath=` (unambiguous long forms only — `-n`
  dropped: too noisy against message text).
- raw literally contains `cargo`…`fmt` **and** a standalone `--all`.
- raw literally contains `git push` **and** `--force`/`--force-with-lease`/
  standalone `-f`, no `NEBULA_ALLOW_FORCE=1`.

Obfuscation (`ca'rg'o`, wrappers, quotes) is **not** A's problem anymore: the
*outcome* of any cheat is caught structurally by B (can't weaken tests) + A2/C
(can't record/claim a green that wasn't a clean gate).

> **Verified harness facts (2026-05, code.claude.com/docs/en/hooks + issue #6371) — load-bearing for D10:**
> (1) `PostToolUse` stdin `tool_response` for Bash is a **structured object**
> (`stdout`/`stderr`/`exit_code`/`success`), not a flat string — A2 reads the
> authenticated `exit_code`/`success`. (2) `PostToolUse` **does not fire for
> exit≠0 Bash commands** (documented, "not planned") — so a failing gate
> *never reaches* A2 → never recorded → C blocks: fail-safe by construction.
> (3) The `Stop` hook has **no access to tool results / transcript** — so C
> (Stop-gate) must and does read only the turn-state file. This validates C's
> mechanism as the single viable one.
>
> Consequence: A2 records green via an **allowlist of the canonical *clean*
> gate invocation** (not a blocklist of evasions). Anything with
> chaining/masking/redirection/comment (`|| && ; | $( \` > < #`), lint
> suppression (`-A`/`--allow`/`--cap-lints`/`RUSTFLAGS=…`), or a non-`cargo`/
> `task` argv0 (`echo …`, `grep …`) is simply **not recognized** ⇒ not
> recorded ⇒ C blocks. Over-strictness only forces the agent to run the gate
> plainly — the safe direction, and finite (no arms race).

### A2. PostToolUse / Bash — `.claude/hooks/record.sh` (matcher `Bash`) — the oracle's integrity gate (D10)

Reads the tool result/exit code (PreToolUse cannot). Records
`gate_green: [<crate>…]` **only when** `cargo clippy -p <crate> -- -D warnings`
**and** `cargo nextest run -p <crate>` (or `task dev:check`) exit `0` **and**
the clippy command did **not** suppress lints — i.e. it must **refuse to record
green** if the recognized gate command contains `-A`/`--allow`/
`RUSTFLAGS=…-A…`. This self-protecting check is the *structural* home of the
old hook-A clippy rule (D10): a suppressed clippy is not a clean gate, so C
will still block "done". This is the falsifiable anchor consumed by C and the
load-bearing no-cheat guarantee.

### B. PreToolUse / Edit|Write|MultiEdit — `.claude/hooks/edit-guard.sh`

> **D11:** B is **early edit-time advisory**, not the sole guarantee (that is
> C-via-git-diff + A2-clean-gate + CI). Required corrections: (a) record the
> impl file into turn-state by **file path** (`is_lib_rust`) *independent of
> payload `#[cfg(test)]` markers* — the payload-test signal gates only the
> unwrap/allow/TODO sub-checks (where clippy backstops), **never** the
> recording (CRIT-1); (b) the test-weaken deny also covers **`Write`** to a
> test path and **in-`src` `#[cfg(test)]`** edits (CRIT-2/3); (c)
> `// guard-justified:` is **per-occurrence/adjacent**, not one-unlocks-all
> (IMPORTANT-2); (d) the unwrap/panic deny honors the same
> `// guard-justified:` escape so doc-comments/strings aren't hard-wedged
> (MINOR-1). B may still miss exotic shapes — acceptable, because C+A2+CI is
> the structural backstop. Behavioral intent (unchanged) below.

Operates on proposed new content / diff. **Deny:**

- New `unwrap() | expect() | panic!()` in library `.rs` (excludes `#[cfg(test)]`,
  `tests/`, `const`, `bin`/`examples`). AGENTS.md is absolute; caught at write
  time, not at commit. No escape.
- New `#[allow(…)] | todo!() | unimplemented!() | unreachable!()` in non-test
  code — escape **only** via an explicit justification comment
  `// guard-justified: <reason>` on the preceding line (same philosophy as the
  workspace's `undocumented_unsafe_blocks` ergonomics: suppression must be
  justified and reviewable, never silent).
- New `// TODO | FIXME | HACK | XXX` or plan-id markers (`TODO(A-5)`,
  `Phase A`, task IDs) in committed code (comments must read fine after the
  plan is deleted).
- `let _ = <call>` swallowing a `Result`/must-use where the callee name matches
  `transition|send|write|commit|flush|lock|spawn` (the
  `let _ = transition_node(…)` class).
- **Test-integrity:** weakening a `*/tests/*.rs` / `#[cfg(test)]` unit
  (removing/commenting an `assert*!`, adding `#[ignore]`, substituting
  `assert!(true)` / `assert_eq!(x, x)`, deleting a `#[test]` fn, blind bump of
  `*.snap` / inline `expect![[…]]`) **while the same turn already edited a
  non-test file** (turn-state correlation = the cheat signature). A pure test
  refactor with no impl edit in the turn passes.

### C. Stop — `.claude/hooks/stop-gate.sh` (matcher `""`)

Honors `stop_hook_active` (already true → `exit 0`, no re-block — deadlock-safe).
**D11: the touched-crate set is derived from git ground truth, not from B's
recording.** C runs read-only `git -C <repo> diff --name-only` **and**
`git status --porcelain` (uncommitted) **plus** `git diff --name-only
<turn-base>..HEAD` (turn-base = HEAD recorded by A0 at turn start via
`rev-parse --verify -q` — on an unborn branch this yields an *empty* turn-base,
not the literal `HEAD`, so C skips the diff arm and degrades to
git-status + B-union + the lefthook/CI commit-leg, the intended zero-commit
behavior) → any modified `crates/<x>/src/**.rs` ⇒ crate `<x>` is "touched". (Running `git` is
read-only and triggers no tools — Stop-hook-safe.) Union with turn-state
`impl_files_edited` (corroborating belt; never the sole signal). If any touched
crate lacks `gate_green` coverage (or the `*workspace*` sentinel) → `exit 2`:
*"You changed <crates> but never showed a clean clippy + nextest green for
them. Run the gate before claiming done."* This is the structural no-cheat
anchor: weakening a test cannot help (A2 records green only for a clean
canonical gate that must genuinely pass; CI re-runs authoritatively), and a
poisoned/missed B recording can no longer blind C. Forces red-first: new
behavior with no clean recorded gate cannot be reported done.

### A0. UserPromptSubmit — `.claude/hooks/turn-reset.sh` (matcher `""`)

Rotates/initializes the turn-state file for `session_id` at the start of each
user turn (resets `impl_files_edited` and `gate_green`). This is the concrete,
non-hand-wavy reset mechanism C and A2 depend on. Injects no context; the only
`UserPromptSubmit` hook once BridgeSpace is removed.

### D. PostToolUse / Edit|Write|MultiEdit — `.claude/hooks/fmt.sh`

After editing a `.rs` file: `rustfmt --edition 2024 <file>` (rustfmt.toml
supplies the rest); for `.toml`: `taplo fmt <file>`. Format **only that file**,
no behavior change, < 1 s. Single-file `rustfmt` is used deliberately with an
explicit `--edition 2024` (rustfmt does not infer edition from `Cargo.toml`);
this matches the workspace's established per-file fmt invocation.
Mirrors Bun's `post-edit-zig-format.js`, including its deliberate note:
**format-only, no organize-imports** (import reorg breaks split edits — add
import in edit 1, use in edit 2). The agent never accumulates fmt debt and
never needs `cargo fmt --all`. Explicitly NOT clippy here (synchronous
per-edit clippy is too slow per current guidance; clippy stays at the gate via
A2/C).

### Justified-escape pattern

The single sanctioned bypass for B's discretionary denies is a preceding
`// guard-justified: <reason>` line. This converts silent corner-cutting into a
reviewable, greppable, auditable decision (`rg "guard-justified"` becomes a
review surface). No env flag, no CLI bypass for the edit guard.

### Turn-state file

`<git-common-dir>/.nebula-guard/turn-<session_id>.json`, where
`<git-common-dir>` is resolved via `git rev-parse --git-common-dir` (fallback
`${TMPDIR:-/tmp}/nebula-guard`). **This is worktree-safe**: in a git worktree
`.git` is a *file*, not a directory, so a naive `<repo>/.git/` path breaks
exactly in this environment; the common-dir is shared, never tracked, never
staged. Shape:
`{ session, started_at, impl_files_edited: [...], gate_green: [...crates] }`.
Reset by the A0 `UserPromptSubmit` hook at the start of each user turn.
Concurrent worktrees/sessions are isolated by `session_id` in the filename.

## 5. G. Skill Curation

**Durability constraint (Rule 7):** base `.claude/skills/aif-*/SKILL.md` are
overwritten on AI-Factory update. All Rust-ification lands in
`.ai-factory/skill-context/<skill>/SKILL.md` (the sanctioned override path).
Deletion is durable via a disable/exclude entry in `.ai-factory/config.yaml` /
`.ai-factory.json` **plus** directory removal — reversible, and not resurrected
by an update.

**DELETE (dead weight / canon-conflicting; ~76 non-Rust payload files):**
`aif-dockerize`, `aif-ci`, `aif-build-automation`, `aif-architecture`,
`aif-loop`, `aif-roadmap`. Rationale: zero Rust templates; would overwrite
canonical `Taskfile.yml` / `ci.yml` / `deny.toml` / AGENTS.md layer map;
`aif-loop` is reimplemented inline by the `loop-*` path it shares.

**RUST-IFY + TIGHTEN (via skill-context overrides):** `aif-fix`,
`aif-implement`, `aif-best-practices`, `aif-review`, `aif-security-checklist`,
`aif-verify`, `aif-plan`. `aif-fix` currently *mandates* `console.log`/
`try-catch` — illegal here. Inject one shared **Rust-strictness ruleset**
(identical to what hooks B/C enforce — one discipline, two layers):

- no `unwrap()/expect()/panic!()` in lib code;
- every new state/error/hot-path ships a `thiserror` variant + `tracing` span +
  invariant check (observability is DoD);
- cross-crate via `nebula-eventbus`, not layer-violating imports;
- strictness gate = `task dev:check`; architecture gate = `cargo deny check`
  against `deny.toml [wrappers]` (mechanical, not prose); any clippy warning is
  a hard fail; account for the Windows `cargo fmt --all` os-error-206 (no false
  green from a deep worktree);
- branch/worktree via `scripts/worktree.sh`; commit scope = crate name without
  `nebula-` prefix, convco-validated; stage `Cargo.lock` on any dep change.

Optional (changelog 2.1.119): the Rust-ified skills MAY gate extra-strict
checks behind `${CLAUDE_EFFORT}` (e.g. deeper invariant/perf review only at
`high`/`xhigh`) so low-effort runs stay fast. Enhancement, not required.

**MERGE (proposals — vetoable individually):** `aif-explore`+`aif-grounded` →
one investigate skill; `aif-improve` → `aif-plan --refine`; useful part of
`aif-qa` → `aif-verify` (drop the manual-QA framing — wrong for a
nextest/doctest workspace); `aif-rules` → `aif-evolve`/AGENTS.md.

**KEEP AS-IS:** `aif-skill-generator` (prompt-injection scanner, not covered by
superpowers), `aif-evolve` (override backbone), `aif-reference`,
`aif-grounded` (unless merged).

**E. Symptom-keyed skill (Bun GC-skill analog):** new `nebula-pitfalls` skill
whose `description` is the symptom list (loads on-symptom, not always/never),
routing `docs/pitfalls.md` trap-classes + Rust-1.95 anti-patterns
(`async_trait`, `Box<dyn Error>`, `Arc<Mutex>` default) → the exact Nebula
rule/ADR/memory. Thin router, not a duplicate of `rust-expert`/
`aif-best-practices`. Symptom-first descriptions are official best practice
(the description does ~90% of skill selection).

## 6. H. Subagent Curation

Execute **jointly** with G (skill renames silently break sidecar/`plan-polisher`
`skills:` injection).

**DELETE — 9 orphan `loop-*`:** `loop-orchestrator`, `loop-planner`,
`loop-producer`, `loop-evaluator`, `loop-critic`, `loop-refiner`,
`loop-test-prep`, `loop-perf-prep`, `loop-invariant-prep`. `aif-loop` inlines
these as generic `Task` agents; nothing spawns the files; registered only in
`.ai-factory.json`. No skill edit required to remove them.

**KEEP — 4 load-bearing:** `implement-worker` (`isolation: worktree`,
non-replicable, fixed dispatch target), `plan-polisher` (fixed target + skill
bundle + write scope), `implement-coordinator` & `plan-coordinator` (top-level
`Agent`-spawning entrypoints — subagents cannot spawn subagents). **Trim the
dead `mcp__handoff__*` tool grants** from both coordinators (no `handoff` MCP
registered in this workspace).

**MERGE — 5 sidecars → 1 `quality-sidecar`:** `review-sidecar`,
`security-sidecar`, `best-practices-sidecar`, `docs-auditor`,
`commit-preparer` are one template × (injected skill, output contract).
Collapse to a single parameterized sidecar (skill + mode as input);
preserve the `sonnet` pin for the docs/commit modes (only meaningful
divergence). Requires updating `implement-coordinator`'s `Agent(...)` allowlist
and spawn calls — the one invasive change in H; sequenced accordingly.

Post-curation roster ≈ 5 (`implement-coordinator`, `implement-worker`,
`plan-coordinator`, `plan-polisher`, `quality-sidecar`) — squarely in the
community-recommended 3–7 band; externally validated, and consistent with
Anthropic's own note that Opus over-delegates.

## 7. F. Friction Removal

- **Permissions:** replace the 60-entry ad-hoc `permissions.allow` accretion
  with a curated, committed `.claude/settings.json`:
  `Bash(cargo *)`, `Bash(cargo nextest *)`, `Bash(task *)`, `Bash(git *)`,
  `Bash(gh *)`, `Bash(bash scripts/*)`, plus the default-allowed Read/Glob/Grep.
  Key synergy: permissions can be broadened safely **because the Bash deny-hook
  (A) is now the real guard**, not the allowlist. Only genuinely personal,
  machine-specific entries remain in `settings.local.json`.
- **BridgeSpace removal (done this session):** the integration is dropped
  (owner evaluated the product, rejected it). In-repo footprint removed: the
  three `settings.local.json` hooks (UserPromptSubmit/Stop/Notification →
  `bs-claude-hook.cjs`) and the `bs-mail` / `mcp__bridgemind__*` /
  `.bridgespace/...` `permissions.allow` entries (settings.local.json now valid
  JSON, no `hooks` key, zero bridge residue). The agent's first whole-file
  rewrite was correctly blocked by the harness as permission-file
  self-modification; removal was completed via targeted Edits after the owner
  granted permission. Out-of-repo (`~/.bridgespace/`) was removed by the owner
  directly. Stale memory `reference_bridgemind_nebula` was deleted + de-indexed.
- **lefthook.yml:** pre-commit `clippy` → changed-crate scoped; add
  full-workspace `clippy -D warnings` to pre-push (CI parity preserved —
  pre-push must mirror CI required jobs; removes the coarse-commit pain).

## 8. Cross-Cutting: Hooks Go Stale

Current-practice caveat (community, contradicting the naive "hooks prevent
cheating"): hooks rot; bypasses get found. Mitigations, mandatory:

1. **`.claude/hooks/__tests__/`** — each guard ships a Node test asserting it
   **denies the bad case and allows the good case** (the Bun "test is NOT
   VALID" analog applied to the guards themselves). Wired into `task` (e.g.
   `task hooks:test`) and runnable locally.
2. **Canon ⇄ hooks sync** — per D8, **CLAUDE.md is canonical**; it carries a
   short "Enforced Discipline" section enumerating each hard rule and naming
   the guard that enforces it. AGENTS.md is a thin pointer to CLAUDE.md. A
   pre-push check (or `task` target) fails if a guard file referenced in
   CLAUDE.md is missing. Same philosophy as `lefthook` mirroring CI.
3. Hooks log denials to `.git/.nebula-guard/denials.log` (local, uncommitted)
   for periodic review of false-positive rate.

## 9. Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| False-positive denies obstruct legit work (the thing the owner asked to avoid) | `// guard-justified:` escape for discretionary B rules; hard rules (no-unwrap, --no-verify) have no false-positive class; denial log reviewed |
| Stop-gate deadlock / infinite loop | C is side-effect-free, honors `stop_hook_active`, fails open on internal error |
| Skill rename breaks sidecar/`plan-polisher` `skills:` injection | G+H executed jointly; ordered plan; post-change `task` smoke that every referenced skill resolves |
| Sidecar 5→1 merge breaks `implement-coordinator` | Sequenced last in H; coordinator spawn-path updated and exercised before deleting old sidecars |
| Rust-ification wiped by AI-Factory update | All overrides in `.ai-factory/skill-context/`; deletions in config + dir; never base files (Rule 7) |
| Hook latency hurts UX | bash+jq, < 2 s budget, fmt-only post-edit, clippy stays at gate not per-edit |
| D9: bash command-parsing for hook A is harder/bug-prone than a real tokenizer (a Node-draft bypass was caught in review) | Hook A is **fail-closed**: un-normalizable command ⇒ deny, so parser gaps over-block (safe) not under-block (bypass); conservative matcher + shell test suite (`task hooks:test`) |
| D9: `jq` not installed on some machine | Present+verified here; declared a prerequisite alongside bash; hook A degrades fail-closed (deny) without jq, others fail-open |
| Hook A: a wrapper NOT in the known set (`doas`, `proxychains`, `ssh host …`) makes the wrapper the resolved argv0, so `doas cargo fmt --all` is allowed | **Tracked residual, accepted** (out of Plan-1 scope, owner-scoped, low likelihood for an agent's own shell). `env --split-string` (the realistic variant) IS closed. Future hardening: treat an unresolved-but-followed-by-more-tokens leading word as UNPARSEABLE, or extend the wrapper set. Not blocking. |
| D8 inversion: non-Claude AGENTS.md consumers (Cursor/Copilot/Codex/generic) read only a pointer, losing the rules | AGENTS.md pointer explicitly names CLAUDE.md as canonical; `.cursor/rules/*` + `.github/copilot-instructions.md` updated to point at CLAUDE.md; generic AGENTS.md-only readers seeing just the pointer is an owner-accepted trade-off |
| Future session reverts AGENTS.md to canon out of habit (the harness/CLAUDE.md long said "treat AGENTS.md as source of truth") | D8 recorded in committed spec + project memory; the inverted CLAUDE.md states it is canonical at the top |

## 10. Implementation Ordering (constraints for the plan)

1. Curated `.claude/settings.json` (permissions + `$schema`); confirm
   `settings.local.json` (personal, now BridgeSpace-free) still loads and
   hook arrays concatenate as expected.
2. **D8 doc-canon inversion**: CLAUDE.md absorbs the canonical content
   (project map + rules) and gains the "Enforced Discipline" section;
   AGENTS.md → thin pointer naming CLAUDE.md canonical; update
   `.cursor/rules/*` and `.github/copilot-instructions.md` to point at
   CLAUDE.md; drop the old "treat AGENTS.md as source of truth" line.
3. Hook scripts A0, A, A2, B, C, D + their `__tests__`; wire into
   `.claude/settings.json`; cross-link from CLAUDE.md "Enforced Discipline".
4. Skill deletes (G) + subagent `loop-*` deletes (H) — independent, parallel.
5. Skill Rust-ification via `.ai-factory/skill-context/` (G) **jointly** with
   sidecar 5→1 merge + coordinator rewiring + dead-MCP trim (H).
6. `nebula-pitfalls` symptom skill (E).
7. `lefthook.yml` commit-granularity change (F) + pre-push full clippy.
8. Post-change smoke: every `skills:`/`Agent(...)` reference resolves;
   `task hooks:test` green; a deliberate cheat attempt (weaken a test +
   edit impl) is denied; a clean change is not; AGENTS.md pointer + updated
   cross-ref files resolve to CLAUDE.md.

## 11. Acceptance Criteria

- Each hook has a passing test proving deny-bad / allow-good.
- A scripted cheat (remove an `assert!`, edit impl same turn) is **denied**.
- `git commit --no-verify`, `cargo clippy -A…`, `cargo fmt --all` from worktree
  are **denied** with actionable redirects.
- Ending a turn after editing `crates/*/src/**` without a recorded green gate
  is **blocked**.
- A clean, properly-verified change flows with **no** false denial.
- Subagent roster = 5; no dead `mcp__handoff__*`; every `skills:` reference
  resolves.
- aif Rust-ification survives a simulated AI-Factory update (lives in
  skill-context).
- `.claude/settings.json` validates against its `$schema`.
- No BridgeSpace residue in repo config: zero `bs-mail` / `mcp__bridgemind__*`
  / `bs-claude-hook` references under `.claude/`. (In-repo removal already
  done this session; verified valid JSON, no `hooks`, zero residue.)
- D8: CLAUDE.md opens by declaring itself canonical and carries the Enforced
  Discipline section; AGENTS.md is a pointer naming CLAUDE.md; `.cursor/rules/*`
  and `.github/copilot-instructions.md` resolve to CLAUDE.md; no remaining
  "treat AGENTS.md as source of truth" wording.

## 12. Sources

- oven-sh/bun#30412 (`CLAUDE.md`, `.claude/hooks/pre-bash-zig-build.js`,
  `post-edit-zig-format.js`) — the enforced-discipline pattern.
- Claude Code docs: sub-agents, hooks, skills, settings
  (`code.claude.com/docs/en/{sub-agents,hooks,skills,settings}`).
- Community 2026: claudefa.st (cross-platform hooks; subagent best practices),
  pubnub.com, nimbalyst.com (3–7 subagent consensus; skill description budget).
- Internal audits (2026-05-16): aif-skill audit, subagent audit, Claude Code
  changelog/blog delta-check vs this spec (this session) — verdict: no blocking
  revision; `permissionDecision`/`stop_hook_active`/hook-array-concatenation/
  skill+subagent frontmatter all confirmed current through 2026-05.
