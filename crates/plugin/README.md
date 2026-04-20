---
name: nebula-plugin
role: Plugin Distribution Unit (registry + metadata; canon §7.1 — unit of registration, not size)
status: partial
last-reviewed: 2026-04-17
canon-invariants: [L1-7.1, L2-7.1, L2-13.1]
related: [nebula-core, nebula-error, nebula-action, nebula-resource, nebula-credential, nebula-sandbox, nebula-plugin-sdk]
---

# nebula-plugin

## Purpose

Actions, Resources, and Credentials need a versioned, discoverable distribution unit — one that the engine can load, catalog, and enforce dependency rules against without re-inventing per-integration registration. `nebula-plugin` provides that unit: the `Plugin` trait, `PluginMetadata` static descriptor, lightweight per-concept descriptors, and an in-memory registry. Plugin authors implement `Plugin`, return their actions/credentials/resources from the trait methods, and rely on the engine (via `nebula-sandbox`) to load, version, and dependency-check their crate.

## Role

**Plugin Distribution Unit.** A plugin is the unit of **registration**, not the unit of size — a full integration crate ("Slack plugin" with many actions, credentials, and resources) and a micro-plugin (one resource + one credential) use the exact same contract: `Rust crate + plugin.toml marker + impl Plugin`. See canon §7.1 and `docs/INTEGRATION_MODEL.md`.

## Public API

- `Plugin` — base trait every plugin implements. Methods: `metadata() -> PluginMetadata`, `actions() -> Vec<ActionDescriptor>`, `credentials() -> Vec<CredentialDescriptor>`, `resources() -> Vec<ResourceDescriptor>`, `on_load()`, `on_unload()` (default no-ops).
- `PluginMetadata` — static descriptor with builder API: key, human name, version, group, icon, docs URL.
- `ActionDescriptor`, `CredentialDescriptor`, `ResourceDescriptor` — lightweight descriptors returned by `Plugin` trait methods; the engine uses these for catalog and cross-plugin validation.
- `PluginType` — enum wrapping a single `Plugin` or a `PluginVersions` set.
- `PluginVersions` — multi-version container keyed by `u32`.
- `PluginRegistry` — in-memory `PluginKey → PluginType` registry.
- `PluginError` — typed error for plugin operations.
- `PluginKey` — re-exported from `nebula-core`; stable identity type.
- `#[derive(Plugin)]` — proc-macro derivation for `Plugin` boilerplate.

## Contract

- **[L1-§7.1]** Plugin is the unit of **registration**, not the unit of size. Full plugins and micro-plugins use the same contract. No secondary manifest duplicating `fn actions()` / `fn credentials()` / `fn resources()` — `impl Plugin` is the single runtime source of truth for what is registered.
- **[L2-§7.1]** Three sources of truth, no drift: `Cargo.toml` (Rust package identity + dependency graph), `plugin.toml` (trust + compatibility boundary, read without compiling), `impl Plugin + PluginMetadata` (runtime registration and display metadata). This crate owns the `impl Plugin` surface; `plugin.toml` parsing belongs to tooling.
- **[L2-§13.1]** Plugin load → registry: a plugin loads; Actions / Resources / Credentials from `impl Plugin` appear in the catalog without a second manifest that duplicates `fn actions()` / `fn resources()` / `fn credentials()`. Seam: `PluginRegistry::register`. Test: unit tests in `crates/plugin/`.
- **Cross-plugin dependency rule** — types from another plugin come in only via `Cargo.toml [dependencies]` on the provider plugin crate. Referencing a type not in the declared dependency closure is a misconfiguration caught at activation. This rule is enforced by the loader in `nebula-sandbox`, not by this crate directly.

## Non-goals

- Not the plugin loader or sandbox — loading, isolation (`ProcessSandbox`), capability enforcement, OS-level hardening, and cross-plugin dependency activation-time checks live in `nebula-sandbox`.
- Not the plugin authoring SDK — `nebula-plugin-sdk` provides the higher-level authoring surface on top of these traits.
- Not responsible for `plugin.toml` parsing or signature verification — those belong to pre-compile tooling (`cargo-nebula`); see canon §7.1.
- Not a runtime runtime catalog with persistence — this is an in-memory registry; persistence lives in `nebula-storage`.

## Maturity

See `docs/MATURITY.md` row for `nebula-plugin`.

- API stability: `partial` — trait, metadata, descriptors, registry, and `PluginVersions` are implemented and stable as the registration surface; integration paths (loader activation, cross-plugin dependency resolution) are in `nebula-sandbox` and remain `frontier`.
- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]` enforced.
- 7 unit test markers, 0 integration tests.
- `PluginType::Versioned(PluginVersions)` is the only consumer of `plugin_type.rs` + `versions.rs` — these two modules could potentially merge; not a smell, but review when touching either.
- Signing / trust boundary (`[signing]` in `plugin.toml`): `planned` — not enforced at runtime yet. See canon §7.1 and `docs/INTEGRATION_MODEL.md` signing section.

## Related

- Canon: `docs/PRODUCT_CANON.md` §1 (plugin as integration surface), §3.5 (Plugin = distribution + registration unit), §7.1 (packaging: `Cargo.toml` + `plugin.toml` + `impl Plugin`; unit of registration not size), §13.1 (plugin load → registry contract).
- Integration model: `docs/INTEGRATION_MODEL.md` §7 — full plugin packaging mechanics, three-sources-of-truth rule, cross-plugin dependency rule, signing rationale, discovery / load lifecycle, ABI policy, tooling notes.
- Siblings: `nebula-plugin-sdk` (authoring SDK on top of these traits), `nebula-sandbox` (loading and isolation), `nebula-core` (`PluginKey` identity type), `nebula-action` (`ActionDescriptor`), `nebula-resource` (`ResourceDescriptor`), `nebula-credential` (`CredentialDescriptor`).
