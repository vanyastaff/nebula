[← Previous Page](contributing.md) · [Back to README](../README.md)

# Workflow

Nebula uses an issue-driven, branch-based development workflow.

## Branching

- Create short-lived topic branches from main.
- Keep branch names descriptive and issue-oriented.

Recommended branch patterns:

```bash
git checkout -b feat/<area>-<short-topic>
git checkout -b fix/<area>-<short-topic>
git checkout -b docs/<short-topic>
```

Examples:

- `feat/runtime-cancellation-routing`
- `fix/storage-postgres-timeout`
- `docs/getting-started-fast-loop`

## Commits

- Prefer focused commits by concern.
- Use clear, conventional commit messages when possible.

Examples:

```text
feat(action): stabilize capability context boundaries
fix(engine): handle missing dependency edge in scheduler
docs(getting-started): add crate-scoped validation loop
```

Before committing, run at least crate-scoped checks for changed code.

## Pull Requests

1. Open PR early with draft status if work is ongoing.
2. Request review after checks pass.
3. Address feedback with follow-up commits.

Practical PR checklist:

1. Problem and scope are explicit in PR description.
2. Tests or validations are listed with actual commands.
3. Behavior changes are covered by tests.
4. Docs are updated when API, flows, or contracts changed.
5. No unrelated refactors in the same PR.

## See Also

- [Contributing](contributing.md) - Quality and contribution standards
- [Getting Started](getting-started.md) - Setup and onboarding
