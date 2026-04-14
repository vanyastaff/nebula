# nebula-config
Multi-source configuration with env interpolation, hot-reload, and typed access.

## Invariants
- `ConfigBuilder::build()` is **async**; sources are merged last-wins.
- Hot-reload pipeline: `FileWatcher` → bounded `mpsc<ReloadTrigger>(64)` → 250 ms-debounced reload task → `Config::reload()`. Builder owns channel creation; task is reclaimed when `Config::drop` fires `cancel_token`.

## Key Decisions
- Env interpolation: `${VAR}` / `${VAR:-default}`, evaluated once at load time.
- Formats: TOML, JSON; YAML behind `yaml` feature.
- Typed access: `config.get::<T>("a.b.1.name")` — arrays indexed by position.
- `FileWatcher` notify callback uses `try_send` (never `blocking_send`); overflow is counted in `dropped_events()` and warned at power-of-two intervals.

## Traps
- Path errors are descriptive, not panics. Empty segments (`.a`, `a.`, `a..b`) → `PathError`.
- `EnvLoader` requires the `env` feature flag.
- `PollingWatcher` is the fallback for environments without native file watching.
- **`interpolate` takes `Value` by value** — `merged_data` is moved in `builder.rs`.
- **`reload()` does NOT re-interpolate** — `${VAR}` references resolve only at initial build. Pre-existing bug, tracked separately.
- **`FileWatcher::start_watching` claims the slot via `compare_exchange` (#294)** — concurrent calls produce exactly one Ok, the rest `Err("Already watching")`. Setup failure after the claim unwinds via RAII guard so retries work.
- **`Config::start_watching` is `#[deprecated]`** — only starts the raw watcher, does NOT wire events to `reload()`. Use `with_hot_reload(true)` instead (#313).
- **User-supplied `with_watcher(...)` does NOT auto-bridge to the reload pipeline** — only the default `FileWatcher` installed by `with_hot_reload(true)` forwards events into the builder's `ReloadTrigger` channel.

## Relations
- Depends on nebula-log. Used by nebula-api, nebula-runtime, anything needing runtime config.

<!-- reviewed: 2026-04-14 -->
