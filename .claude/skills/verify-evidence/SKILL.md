---
name: verify-evidence
description: Use before claiming a task is "done" / "fixed" / "working" / ready for review. Requires fresh command output, not memory. Prevents completion theatre and silently skipped work. This is the Iron Law gate — no completion claims without evidence from commands run this turn.
---

# verify-evidence

## When to invoke

- You are about to say "done" / "fixed" / "working" / "ready for review".
- You are about to create a commit, open a PR, or hand off.
- The user asked "did it work?" and you haven't run the commands this session.
- You are tempted to say "I believe the tests pass" — especially then.

## The Iron Law

**No completion claim without fresh command evidence.**

Memory of a green run from 30 minutes ago is not evidence — the state may have changed, you may have edited files since, CI may have different gates. Run the commands this turn, capture the output, paste it into your reply.

Banned phrases: *should work*, *no issues expected*, *I believe it's fine*, *the tests passed earlier*, *it compiles in my head*.

Required phrases: actual tool output — a test summary line, a clippy "0 warnings" line, a fmt no-diff confirmation.

## Checklist

### 1. Canonical fast gate (root `CLAUDE.md`)

Read the current fast-gate commands from root `CLAUDE.md` §"Canonical Commands", then run each one and capture the output. Every listed `cargo` invocation. Every time. Capture the last ~10 lines of each, especially the summary line from `nextest`.

Do not hardcode the command list here — it would drift from the canon. `CLAUDE.md` is the source of truth for the gate.

### 2. Full gate (before PR)

Use the current "Full validation" commands from root `CLAUDE.md` §"Canonical Commands".

`lefthook` pre-push additionally runs `cargo check --workspace --all-features --all-targets`, `cargo check --no-default-features` for selected crates, `cargo doc`, and `cargo shear`. You do not need to run those by hand — the push-time mirror handles them. If CI fails on one, diagnose root cause (do not `--no-verify`).

### 3. Single-crate iteration mode

If iterating fast:

```bash
cargo check -p nebula-<crate>
cargo nextest run -p nebula-<crate>
```

…but the **fast gate (§1) still runs before declaring done**. Crate-scoped checks are for the dev loop, not for claiming completion.

### 4. Intentionally skipped steps

If any canonical step was skipped (for example, `--all-features` is slow locally), say so **explicitly**:

> Skipped `cargo check --workspace --all-features` because [reason]. Will be enforced in CI via `pre-push`.

Never silently drop a step.

### 5. New / modified tests

- [ ] New tests are in the suite and **PASSED** — not `#[ignore]`'d to move the green line.
- [ ] Any new `#[should_panic]` / `#[ignore]` has a 1-line comment explaining why.
- [ ] If the work claims a behavior change, at least one test exercises the new behavior.

### 6. Planned deletions

If the plan said to remove X, confirm X is actually gone — do not ship a partial removal:

- [ ] `git grep "X"` returns only legitimate references (tests asserting removal, changelog).
- [ ] `cargo build` does not reference X.

### 7. Doctest discipline

- [ ] No newly-introduced intra-doc-link brackets to out-of-scope paths in `//!` / attribute docs — `rustdoc -D warnings` will fail on paths it cannot resolve.

### 8. Lefthook mirror check

`lefthook.yml` pre-push is the local mirror of CI required jobs (see root `CLAUDE.md` §"Canonical Commands" and the repo's own `lefthook.yml`):

- [ ] If this PR adds a CI required job, `lefthook.yml` pre-push gains the same check in the same PR, so local push and CI cannot diverge.

## Output format

Paste **actual tool output**. Do not paraphrase. Do not summarize "all green" without the evidence.

```
## Verification evidence

(One section per fast-gate / full-gate command you ran, using the current
canonical list from root `CLAUDE.md`. Headers are named after the step,
not the exact invocation — the command itself lives in `CLAUDE.md`.)

### fmt
<last 3 lines of the fmt step's output, or "(no output — clean)">

### clippy
<last 10 lines of the clippy step's output>
(clippy: 0 warnings across <N> crates)

### nextest
<at minimum the final Summary line, e.g. "Summary [  X.YZs] N tests run: N passed, 0 failed, 0 skipped">

### doctests (if ran)
<summary>

### deny (if ran)
<summary>

### Skipped steps
- none
OR
- <step>: <reason>

### Deletions confirmed (if applicable)
- X: removed from <crate>, grep clean
```

Only after this evidence block is **fresh and complete** are you allowed to say "done" / "working" / "ready for review".
