# Code Review Checklist — Nebula

When reviewing code (own or others'), verify each category:

## Correctness
- [ ] Does the change do what it claims?
- [ ] Are edge cases handled (empty input, zero, overflow, None)?
- [ ] Are error paths tested, not just happy paths?
- [ ] No silent swallowing of errors (`.ok()`, `let _ =` on Results that matter)

## Architecture
- [ ] Layer boundaries respected — no upward deps (Core → Business → Exec)
- [ ] Cross-crate communication via `EventBus`, not direct imports between peers
- [ ] New public types in `nebula-core` approved? (cascade risk)
- [ ] DI via `Context` — no global state or singletons

## Safety & Security
- [ ] No `unwrap()` / `expect()` outside tests
- [ ] No hardcoded secrets, credentials, or API keys
- [ ] `unsafe` blocks have `// SAFETY:` justification
- [ ] External input validated at boundaries

## API Surface
- [ ] Public API has doc comments with `# Examples` and `# Errors`
- [ ] Breaking changes documented and intentional
- [ ] Enums that may grow are `#[non_exhaustive]`
- [ ] Error types are meaningful, not stringly-typed

## Tests
- [ ] New behavior has corresponding tests
- [ ] Test names describe behavior: `rejects_X`, `returns_Y_when_Z`
- [ ] No flaky tests (race conditions, timing deps, random data without seed)
- [ ] Integration tests use `MemoryStorage`, not mocks

## Performance
- [ ] No unnecessary `clone()` in hot paths
- [ ] Async functions don't block the runtime (`spawn_blocking` for CPU work)
- [ ] No unbounded collections growing without limit

## Hygiene
- [ ] `cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace` passes
- [ ] Commit messages follow conventional commits
- [ ] `.claude/crates/{name}.md` updated if invariants/decisions/traps changed
