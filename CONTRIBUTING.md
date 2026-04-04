# Contributing to Nebula

## Quick Start

```bash
# Clone and build
git clone https://github.com/vanyastaff/nebula.git
cd nebula
cargo build --workspace

# Run tests
cargo nextest run --workspace

# Check everything (before PR)
cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace
```

## Requirements

- Rust 1.94+ (MSRV)
- `cargo-nextest` for tests

## Pull Requests

- Branch from `main`, target `main`
- PR title: conventional commits (`feat(scope): description`)
- Squash merge only
- CI must pass: fmt, clippy, tests, MSRV, doc, deny

## Commit Convention

```
<type>(<scope>): <description>
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `chore`
Scope: crate name without `nebula-` prefix (e.g., `feat(resilience): ...`)

## Code Style

- `cargo fmt` with `rustfmt.toml`
- `cargo clippy -- -D warnings` (zero warnings)
- No `unwrap()` / `expect()` outside tests
- `thiserror` in libraries, `anyhow` in binaries
- Doc comments on all public items

## Testing

- Unit tests in `mod tests` inside source files
- Integration tests in `tests/` directory
- `cargo nextest run` (not `cargo test`)
- Test names describe behavior: `rejects_negative_timeout`

## License

By contributing, you agree that your contributions will be licensed under MIT.
