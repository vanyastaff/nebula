<!--
# budget-justified: human contributor onboarding doc with table-driven reference sections (toolchain matrix, command matrix, schema-style standards lists) intentionally exceeds the structural-block cap
-->

# Contributing to Nebula

Thanks for considering a contribution. Nebula is a Rust workflow automation
engine. The bar for changes: **the canon stays honest, the public API stays
correct, and observability ships with every state and error path.**

## Project Philosophy

- **Type-safety over discipline.** Encode invariants in types where possible;
  don't rely on "remember to call this helper."
- **Failures are part of the contract.** Every `Result` variant should be
  documented with when it appears.
- **Observability is part of Definition of Done.** A new state, error, or
  hot path ships with a typed error variant + tracing span + invariant check.
- **One way to do it.** We delete shortcuts when they drift from the canon.

Machine-enforced agent rules and the canonical workspace map live in
[`AGENTS.md`](AGENTS.md). Product canon lives in `docs/PRODUCT_CANON.md`.

## Repository Layout

See the *Workspace Layout* and *Layered Dependency Map* sections of
[`AGENTS.md`](AGENTS.md). Short version:

- `crates/` — 36 workspace members.
- `examples/` — runnable examples (one workspace member, not per-crate).
- `docs/` — agent doc map (`docs/README.md` is the entry point).
- `Taskfile.yml` — the canonical task runner. Don't call raw `cargo` for
  fmt / lint.

## Development Setup

```sh
# 1. Toolchain pinned in rust-toolchain.toml; rustup will install it.
rustup show

# 2. Install Taskfile (https://taskfile.dev) and lefthook
brew install go-task lefthook   # or: scoop install task lefthook

# 3. Install hooks
lefthook install
```

## Required Toolchain

| Tool             | Why                                                      |
|------------------|----------------------------------------------------------|
| `cargo`          | Rust 1.95+, edition 2024, resolver 3                     |
| `task`           | `Taskfile.yml` is the canonical entry point              |
| `lefthook`       | Local pre-commit / pre-push (mirrors CI required jobs)   |
| `convco`         | Conventional-Commits validation                          |
| `cargo-deny`     | License + layer-wrapper enforcement                      |
| `cargo-nextest`  | Fast test runner used by `task dev:check`                |

## Common Commands

```sh
task dev:check                            # full pre-PR gate (fmt + clippy + nextest + doctests + deny)
task fmt                                  # cargo fmt --all (pinned toolchain)
task clippy                               # cargo clippy --all-targets --all-features -- -D warnings
task test                                 # workspace tests
task doc                                  # workspace rustdoc
cargo nextest run -p <crate>              # single-crate nextest
cargo test -p <crate> --doc               # doctests for one crate
```

Use `task --list` for the full catalog.

## Coding Standards

- **No `unwrap()` / `expect()` / `panic!()` in library code.** Tests, `const`,
  and binaries are exempt per [`clippy.toml`](clippy.toml). The
  `.claude/hooks/edit-guard.sh` enforces this on every edit.
- **No `TODO` / `FIXME` / `HACK` / plan-IDs in committed code.** Use ADRs and
  GitHub issues for tracking; comments should read fine after the plan is
  deleted.
- **Cross-crate communication goes through `nebula-eventbus`.** Don't reach
  across layer boundaries with direct imports.
- **No async locks across `.await`.** No unbounded channels without explicit
  justification.
- **Layered dependency.** New code must respect the *Layered Dependency Map*
  in [`AGENTS.md`](AGENTS.md); `cargo-deny` will reject layer violations.

## Documentation Standards

- Public API: rustdoc with `# Errors`, `# Cancellation`, `# Panics`,
  `# Safety`, `# Examples` sections wherever they apply.
- Crate-level `//!`: the manual for that crate.
- `crates/<name>/README.md`: human entry point.
- Use intra-doc links (`` [`Resource`] ``, `` [`crate::Manager`] ``) so
  docs.rs cross-references hold across renames.
- Examples must compile (`,no_run` if they hit a real backend; `,ignore`
  only with a justification in the surrounding prose).

## Testing Expectations

- Unit tests next to the code (`#[cfg(test)]` module).
- Integration tests in `crates/<name>/tests/`.
- Compile-fail / proc-macro tests via `trybuild` (see
  `crates/resource/tests/trybuild/`).
- New public API → at least one happy-path test + one error-path test.

## Commit / PR Guidelines

- **Branch from `main`.** Create persistent worktrees with
  `bash scripts/worktree.sh new <slug> <type> <scope>`.
- **Conventional Commits**, validated by `convco`. Scope is the crate
  name without `nebula-` prefix (`resource`, `engine`, `api`) or a
  top-level area (`docs`, `ci`).
- **Squash-merge to `main`.** Never force-push shared history.
- One concern per PR. If you find an unrelated issue, file it.

## Issue Guidelines

Use the templates in `.github/ISSUE_TEMPLATE/`:

- **Bug report** — what went wrong, repro, env.
- **Feature request** — what the gap is, what the world looks like after.
- **Documentation issue** — what's wrong, what it should say.

## Architecture Change Policy

Major architectural changes require an ADR. Design records (ADRs, roadmap,
specs, research) are maintained in the maintainers' private design vault and
are not tracked in this public repository. External contributors propose such a
change via a detailed issue or PR description; maintainers record the resulting
ADR in the private vault. An ADR is required if the change:

- Adds a new public crate, or removes one.
- Changes a trait's associated types or method signatures (after a
  crate has consumers).
- Crosses or moves a layer boundary in the Dependency Map.
- Introduces a new cross-cutting invariant.

Smaller refactors don't need an ADR but should reference the relevant
canon section in their PR description.

## Security Issues

See [`.github/SECURITY.md`](.github/SECURITY.md). **Do not open public
issues for security vulnerabilities.**

## License

By contributing you agree your work is licensed under the project's
license (see [`LICENSE`](LICENSE)).
