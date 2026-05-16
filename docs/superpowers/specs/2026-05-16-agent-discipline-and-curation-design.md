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

## 4. Architecture — Enforcement Layers

Hook runtime: **Node.js `.mjs`** (Node 22 present; already proven — BridgeSpace
hooks use `node`). Cross-platform per current guidance (no bash/PowerShell
syntax in hook bodies; `path.join`, `os.tmpdir`). Hook scripts live in
`.claude/hooks/`; wiring in **committed `.claude/settings.json`** (hooks arrays
concatenate with `settings.local.json`, so BridgeSpace hooks are unaffected).
Add `"$schema": "https://json.schemastore.org/claude-code-settings.json"`.

Hook contract (official): PreToolUse returns
`hookSpecificOutput.permissionDecision: "deny"` + reason, `exit 0`. Blocking
Stop returns `exit 2` + stderr. Non-blocking notes `exit 1`. Every hook
validates stdin JSON strictly and **fails open with `exit 0` on its own
internal error** (a broken guard must never wedge the session) while logging to
stderr. Hooks must complete < 2 s.

### A. PreToolUse / Bash — `nebula-guard-bash.mjs` (matcher `Bash`)

Hardened parser (strip inline env assignments, `>`/`>>`/`2>&1`/`|` redirects
before matching — explicit anti-evasion, mirrors Bun). **Deny:**

- `git commit … --no-verify | -n | --no-gpg-sign | -c core.hooksPath=…` —
  bypassing `lefthook` is the top-level cheat; blocked first.
- `cargo clippy … -A… | --allow … | RUSTFLAGS=…-A…` — silencing the linter to
  reach green is cheating the oracle.
- `cargo fmt --all` / `cargo +nightly fmt --all` — **unconditional** deny with
  redirect to `bash scripts/pre-commit-fmt-check.sh` (or `cargo fmt -p <crate>`).
  The `--all` form is never needed once D formats per-file, and it is the exact
  shape that trips the silent Windows os-error-206 and produces the false
  "dev:check green from a worktree" report. No path-length heuristic →
  unambiguous. Discipline **and** friction removal in one rule.
- `git push --force | --force-with-lease` targeting shared history without an
  explicit `NEBULA_ALLOW_FORCE=1` override (AGENTS.md).

### A2. PostToolUse / Bash — `nebula-guard-record.mjs` (matcher `Bash`)

Reads the tool result/exit code (PreToolUse cannot). When
`cargo clippy -p <crate> -- -D warnings` **and** `cargo nextest run -p <crate>`
(or `task dev:check`) exit `0`, record `gate_green: [<crate>…]` into the
turn-state file. This is the falsifiable anchor consumed by C.

### B. PreToolUse / Edit|Write|MultiEdit — `nebula-guard-edit.mjs`

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

### C. Stop — `nebula-guard-stop.mjs` (matcher `""`)

Side-effect-free (reads turn-state only; runs no tools — deadlock-safe).
Honors `stop_hook_active` from stdin: if already true, `exit 0` (no re-block).
Otherwise, if since the last user message the agent edited `crates/*/src/**`
but turn-state has **no** `gate_green` covering every touched crate → `exit 2`
with stderr: *"You changed <crates> but never showed clippy + nextest green for
them. Run the gate before claiming done — weakening tests to get there is
blocked by guard-edit."* This is the structural fix for "claims done without
verifying / fixes tests to green"; it also forces red-first (landing new public
behavior with zero test delta cannot be reported done).

### A0. UserPromptSubmit — `nebula-guard-turn-reset.mjs` (matcher `""`)

Rotates/initializes the turn-state file for `session_id` at the start of each
user turn (resets `impl_files_edited` and `gate_green`). This is the concrete,
non-hand-wavy reset mechanism C and A2 depend on. Concatenates with the
existing BridgeSpace `UserPromptSubmit` hook (arrays merge); injects no context.

### D. PostToolUse / Edit|Write|MultiEdit — `nebula-guard-fmt.mjs`

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
`os.tmpdir()/nebula-guard/`). **This is worktree-safe**: in a git worktree
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
  (A) is now the real guard**, not the allowlist. Personal/BridgeSpace entries
  remain in `settings.local.json`.
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
2. **AGENTS.md ⇄ hooks sync** — a short "Enforced Discipline" section in
   AGENTS.md enumerates each hard rule and names the guard that enforces it; a
   pre-push check (or `task` target) fails if a guard file referenced there is
   missing. Same philosophy as `lefthook` mirroring CI.
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
| Hook latency hurts UX | Node, < 2 s budget, fmt-only post-edit, clippy stays at gate not per-edit |

## 10. Implementation Ordering (constraints for the plan)

1. Curated `.claude/settings.json` (permissions + `$schema`); verify
   BridgeSpace hooks in `settings.local.json` still run (concatenation).
2. Hook scripts A0, A, A2, B, C, D + their `__tests__`; wire into settings;
   AGENTS.md "Enforced Discipline" section.
3. Skill deletes (G) + subagent `loop-*` deletes (H) — independent, parallel.
4. Skill Rust-ification via `.ai-factory/skill-context/` (G) **jointly** with
   sidecar 5→1 merge + coordinator rewiring + dead-MCP trim (H).
5. `nebula-pitfalls` symptom skill (E).
6. `lefthook.yml` commit-granularity change (F) + pre-push full clippy.
7. Post-change smoke: every `skills:`/`Agent(...)` reference resolves;
   `task hooks:test` green; a deliberate cheat attempt (weaken a test +
   edit impl) is denied; a clean change is not.

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
- `.claude/settings.json` validates against its `$schema`; BridgeSpace
  notifications still fire.

## 12. Sources

- oven-sh/bun#30412 (`CLAUDE.md`, `.claude/hooks/pre-bash-zig-build.js`,
  `post-edit-zig-format.js`) — the enforced-discipline pattern.
- Claude Code docs: sub-agents, hooks, skills, settings
  (`code.claude.com/docs/en/{sub-agents,hooks,skills,settings}`).
- Community 2026: claudefa.st (cross-platform hooks; subagent best practices),
  pubnub.com, nimbalyst.com (3–7 subagent consensus; skill description budget).
- Internal audits (2026-05-16): aif-skill audit, subagent audit (this session).
