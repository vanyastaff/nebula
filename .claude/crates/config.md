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
- `ConfigResultAggregator` collects multiple errors before returning — use for batch validation.
- `EnvLoader` requires the `env` feature flag. Not default.
- `PollingWatcher` is the fallback when native file watching isn't available — it polls on an interval.

## Relations
- Depends on nebula-log (re-exports `info!`, `debug!` etc. in prelude). Used by nebula-api, nebula-runtime, and any crate needing runtime configuration.
