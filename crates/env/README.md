# nebula-env

Typed, cross-cutting environment-variable reader for the Nebula workspace.

## Role

**Cross-cutting** (same tier as `nebula-log` / `nebula-error` / `nebula-metrics`):
importable from any layer, no upward dependencies, `std` + `thiserror` only.
It provides one parsing contract so every crate stops re-implementing
`std::env::var(...).unwrap_or_default().parse()` with subtly different
defaults and bool/int semantics.

## API

| Function | Purpose |
|----------|---------|
| `var` / `var_opt` | required / optional string (`Err` on non-Unicode) |
| `parse` / `parse_or` | any `FromStr` type, trimmed; `Ok(None)` / default when unset |
| `flag` / `flag_or` | boolean — accepts `true/1/yes/on` and `false/0/no/off`, `Err` otherwise |
| `list` | split on whitespace and commas, dropping empties |

All failures surface as the typed [`EnvError`]; consumers map it into their
own error (`ApiConfigError`, `ProviderError`, …) at the boundary.

## Testing

Enable the `testing` feature for `nebula_env::testing::EnvGuard` — an RAII
guard that serializes process-env mutation behind a global lock and restores
prior values on drop:

```rust,ignore
let mut env = nebula_env::testing::EnvGuard::acquire();
env.set("API_RATE_LIMIT", "200");
// ... assert behaviour ...
// prior value (or "unset") is restored when `env` drops
```

This replaces the hand-rolled `env_lock` / `clear_env` harnesses previously
duplicated across crate test modules.

## Layer

See `CLAUDE.md` → *Layered Dependency Map* (Cross-cutting row) and
ADR-0086 for the placement rationale and the workspace env conventions.
