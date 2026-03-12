[← Previous Page](TASKS.md) · [Back to README](../README.md) · [Next Page →](workflow.md)

# Contributing

This page provides the practical contribution checklist for Nebula.

## Development Standards

- Use Rust 2024 edition style and crate conventions.
- Keep changes scoped and avoid unrelated churn.
- Prefer typed errors and documented public APIs.

## Before Opening a PR

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo check --workspace --all-targets
cargo test --workspace
cargo doc --no-deps --workspace
```

If your change is limited to one crate, run a fast pre-check first:

```bash
cargo check -p <crate>
cargo test -p <crate>
```

Then run full workspace checks before requesting review.

## PR Expectations

- Link the related issue.
- Describe user-visible and architecture-impacting changes.
- Add or update tests where behavior changes.

## Review Checklist (High Signal)

1. Scope: no unrelated refactors in the same PR.
2. Contracts: public API/trait changes are intentional and documented.
3. Layering: no new upward or circular dependencies.
4. Tests: changed behavior is covered by tests, not only manual verification.
5. Docs: update relevant docs when semantics or workflows changed.

## See Also

- [Getting Started](getting-started.md) - Initial setup and first run
- [Workflow](workflow.md) - Branching and review flow
