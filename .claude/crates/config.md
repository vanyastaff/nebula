# nebula-config
Multi-source configuration with env interpolation, hot-reload, and typed access.

## Invariants
- `ConfigBuilder::build()` is **async**. Config loading hits the filesystem and environment.
- Sources are merged in order: later sources override earlier ones (last-wins).

## Key Decisions
- Env variable interpolation uses `${VAR}` and `${VAR:-default}` syntax — evaluated at load time.
- Supported formats: TOML, JSON; YAML behind `yaml` feature.
- Hot-reload: `FileWatcher` + `with_hot_reload(true)` triggers callbacks on file change.
- `config.get::<T>("path.to.key")` for typed access — dot-separated path, arrays indexed by position (`arr.1.name`).

## Traps
- Invalid array index (`arr.x`) and out-of-bounds (`arr.5.name`) return descriptive errors — they don't panic.
- Empty path segments are rejected: leading dot (`.a`), trailing dot (`a.`), and consecutive dots (`a..b`) all return `PathError`. Don't silently accept malformed paths.
- `ConfigResultAggregator` collects multiple errors before returning — use for batch validation.
- `EnvLoader` requires the `env` feature flag. Not default.
- `PollingWatcher` is the fallback when native file watching isn't available — it polls on an interval.
- **`interpolate` takes `Value` by value** (not `&Value`) — callers must pass ownership. In `builder.rs` this means `merged_data` is moved into `interpolate(merged_data)`.
- **`reload()` does NOT re-interpolate** — env var references (`${VAR}`) in config sources are NOT re-resolved on hot-reload. Interpolation runs once at initial build time only. Pre-existing bug, tracked separately.

## Relations
- Depends on nebula-log (re-exports `info!`, `debug!` etc. in prelude). Used by nebula-api, nebula-runtime, and any crate needing runtime configuration.

<!-- reviewed: 2026-04-06 — perf: optimized hot paths (3.5x flatten, 2.2x build), added benchmarks; no architectural changes -->

<!-- reviewed: 2026-04-11 — Workspace-wide nightly rustfmt pass applied (group_imports = "StdExternalCrate", imports_granularity = "Crate", wrap_comments, format_code_in_doc_comments). Touches every Rust file in the crate; purely formatting, zero behavior change. -->
