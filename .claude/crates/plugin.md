# nebula-plugin
Plugin packaging unit — the user-visible integration bundle (e.g., Slack, HTTP Request, PostgreSQL).

## Invariants
- A plugin is a packaging/metadata unit only. Actions, resources, and credentials are registered **separately** in nebula-engine, not inside the plugin struct.
- `PluginRegistry` is in-memory. It maps `PluginKey → PluginType`. Populated at startup.

## Key Decisions
- `PluginType` wraps either a single `Plugin` or a `PluginVersions` multi-version set (for backward compatibility).
- `PluginVersions` keyed by `u32` version number.
- Dynamic loading (`PluginLoader`) is feature-gated behind `dynamic-loading` — uses unsafe FFI. The module `loader` uses `allow(unsafe_code)`.

## Traps
- The `Plugin` trait itself only provides metadata (`PluginMetadata`). Don't expect it to register actions — that's done by the engine consumer.
- Dynamic loading requires ABI stability between the host and the loaded library. This is fragile; prefer in-process plugins.

## Relations
- Depends on nebula-core (re-exports `PluginKey`). Used by nebula-engine (re-exports `Plugin`, `PluginMetadata`, etc.), nebula-sdk.

<!-- reviewed: 2026-04-01 — Plugin derive moved to nebula-plugin-macros, re-exported from crate root -->

<!-- reviewed: 2026-04-02 -->

<!-- reviewed: 2026-04-02 — dep cleanup only: removed unused Cargo.toml deps via cargo shear --fix, no code changes -->
