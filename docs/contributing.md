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

## PR Expectations

- Link the related issue.
- Describe user-visible and architecture-impacting changes.
- Add or update tests where behavior changes.

## Source Consolidation Notes

This page consolidates contribution guidance previously kept in CONTRIBUTING.md.
The root file remains temporarily for review.

## See Also

- [Getting Started](getting-started.md) - Initial setup and first run
- [Workflow](workflow.md) - Branching and review flow
- [Issues](issues.md) - Issue templates and triage
