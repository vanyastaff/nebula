# Architecture

## Problem Statement

- Business problem:
  - Nebula crates need a consistent way to load, validate, and refresh runtime configuration across environments.
- Technical problem:
  - avoid per-crate ad-hoc config logic, inconsistent precedence rules, and unsafe runtime overrides.

## Current Architecture

### Module Map

| Module | File(s) | Responsibility |
|--------|---------|----------------|
| `core/config` | `core/config.rs` | `Config` struct; `Arc<RwLock<Value>>` storage; dot-path traversal; merge; reload |
| `core/builder` | `core/builder.rs` | `ConfigBuilder`; source priority sort; concurrent load; validator gate; hot-reload/auto-reload spawn |
| `core/source` | `core/source.rs` | `ConfigSource` (11 variants, priority, optional flag); `ConfigFormat`; `SourceMetadata` builder |
| `core/traits` | `core/traits.rs` | `ConfigLoader`, `ConfigValidator`, `ConfigWatcher`; blanket impl bridging `nebula-validator` |
| `core/error` | `core/error.rs` | `ConfigError` (15 variants, `#[non_exhaustive]`); `ErrorCategory`; `ContractErrorCategory`; `From` impls |
| `core/result` | `core/result.rs` | `ConfigResult<T>`; `ConfigResultExt`; `ConfigResultAggregator`; `try_sources` |
| `loaders/` | `loaders/*.rs` | `FileLoader` (JSON/TOML/YAML/INI/HCL/Properties/env); `EnvLoader`; `CompositeLoader` |
| `watchers/` | `watchers/*.rs` | `FileWatcher` (notify); `PollingWatcher`; `NoOpWatcher`; `ConfigWatchEvent`/`ConfigWatchEventType` |
| `builders` | `lib.rs` | Convenience factory fns: `from_file`, `from_env`, `standard_app_config`, `with_hot_reload` |
| `utils` | `lib.rs` | `check_config_file`, `merge_json_values`, `parse_config_string` |

### Data/Control Flow

```
ConfigBuilder
  ├── with_defaults_json()   →  in-memory Value (priority 100)
  ├── with_source()          →  source list (sorted by priority ascending on build)
  ├── with_validator()       →  Arc<dyn ConfigValidator>
  ├── with_watcher()         →  Arc<dyn ConfigWatcher>
  └── build() ─────────────────────────────────────────────────────────────
        │
        ├── join_all: loader.load(source) for each non-Default source ──→ Vec<Value>
        │
        ├── merge in priority order (deep-merge objects, replace scalars/arrays)
        │
        ├── validator.validate(&merged) ──→ Err blocks activation
        │
        ├── Config::new(merged, sources, defaults, ...) stored in Arc<RwLock<Value>>
        │
        ├── watcher.start_watching() [if hot_reload]
        │
        └── tokio::spawn(auto-reload loop) [if auto_reload_interval]

Config::reload()
  ├── start from defaults.clone()
  ├── join_all: loader.load(source) for each non-Default source
  ├── merge in priority order
  ├── validator.validate() ──→ Err: preserve previous state
  └── RwLock::write() → atomic swap
```

### Key Internal Invariants

- **`Config` is `Clone`:** All mutable state is `Arc`-wrapped (`Arc<RwLock<Value>>`, `Arc<DashMap<...>>`). Clones share the same live state.
- **Source priority is stable:** Sources sorted at `build()` time; never re-sorted on reload. Insertion order is tiebreaker.
- **Defaults are re-applied on every reload:** Captured at `build()`, used as merge base each `reload()`. Optional sources do not erase defaults.
- **Validator is atomic gate:** Failure during `build()` aborts construction; failure during `reload()` preserves previous state.
- **Auto-reload uses `CancellationToken`:** Spawned task holds an `Arc<Config>` clone. `Config::drop` calls `cancel_token.cancel()`, ensuring the task stops even if the original `Config` is dropped.
- **`nebula-validator` blanket impl:** Any `T: Validate<Value> + Send + Sync` automatically implements `ConfigValidator` via the bridge in `traits.rs`. Maps `ValidationError` → `ConfigError::ValidationError`.
- **`ConfigError` is `#[non_exhaustive]`:** Match arms must have a wildcard; new variants are non-breaking at the source level.
- **Path traversal is dot-notation:** Objects use string keys; arrays use numeric string indices. Creates intermediate objects on `set_value`.
- **known bottlenecks:**
  - Large nested config merges under frequent reloads (recursive `merge_json`).
  - User-defined validation complexity in hot paths.

## Target Architecture

- target module map:
  - keep current split; improve contract docs and test rigor before structural refactor
- public contract boundaries:
  - `ConfigBuilder` for assembly
  - `Config` for runtime access and mutation
  - `ConfigLoader`/`ConfigValidator`/`ConfigWatcher` as extension points
- internal invariants:
  - source precedence must remain deterministic
  - merge must be idempotent for same input set
  - reload must either fully succeed or preserve previous valid state

## Design Reasoning

- trade-off 1: flexibility vs determinism
  - chosen: flexible source types with explicit priority ordering.
- trade-off 2: dynamic JSON tree vs compile-time config structs
  - chosen: dynamic core storage + typed retrieval bridges.
- rejected alternatives:
  - compile-time-only typed config tree as single model (too rigid for plugin/runtime extensibility).

## Comparative Analysis

References: n8n, Node-RED, Activepieces/Activeflow, Temporal/Airflow ecosystem practices.

- Adopt:
  - layered source precedence (defaults + file + env overrides), common in automation platforms.
  - hot-reload/watcher semantics for long-running orchestrators.
- Reject:
  - implicit/undocumented precedence rules (causes production misconfiguration).
  - silently coercing invalid values without explicit errors.
- Defer:
  - remote config source first-class support (`Remote/Database/KeyValue`) until reliability/security model is hardened.

## Breaking Changes (if any)

- none now.
- future candidates:
  - stronger typed path model
  - stricter merge conflict policy options

## Open Questions

- should reload support transactional staging hooks per consumer crate?
- should source priority become fully user-configurable at runtime?
