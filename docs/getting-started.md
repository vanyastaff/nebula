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

## First Contribution Flow

1. Read the architecture and roadmap pages first.
2. Pick an issue from the project backlog.
3. Create a branch and implement focused changes.
4. Run checks before opening a PR.

## Source Consolidation Notes

This page consolidates onboarding content previously kept in QUICK_START.md and NEWCOMERS.md.
Those root files are still present for review and can be deleted after confirmation.

## See Also

- [Architecture](ARCHITECTURE.md) - Workspace structure and layering
- [Contributing](contributing.md) - Contribution rules and checklist
- [Workflow](workflow.md) - Branch and PR lifecycle
