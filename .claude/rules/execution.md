# Execution Rules

## Confidence Check

Before starting any task, assess confidence silently:
- **≥90%**: proceed without comment
- **70–89%**: state approach in 1 sentence, then proceed
- **<70%**: ask 1–2 clarifying questions before doing anything

## Wave Pattern (parallel I/O)

For multi-file changes, batch I/O instead of serial read-edit cycles:
1. **Read wave** — read all relevant files in parallel
2. **Analyze** — form the plan, identify all touch points
3. **Edit wave** — apply all edits in parallel
4. **Verify** — `cargo check`, then `cargo nextest run` on affected crates

## Self-Check Before Reporting Done

After implementing, before saying "done":
1. Re-read the changed files — does the diff match the intent?
2. Check `.claude/crates/{name}.md` invariants — did I violate any?
3. Run `cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace`
4. If any step fails, fix it silently — don't report partial success

## Task Sizing

- If a task touches >5 files or >3 crates, outline the plan first and wait for approval
- If a task requires breaking changes to `nebula-core` traits, always ask before proceeding
- For exploratory questions ("how does X work?"), answer concisely — don't refactor

## Tooling — Use the Fast Path

Always prefer faster tools when available:

| Slow | Fast | When |
|------|------|------|
| `cargo test` | `cargo nextest run` | Always — parallel execution, better output |
| `cargo nextest run --workspace` | `cargo nextest run -p nebula-<crate>` | Single-crate changes — skip unrelated crates |
| `cargo check --workspace` | `cargo check -p nebula-<crate>` | Quick iteration on one crate |
| `cargo clippy --workspace` | `cargo clippy -p nebula-<crate>` | Single-crate lint check |

### Iteration loop (fastest)
```bash
# While developing — check + test only the crate you're changing
cargo check -p nebula-<crate> && cargo nextest run -p nebula-<crate>
```

### Pre-commit (full)
```bash
cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace
```

### Pre-PR (complete validation)
```bash
cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace && cargo test --workspace --doc && cargo deny check
```

`cargo test --doc` runs separately because nextest doesn't support doctests.

## Error Recovery

- If a compile error persists after 2 fix attempts, stop and explain the root cause
- Never silence warnings with `#[allow(...)]` unless the lint is a known false positive
- Never use `unwrap()` or `expect()` outside of tests
