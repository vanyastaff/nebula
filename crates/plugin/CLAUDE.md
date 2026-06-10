# nebula-plugin — Claude Code orientation
> Agent quick-map for `crates/plugin/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** The in-process plugin distribution/registration unit — `Plugin` trait, `ResolvedPlugin`, in-memory `PluginRegistry`. A human implements `Plugin` in Rust and registers its actions / credentials / resources; the engine dispatches them in-process (ADR-0091).
**Layer:** Business — depends only downward (root CLAUDE.md -> Layered Dependency Map).

## Commands
- `cargo check -p nebula-plugin`
- `cargo nextest run -p nebula-plugin`  ·  doctests: `cargo test -p nebula-plugin --doc`
- Derive macro lives in the sibling crate `crates/plugin/macros/` (`nebula-plugin-macros`) — re-exported as `#[derive(Plugin)]`.

## Key files
- `src/lib.rs` — crate facade + public re-exports; module map.
- `src/plugin.rs` — the `Plugin` base trait (`actions()`/`credentials()`/`resources()`/`on_load`/`on_unload`).
- `src/resolved_plugin.rs` — `ResolvedPlugin`: eager component caches; enforces `{plugin.key()}.` namespace invariant + within-plugin dup rejection at construction (ADR-0027).
- `src/registry.rs` — `PluginRegistry`: `PluginKey → Arc<ResolvedPlugin>`; `all_*` / `resolve_*` accessors.
- `src/manifest.rs` — local `PluginManifest` (canonical home is `nebula-metadata`; re-exported for source compat).

## Conventions & never-do
- `impl Plugin` is the single runtime source of truth for what's registered. Do NOT duplicate `fn actions()`/`fn credentials()`/`fn resources()` in `plugin.toml` (spec theater).
- `PluginManifest` does NOT compose `BaseMetadata<K>` — a plugin is a container, not a schematized leaf.
- This crate is NOT `plugin.toml` parsing/signing tooling and NOT a persistent catalog — registry is in-memory only. Process/WASM isolation is a non-goal (ADR-0091, canon §12.6).
- Cross-plugin type references come via `Cargo.toml [dependencies]` — the Rust compiler enforces the dependency closure at link time (in-process model).
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · canon §3.5/§7.1/§13.1 · ADR-0018, ADR-0027 (`docs/adr/HISTORICAL.md`) · `docs/INTEGRATION_MODEL.md` §7.
