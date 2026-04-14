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
- Invalid array index (`arr.x`) and out-of-bounds (`arr.5.name`) return descriptive errors — they don't panic.
- Empty path segments are rejected: leading dot (`.a`), trailing dot (`a.`), and consecutive dots (`a..b`) all return `PathError`.
- `ConfigResultAggregator` collects multiple errors before returning — use for batch validation.
- `EnvLoader` requires the `env` feature flag. Not default.
- `PollingWatcher` is the fallback when native file watching isn't available — it polls on an interval.
- **`interpolate` takes `Value` by value** (not `&Value`) — callers must pass ownership. In `builder.rs` this means `merged_data` is moved into `interpolate(merged_data)`.
- **`reload()` does NOT re-interpolate** — env var references (`${VAR}`) in config sources are NOT re-resolved on hot-reload. Pre-existing bug, tracked separately.
- **`FileWatcher::start_watching` claims the slot via `compare_exchange` (#294)** — concurrent calls produce exactly one Ok, the rest `Err("Already watching")`. Setup failure after the claim unwinds via RAII guard so retries work; the event processor is the lifecycle owner for clearing `watching` on loop exit.
- **`Config::start_watching` is `#[deprecated]`** — only starts the raw watcher, does NOT wire events to `reload()`. Use `with_hot_reload(true)` instead (#313).
- **User-supplied `with_watcher(...)` does NOT auto-bridge to the reload pipeline** — only the default `FileWatcher` installed by `with_hot_reload(true)` forwards events into the builder's `ReloadTrigger` channel.

## Relations
- Depends on nebula-log. Used by nebula-api, nebula-runtime, anything needing runtime config.

<!-- reviewed: 2026-04-14 -->
