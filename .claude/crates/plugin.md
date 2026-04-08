# nebula-plugin
Plugin packaging unit — the user-visible integration bundle (e.g., Slack, HTTP Request, PostgreSQL).

## Invariants
- Plugin is a packaging/metadata unit only. Actions/credentials/resources registered separately in nebula-engine.
- `PluginRegistry`: in-memory `PluginKey → PluginType`. Populated at startup.
- `Plugin` trait: all methods except `metadata()` have default impls — existing impls need no changes.
- `PluginMetadata` new optional fields (v2): `author`, `license`, `homepage`, `repository`, `nebula_version` — all `None` by default, skipped in JSON when `None`.

## Key Decisions
- `PluginType` wraps a single `Plugin` or `PluginVersions` (multi-version set keyed by `u32`).
- Dynamic loading (`PluginLoader`) feature-gated behind `dynamic-loading` — uses unsafe FFI.

## Traps
- Descriptor methods (`actions`, `credentials`, `resources`) return lightweight key/name/description structs only — no handler factories. Adding a factory would require `nebula-action` dep, violating peer-layer constraint.
- `on_load`/`on_unload` are sync. Async init must happen outside the trait.
- Dynamic loading requires ABI stability — fragile; prefer in-process plugins.

## Relations
- Depends on nebula-core (re-exports `PluginKey`). Used by nebula-engine, nebula-sdk.

<!-- reviewed: 2026-04-01 — Plugin derive moved to nebula-plugin-macros, re-exported from crate root -->

<!-- reviewed: 2026-04-02 -->

<!-- reviewed: 2026-04-02 — dep cleanup only: removed unused Cargo.toml deps via cargo shear --fix, no code changes -->

<!-- reviewed: 2026-04-07 — PR #232 doc fixes: wording, lib.rs crate docs, JSON test robustness, # Examples on new builder/trait methods -->
