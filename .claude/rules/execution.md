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
4. **Verify** — `cargo check`, then `cargo test` on affected crates

## Self-Check Before Reporting Done

After implementing, before saying "done":
1. Re-read the changed files — does the diff match the intent?
2. Check `.claude/crates/{name}.md` invariants — did I violate any?
3. Run `cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace`
4. If any step fails, fix it silently — don't report partial success

## Task Sizing

- If a task touches >5 files or >3 crates, outline the plan first and wait for approval
- If a task requires breaking changes to `nebula-core` traits, always ask before proceeding
- For exploratory questions ("how does X work?"), answer concisely — don't refactor

## Error Recovery

- If a compile error persists after 2 fix attempts, stop and explain the root cause
- Never silence warnings with `#[allow(...)]` unless the lint is a known false positive
- Never use `unwrap()` or `expect()` outside of tests
