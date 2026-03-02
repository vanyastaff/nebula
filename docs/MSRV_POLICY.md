# MSRV Policy (Phase 5)

**Current baseline:** Rust 1.93 (edition 2024)

## Policy

1. **MSRV is explicit** — `rust-version` in workspace `Cargo.toml` and crate `Cargo.toml` files.
2. **CI enforces MSRV** — build and tests run on the declared MSRV.
3. **Bump process** — documented so upgrades are deliberate.

## Bump path (future upgrades)

1. Update `rust-version` in workspace and crates.
2. Run `cargo update`; resolve any dependency MSRV conflicts.
3. Run `cargo clippy --workspace -- -D warnings`; fix new lints.
4. Run `cargo test --workspace`.
5. Update CI matrix (e.g. `.github/workflows/ci.yml`) if needed.
6. Update this policy and ROADMAP Phase 5.
7. Tag release with MSRV in release notes.

## Rationale

- **1.93** — Rust 2024 edition, improved diagnostics, newer std.
- **Explicit policy** — downstream users know compatibility expectations.
- **Stable CI** — reproducible builds across contributors.
