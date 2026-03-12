[Back to README](../README.md) · [Next Page →](ARCHITECTURE.md)

# Getting Started

This guide is the recommended onboarding path for Nebula contributors.

## Prerequisites

- Rust 1.93+
- Cargo
- Git

## Clone and Build

```bash
git clone https://github.com/vanyastaff/nebula.git
cd nebula
cargo build
```

## Validate the Workspace

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo check --workspace --all-targets
cargo test --workspace
```

## Pick a Workstream First

Choose one concrete track before coding. This reduces context switching and keeps PRs focused.

| Workstream | Start Here | Typical Code Area |
|-----------|------------|-------------------|
| Engine/runtime behavior | [Architecture](ARCHITECTURE.md), [Tasks](TASKS.md) | `../crates/engine`, `../crates/runtime`, `../crates/action` |
| API routes/contracts | [API Reference](api.md), [Tasks](TASKS.md) | `../crates/api`, `../crates/webhook` |
| Credential/resource integration | [Architecture](ARCHITECTURE.md), [Tasks](TASKS.md) | `../crates/credential`, `../crates/resource` |
| Storage and persistence | [Architecture](ARCHITECTURE.md), [Tasks](TASKS.md) | `../crates/storage`, `migrations/` |
| Desktop UX/IPC | `../apps/desktop/README.md`, [Tasks](TASKS.md) | `../apps/desktop`, `../apps/desktop/src-tauri` |

## Fast Developer Loop (Per-Crate)

When you change one crate, prefer targeted checks first, then run full workspace checks before opening PR.

```bash
# Replace <crate> with package name, for example: nebula-action
cargo check -p <crate>
cargo test -p <crate>
```

For one specific test while debugging:

```bash
# Substring match
cargo test -p <crate> test_name

# Exact name
cargo test -p <crate> -- --exact exact_test_name
```

If you touched public API, shared types, or dependency wiring, run full validation:

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo check --workspace --all-targets
cargo test --workspace
```

## What to Validate by Change Type

1. Rust logic only (single crate): `cargo check -p <crate>`, `cargo test -p <crate>`.
2. Cross-crate contracts or traits: full workspace `check` and `test`.
3. Docs-only changes: verify links and keep commands/examples runnable.
4. API/behavior changes: add or update tests that demonstrate the new contract.

## Common Local Failure Patterns

1. `clippy` fails on warnings: fix warnings instead of suppressing; this workspace treats warnings as errors.
2. Formatting drift: run `cargo fmt` before re-running checks.
3. Wrong crate path/package name: use Cargo package names (`nebula-*`) for `-p` commands.
4. Feature work blocked by dependency order: check [Tasks](TASKS.md) and [Roadmap](ROADMAP.md) for prerequisites.

## First Contribution Flow

1. Read the architecture and roadmap pages first.
2. Pick an issue from the project backlog.
3. Create a branch and implement focused changes.
4. Run checks before opening a PR.

Minimal flow that works well in practice:

```bash
git checkout -b feat/<short-topic>
# implement
cargo check -p <crate>
cargo test -p <crate>
git add .
git commit -m "feat(<crate>): short summary"
```

## See Also

- [Architecture](ARCHITECTURE.md) - Workspace structure and layering
- [Contributing](contributing.md) - Contribution rules and checklist
- [Workflow](workflow.md) - Branch and PR lifecycle
