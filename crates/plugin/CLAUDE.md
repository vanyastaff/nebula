# nebula-plugin — Claude Code orientation
> Agent quick-map for `crates/plugin/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** The plugin distribution/registration unit — `Plugin` trait, `ResolvedPlugin`, in-memory `PluginRegistry`, plus the host-side discovery path that registers out-of-process plugin binaries.
**Layer:** Business — depends only downward (root CLAUDE.md -> Layered Dependency Map); consumes `nebula-sandbox`/`nebula-plugin-sdk` (Plugin-Proto) downward.

## Commands
- `cargo check -p nebula-plugin`
- `cargo nextest run -p nebula-plugin`  ·  doctests: `cargo test -p nebula-plugin --doc`
- Derive macro lives in the sibling crate `crates/plugin/macros/` (`nebula-plugin-macros`) — re-exported as `#[derive(Plugin)]`.

## Key files
- `src/lib.rs` — crate facade + public re-exports; module map.
- `src/plugin.rs` — the `Plugin` base trait (`actions()`/`credentials()`/`resources()`/`on_load`/`on_unload`).
- `src/resolved_plugin.rs` — `ResolvedPlugin`: eager component caches; enforces `{plugin.key()}.` namespace invariant + within-plugin dup rejection at construction (ADR-0027).
- `src/registry.rs` — `PluginRegistry`: `PluginKey → Arc<ResolvedPlugin>`; `all_*` / `resolve_*` accessors.
- `src/discovery.rs` — host-side directory scan that probes plugin binaries' `plugin.toml` + wire manifest and registers them.
- `src/sandbox_bridge.rs` — the single `SandboxError` → `ActionError` classification seam (shared by handler + engine runner).
- `src/manifest.rs` — local `PluginManifest` (canonical home is `nebula-metadata`; re-exported for source compat).

## Conventions & never-do
- `impl Plugin` is the single runtime source of truth for what's registered. Do NOT duplicate `fn actions()`/`fn credentials()`/`fn resources()` in `plugin.toml` (spec theater).
- `PluginManifest` does NOT compose `BaseMetadata<K>` — a plugin is a container, not a schematized leaf.
- This crate is NOT the loader/sandbox/isolation, NOT the authoring SDK (`nebula-plugin-sdk`), NOT `plugin.toml` parsing/signing tooling, and NOT a persistent catalog — registry is in-memory only.
- Cross-plugin type references come via `Cargo.toml [dependencies]`; the dep-closure check is enforced by `nebula-sandbox` at activation, not here.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · canon §3.5/§7.1/§13.1 · ADR-0018, ADR-0027 (`docs/adr/HISTORICAL.md`) · `docs/INTEGRATION_MODEL.md` §7.
