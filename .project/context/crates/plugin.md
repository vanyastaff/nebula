# nebula-plugin
Plugin packaging unit — the user-visible integration bundle (e.g., Slack, HTTP Request, PostgreSQL).

## Invariants
- Plugin is a packaging/metadata unit only. Actions/credentials/resources registered separately in nebula-engine.
- `PluginRegistry`: in-memory `PluginKey → PluginType`. Populated at startup.
- `Plugin` trait: all methods except `metadata()` have default impls — existing impls need no changes.
- **No loading, no I/O, no unsafe.** Plugin loading moved to `nebula-sandbox`. This crate is pure metadata/traits.

## Key Decisions
- `PluginType` wraps a single `Plugin` or `PluginVersions` (multi-version set keyed by `u32`).
- `PluginLoader` (libloading) **removed** — segfaulted due to vtable ABI mismatch. WASM loading planned in `nebula-sandbox`.
- `dynamic-loading` feature **removed** along with `libloading` dep.

## Traps
- Descriptor methods (`actions`, `credentials`, `resources`) return lightweight key/name/description structs only — no handler factories.
- `on_load`/`on_unload` are sync. Async init must happen outside the trait.

## Relations
- Depends on nebula-core (re-exports `PluginKey`). Used by nebula-engine, nebula-sdk.

<!-- reviewed: 2026-04-08 — removed loader.rs, libloading, dynamic-loading feature. Loading moved to nebula-sandbox. -->

<!-- reviewed: 2026-04-11 — Workspace-wide nightly rustfmt pass applied (group_imports = "StdExternalCrate", imports_granularity = "Crate", wrap_comments, format_code_in_doc_comments). Touches every Rust file in the crate; purely formatting, zero behavior change. -->
