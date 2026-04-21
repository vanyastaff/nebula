# Contributing to Nebula

Thanks for your interest. This guide walks you from clone to merged PR.

1. [Quick Start](#quick-start) — clone, build, test in under 5 minutes
2. [Prerequisites](#prerequisites) — toolchain and optional tooling
3. [Development Workflow](#development-workflow) — branches, style, tests, commits
4. [Submitting a Pull Request](#submitting-a-pull-request) — opening, review, merge
5. [References](#references) — key files and deeper docs
6. [License](#license)

---

## Quick Start

```bash
# Clone and build
git clone https://github.com/vanyastaff/nebula.git
cd nebula
cargo build --workspace

# Run tests
cargo nextest run --workspace

# Full pre-PR gate (fmt, clippy, tests, doctests, deny)
task dev:check
```

On a warm toolchain, clone → build → test completes in under 5 minutes.

For the full developer environment (sccache, lefthook, RA target dir, linker
tuning, worktree tips), see [docs/dev-setup.md](docs/dev-setup.md).

---

## Prerequisites

- **Rust 1.95+** (MSRV, pinned via `workspace.package.rust-version`)
- **[cargo-nextest](https://nexte.st/)** — test runner
- **Nightly `rustfmt`** — `rustup toolchain install nightly --component rustfmt`
- **[task](https://taskfile.dev/)** (recommended) — `task --list` for the catalog
- **[lefthook](https://github.com/evilmartians/lefthook)** — local pre-commit/pre-push hooks
- **`typos-cli`**, **`taplo-cli`** — hygiene checks

All dev tools can be installed in one step:

```bash
bash scripts/install-tools.sh
```

### Optional local automation

```bash
lefthook install
```

- `pre-commit` (≤10s): fmt, clippy, typos, taplo, cargo-deny
- `commit-msg`: conventional-commit validation via `convco`
- `pre-push` (crate-diff gate): runs `nextest` + `cargo check --all-features --all-targets`
  only for crates changed in the pending push range, with selected
  `--no-default-features` checks where applicable. Full doctests/docs/MSRV stay
  in CI required jobs.

See [docs/dev-setup.md](docs/dev-setup.md) for lefthook troubleshooting and
agent-profile notes.

---

## Development Workflow

### Branch naming

Branch from `main`. Use one of:

- `<username>/neb-<id>-<kebab-title>` — linked to a Linear issue
  (take the `gitBranchName` field from the issue, e.g.
  `vanyajohnstafford/neb-100-contributingmd-dev-setup-pr-flow`)
- `<type>/<short-kebab-description>` — for work without an issue
  (e.g. `fix/credential-zeroize-drop`, `docs/adr-index`)

Avoid long-lived feature branches; rebase on `main` frequently.

### Code style

- `cargo +nightly fmt --all` — nightly `rustfmt` is required (see `rustfmt.toml`)
- `cargo clippy --workspace -- -D warnings` — zero warnings
- No `unwrap()` / `expect()` / `panic!()` in library code (tests and binaries excepted)
- `thiserror` in libraries, `anyhow` in binaries
- Doc comments on every public item

Wider idioms, antipatterns, and the error taxonomy live in
[docs/STYLE.md](docs/STYLE.md).

### Testing

- Unit tests in `mod tests` inside source files
- Integration tests under `tests/`
- Always use `cargo nextest run` (not `cargo test`) — except for doctests:
  `cargo test --workspace --doc`
- Test names describe behaviour: `rejects_negative_timeout`, not `test_1`

### Commit messages

Conventional Commits are required and validated in CI by `convco` (commit
messages) and a regex check on the PR title — see
[`.github/workflows/pr-validation.yml`](.github/workflows/pr-validation.yml):

```
<type>(<scope>): <description>
```

- **Types:** `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`,
  `chore`, `ci`, `build`, `revert`
- **Scope:** crate name without `nebula-` prefix (e.g. `feat(resilience): …`,
  `fix(credential): …`) or a top-level area (`docs`, `ci`)
- Reference issues in the body: `Refs NEB-123` or `Closes NEB-123`

---

## Submitting a Pull Request

1. Push your branch and open a PR against `main`.
2. The PR body is prefilled from [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md) —
   fill in Summary, Changes, Testing, and the Safety checklist.
3. PR title must also follow Conventional Commits.
4. Required CI jobs must be green: `fmt`, `clippy -D warnings`, `nextest`,
   `doctests`, `MSRV 1.95`, `--all-features`, `--no-default-features`,
   `cargo deny`. A green `lefthook pre-push` catches most of these locally,
   but some jobs (notably the MSRV check) run only in CI.
5. **Squash-merge only** — keep `main` history linear.

### Code review

- Reviewers are auto-requested via
  [`.github/CODEOWNERS`](.github/CODEOWNERS).
- Security-sensitive paths (`crates/credential/`, auth, webhook) always
  require an owner sign-off.
- For non-trivial design or execution-lifecycle changes, complete the
  **Canon alignment** section in the PR template
  (see [docs/PRODUCT_CANON.md §17](docs/PRODUCT_CANON.md)).

---

## References

| File / Directory | Purpose |
|---|---|
| [`.github/CODEOWNERS`](.github/CODEOWNERS) | Auto-reviewer mapping by path |
| [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md) | PR body template + safety checklist |
| [`docs/adr/`](docs/adr/) | Architecture Decision Records (index) |
| [`docs/dev-setup.md`](docs/dev-setup.md) | Full developer environment guide |
| [`docs/PRODUCT_CANON.md`](docs/PRODUCT_CANON.md) | Normative architecture + Definition of Done |
| [`docs/STYLE.md`](docs/STYLE.md) | Idioms, antipatterns, error taxonomy |
| [`CLAUDE.md`](CLAUDE.md) | Operational guidance for coding agents |

---

## License

By contributing, you agree that your contributions will be licensed under MIT.
