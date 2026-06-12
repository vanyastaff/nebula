# nebula-env — Agent orientation
> Agent quick-map for `crates/env/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** One typed env-var parsing contract (`var`/`parse`/`flag`/`list`) so every crate stops re-rolling `std::env::var(...).unwrap_or_default().parse()` with divergent bool/int semantics.
**Layer:** Cross-cutting — importable from any layer, no upward deps, `std` + `thiserror` only (root AGENTS.md -> Layered Dependency Map).

## Commands
- `cargo check -p nebula-env`
- `cargo nextest run -p nebula-env`  ·  doctests: `cargo test -p nebula-env --doc`
- `cargo nextest run -p nebula-env --features testing` — exercise the `EnvGuard` RAII helper (feature-gated, `unsafe` env mutation)

## Key files
- `src/lib.rs` — crate root: re-exports the reader fns + `EnvError`; gates `testing` module; `forbid(unsafe_code)` unless `test`/`testing`.
- `src/reader.rs` — the parsing contract: `var`/`var_opt`/`parse`/`parse_or`/`flag`/`flag_or`/`list`.
- `src/error.rs` — `EnvError` (`thiserror`); the single typed failure surface consumers map at their boundary.
- `src/testing.rs` — `testing::EnvGuard`: process-global lock + restore-on-drop for serialized env mutation in tests.

## Conventions & never-do
- Readers are total and unsafe-free: unset vars yield `Ok(None)` / default / empty list — never panic. Only failures are non-Unicode (`var`) and unparsable bool (`flag`).
- `unsafe` lives ONLY behind the `testing` module (edition-2024 env mutation); core stays `forbid(unsafe_code)`. Don't introduce `unsafe` outside `testing`.
- This crate does NOT define config structs or map into other crates' errors — consumers convert `EnvError` into their own typed error (`ApiConfigError`, `ProviderError`, …) at the boundary.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · ADR-0086 (placement rationale + workspace env conventions)
