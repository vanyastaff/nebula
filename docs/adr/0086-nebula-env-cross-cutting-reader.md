# ADR-0086 — `nebula-env`: cross-cutting typed environment reader

- **Status:** Accepted
- **Date:** 2026-05-29
- **Supersedes:** N/A
- **Superseded by:** N/A
- **Related:** `docs/plans/2026-05-29-001-refactor-env-handling-handoff.md` (audit + plan)

## Context

Environment-variable handling was scattered across the workspace with no
shared contract: `nebula-api` had typed `parse_*_env` helpers, `nebula-log`
had separate lenient `parse_bool`/`parse_format`, and many sites hand-rolled
`std::env::var(..).unwrap_or_default().parse()`. Bool parsing diverged
(`log` treated any non-`false` value as `true`; `api` accepted a fixed set
and errored otherwise). Test modules in `api`, `log`, and `storage` each
hand-rolled an env lock plus raw `unsafe` `set_var`/`remove_var` blocks
(edition 2024), and `api`'s `clear_env` kept a manually-maintained key list
that drifts from the real registry. Full inventory: the handoff plan above.

## Decision

Introduce **`nebula-env`**, a cross-cutting crate (same tier as
`nebula-log` / `nebula-error` / `nebula-metrics` — importable at any layer,
no upward deps, `std` + `thiserror` only). It owns one parsing contract:

- `var` / `var_opt` — required / optional string (`Err` on non-Unicode).
- `parse` / `parse_or` — any `FromStr` type, trimmed.
- `flag` / `flag_or` — boolean. **Strict** accepted set
  `true|1|yes|on` / `false|0|no|off` (case-insensitive); any other value is
  `EnvError::Invalid`. This resolves the bool-semantics split in favour of
  fail-closed parsing (handoff F3 / Q5).
- `list` — split on whitespace and commas, dropping empties.

Variable names are `&str` (not `&'static str`) so dynamically-built,
prefixed names (per-provider OAuth vars) use the same path. All failures
surface as the typed `EnvError`; **consumers map it into their own error at
the boundary** (`ApiConfigError`, `ProviderError`, …) — `nebula-env` is
shared infra, not a public error contract, so it takes no `nebula-error`
dependency.

The `testing` feature ships `EnvGuard`: an RAII guard that serializes
process-env mutation behind a global lock and **restores prior values on
drop**, replacing the three hand-rolled harnesses and centralizing the one
`unsafe` boundary behind a safe API.

### Layer / deny.toml

Cross-cutting crates carry **no `deny.toml` `[wrappers]` entry** (they are
importable anywhere), so `nebula-env` adds none. It is listed in the
`CLAUDE.md` Layered Dependency Map Cross-cutting row and in
`[workspace.dependencies]`.

## Consequences

- One bool/parse/list contract; consumers stop re-implementing it.
- Test harnesses converge on `EnvGuard`; fewer `unsafe` blocks; no env
  leakage on test panic (the guard restores on drop).
- Migration is incremental and behavior-preserving where the existing
  contract already matches (api OAuth scopes → `list`; api `parse_bool_env`
  → `flag`; log precedence test → `EnvGuard`, all landed).

### Resolved after the initial decision

- **log `NEBULA_LOG_COLORS`**: now a real `ColorMode { Auto, Always, Never }`
  (private to `log::config::env`). `auto` honours TTY detection; `never`
  disables colours (regression fix — the old lenient `parse_bool` returned
  `true` for `never`, silently enabling them); bool aliases stay accepted
  for back-compat. The `colors: bool` field is unchanged — `ColorMode`
  resolves to it at env-override time.

### Deferred (follow-ups, see handoff plan)

- **api `parse_u64_env` / `parse_usize_env`**: keep their typed
  `ParseIntError` source; `nebula_env::parse` returns a `String` message, so
  delegating would be lossy. Left as-is.
- **api `clear_env` + storage/log test helpers**: converge on `EnvGuard`
  and drive any "clean slate" key set off a real registry (handoff Phase 0).
- **`DATABASE_URL`**: read only in `#[cfg(test)]` `pool()` helpers; a
  test-harness consolidation, not a production-contract change.
