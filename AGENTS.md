# Repository Guidelines

## Project Structure & Module Organization
- `crates/` contains all publishable crates (`nebula-core`, `nebula-resource`, etc.); each exposes a `lib.rs` entrypoint under `src/`.
- Integration tests live beside crates under `crates/*/tests`; use them for multi-crate flows.
- `docs/` holds architecture briefs (e.g., `parameter-ui-bridge-implementation-status.md`) that must be refreshed when changing interfaces.
- Generated artifacts stay in `target/`; clean with `cargo clean` before benchmarking to avoid stale builds.

## Build, Test, and Development Commands
- `cargo check --workspace --all-features` ensures every crate compiles with shared feature flags.
- `cargo fmt` and `cargo fmt --check` enforce the shared `rustfmt.toml`.
- `cargo clippy --workspace --all-targets --all-features -D warnings` matches the stricter `clippy.toml` thresholds.
- `cargo test --workspace --all-features` runs unit and integration suites; add `-- --nocapture` when debugging.

## Coding Style & Naming Conventions
- Follow the workspace `rustfmt.toml`: 100-column max width, spaces, Unix line endings.
- Modules and files use `snake_case`; types and traits stay `PascalCase`; async functions prefer `verb_noun_async`.
- Re-export dependencies sparingly; prefer explicit `use` statements reordered by `rustfmt`.
- Document public items to satisfy `missing-docs-in-crate-items = true`; add rationale comments above complex algorithms only.

## Testing Guidelines
- Co-locate unit tests inside `#[cfg(test)]` modules; name them after behavior (e.g., `handles_empty_payload`).
- Use `mockall` for trait fakes and `pretty_assertions` for diff-friendly comparisons.
- Push cross-boundary scenarios into `crates/nebula-core/tests` or crate-specific `tests/` directories.
- Before merging, capture the command output of `cargo test` in the PR description; rerun when touching shared configs.

## Commit & Pull Request Guidelines
- Adopt Conventional Commits with scoped identifiers (`feat(nebula-parameter-ui)`, `docs(nebula-error)`).
- Write commits that ship one logical change and leave the workspace buildable.
- PRs include: concise summary, linked issues, screenshots for UI-bound crates, and risk/rollback notes.
- Mention impacted crates explicitly and list the commands you ran (`cargo fmt`, `cargo clippy`, `cargo test`).

## Documentation & Knowledge Sharing
- When adding features, update or cross-link the relevant brief in `docs/`; short status notes go to `docs/SESSION-SUMMARY.md`.
- Capture breaking API updates in crate-level `README.md` or `DOCS.md` (see `crates/nebula-core/README.md` for tone).
