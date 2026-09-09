# nebula-env — Agent orientation
> Local guide for `crates/env/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** One typed env-var parsing contract (`var`/`parse`/`flag`/`list`) so every crate stops re-rolling `std::env::var(...).unwrap_or_default().parse()` with divergent bool/int semantics.
**Layer:** Cross-cutting — importable from any layer, no upward deps, `std` + `thiserror` only (root AGENTS.md -> Layered Dependency Map).

## Commands

- `cargo nextest run -p nebula-env --features testing` — exercise the `EnvGuard` RAII helper (feature-gated, `unsafe` env mutation)

## Key files

- `src/lib.rs` — crate root: re-exports the reader fns + `EnvError`; gates `testing` module; `forbid(unsafe_code)` unless `test`/`testing`.
- `src/reader.rs` — the parsing contract: `var`/`var_opt`/`parse`/`parse_or`/`flag`/`flag_or`/`list`.
- `src/error.rs` — `EnvError` (`thiserror`); the single typed failure surface consumers map at their boundary.
- `src/testing.rs` — `testing::EnvGuard`: process-global lock + restore-on-drop for serialized env mutation in tests.

## Conventions & never-do

- `var` is required: unset is `EnvError::Missing`. `var_opt`/`parse`/`flag` return `Ok(None)` when unset; `*_or` defaults only on absence, not invalid input. `parse` can return `EnvError::Parse`, and `flag` returns `EnvError::Invalid`; string readers reject non-Unicode input. `list` deliberately returns an empty list for unset or non-Unicode input.
- `EnvError::Invalid` retains the rejected value and `Parse` retains parser text. These are not secret-safe public errors: callers reading sensitive configuration must map them to redacted boundary errors.
- `EnvGuard` serializes cooperating users of its lock; it does not lock arbitrary environment readers or third-party threads. Keep environment-mutating tests isolated and do not assume the helper makes concurrent process-wide mutation safe.
- `unsafe` lives ONLY behind the `testing` module (edition-2024 env mutation); core stays `forbid(unsafe_code)`. Don't introduce `unsafe` outside `testing`.
- This crate does NOT define config structs or map into other crates' errors — consumers convert `EnvError` into their own typed error (`ApiConfigError`, `ProviderError`, …) at the boundary.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Parsing, defaults, or guard restoration | Unit tests in [src/tests.rs](src/tests.rs) with `--features testing`; cover absent, malformed, non-Unicode, and restored values as applicable. |

## See also

- `README.md` — full design · ADR-0086 (placement rationale + workspace env conventions)
