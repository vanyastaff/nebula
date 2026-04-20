---
name: nebula-plugin
role: Plugin Distribution Unit (registry + manifest re-export; canon §7.1 — unit of registration, not size)
status: partial
last-reviewed: 2026-04-20
canon-invariants: [L1-7.1, L2-7.1, L2-13.1]
related: [nebula-core, nebula-error, nebula-metadata, nebula-action, nebula-resource, nebula-credential, nebula-sandbox, nebula-plugin-sdk]
---

> **Forward-looking notice (as of 2026-04-20).** This README documents the plugin
> registration surface as it lands with **plugin load-path stabilization slice B**
> — `ResolvedPlugin`, `PluginRegistry::all_*` / `resolve_*` accessors, and
> `PluginManifest` moved to `nebula-metadata`. On this branch (and on `main` until
> slice B merges) the source still exports `PluginType` / `PluginVersions` and
> defines `PluginManifest` locally in `crates/plugin/src/manifest.rs`. If the
> README and the code disagree, trust `crates/plugin/src/lib.rs` on the current
> branch; the `status` / MATURITY row will flip from `partial` to `stable` in
> the same merge that replaces the legacy API.

# nebula-plugin

## Purpose

Actions, Resources, and Credentials need a versioned, discoverable distribution unit — one that the engine can load, catalog, and enforce dependency rules against without re-inventing per-integration registration. `nebula-plugin` provides that unit: the `Plugin` trait (returning runnable trait objects per canon §3.5), a `ResolvedPlugin` per-plugin wrapper with eager component indices, and an in-memory `PluginRegistry`. The `PluginManifest` bundle descriptor lives in `nebula-metadata` and is re-exported here for source compatibility. Plugin authors implement `Plugin`, return their actions/credentials/resources from the trait methods, and rely on the engine (via `nebula-sandbox`) to load, version, and dependency-check their crate.

## Role

**Plugin Distribution Unit.** A plugin is the unit of **registration**, not the unit of size — a full integration crate ("Slack plugin" with many actions, credentials, and resources) and a micro-plugin (one resource + one credential) use the exact same contract: `Rust crate + plugin.toml marker + impl Plugin`. See canon §7.1 and `docs/INTEGRATION_MODEL.md`.

## Public API

- `Plugin` — base trait every plugin implements. Methods: `manifest() -> &PluginManifest`, `actions() -> Vec<Arc<dyn Action>>`, `credentials() -> Vec<Arc<dyn AnyCredential>>`, `resources() -> Vec<Arc<dyn AnyResource>>`, `on_load()`, `on_unload()` (default no-ops). Returns the runnable trait objects directly, matching canon §3.5.
- `PluginManifest` — re-exported from `nebula-metadata` (canonical home after ADR-0018 follow-up in slice B of the plugin load-path stabilization). Bundle descriptor with builder API: key, human name, semver version, group, `Icon`, maturity, deprecation, author/license/homepage/repository metadata. Does **not** compose `BaseMetadata<K>` — a plugin is a container, not a schematized leaf. New code should prefer importing from `nebula_metadata`.
- `ResolvedPlugin` — per-plugin wrapper with eager component caches. Constructed via `ResolvedPlugin::from(impl Plugin)`, which calls `actions()` / `credentials()` / `resources()` exactly once, validates the namespace invariant (every component key starts with `{plugin.key()}.`), and catches within-plugin duplicate keys; O(1) `action()` / `credential()` / `resource()` lookups thereafter. See ADR-0027.
- `PluginRegistry` — in-memory `PluginKey → Arc<ResolvedPlugin>` registry. Accessors: `all_actions()` / `all_credentials()` / `all_resources()` flat iterators across every registered plugin; `resolve_action()` / `resolve_credential()` / `resolve_resource()` lookups by full key.
- `ComponentKind` — discriminant for namespace-mismatch and duplicate-component errors.
- `PluginError` — typed error for plugin operations (including `NamespaceMismatch`, `DuplicateComponent`, `AlreadyExists`).
- `PluginKey` — re-exported from `nebula-core`; stable identity type.
- `#[derive(Plugin)]` — proc-macro derivation for `Plugin` boilerplate.

## Contract

- **[L1-§7.1]** Plugin is the unit of **registration**, not the unit of size. Full plugins and micro-plugins use the same contract. No secondary manifest duplicating `fn actions()` / `fn credentials()` / `fn resources()` — `impl Plugin` is the single runtime source of truth for what is registered.
- **[L2-§7.1]** Three sources of truth, no drift: `Cargo.toml` (Rust package identity + dependency graph), `plugin.toml` (trust + compatibility boundary, read without compiling), `impl Plugin + PluginManifest` (runtime registration and bundle metadata). This crate owns the `impl Plugin` surface; `plugin.toml` parsing belongs to tooling.
- **[L2-§13.1]** Plugin load → registry: a plugin loads; Actions / Resources / Credentials from `impl Plugin` appear in the catalog without a second manifest that duplicates `fn actions()` / `fn resources()` / `fn credentials()`. Seam: `PluginRegistry::register(Arc<ResolvedPlugin>)` — construction of `ResolvedPlugin` enforces the `{plugin.key()}.` namespace invariant and rejects within-plugin duplicate keys before the entry reaches the registry. Test: unit tests in `crates/plugin/`.
- **Cross-plugin dependency rule** — types from another plugin come in only via `Cargo.toml [dependencies]` on the provider plugin crate. Referencing a type not in the declared dependency closure is a misconfiguration caught at activation. This rule is enforced by the loader in `nebula-sandbox`, not by this crate directly.

## Non-goals

- Not the plugin loader or sandbox — loading, isolation (`ProcessSandbox`), capability enforcement, OS-level hardening, and cross-plugin dependency activation-time checks live in `nebula-sandbox`.
- Not the plugin authoring SDK — `nebula-plugin-sdk` provides the higher-level authoring surface on top of these traits.
- Not responsible for `plugin.toml` parsing or signature verification — those belong to pre-compile tooling (`cargo-nebula`); see canon §7.1.
- Not a runtime runtime catalog with persistence — this is an in-memory registry; persistence lives in `nebula-storage`.

## Maturity

See `docs/MATURITY.md` row for `nebula-plugin`.

- API stability: `partial` today, lifting to `stable` with slice B — `Plugin` trait, `ResolvedPlugin`, and `PluginRegistry` (including `all_*` / `resolve_*` accessors) are the registration surface frozen by ADR-0027 once the refactor merges. `PluginManifest` is canonical in `nebula-metadata` and re-exported here after the slice B move. Integration paths (loader activation, cross-plugin dependency resolution) live in `nebula-sandbox` and remain `frontier`.
- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]` enforced.
- Signing / trust boundary (`[signing]` in `plugin.toml`): `planned` — not enforced at runtime yet. See canon §7.1 and `docs/INTEGRATION_MODEL.md` signing section.

## Related

- Canon: `docs/PRODUCT_CANON.md` §1 (plugin as integration surface), §3.5 (Plugin = distribution + registration unit, returns runnable trait objects), §7.1 (packaging: `Cargo.toml` + `plugin.toml` + `impl Plugin`; unit of registration not size), §13.1 (plugin load → registry contract).
- ADRs: `docs/adr/0018-plugin-metadata-to-manifest.md` (rename rationale); ADR-0027 (`ResolvedPlugin`, namespace invariant, registry accessors) lands with slice B — file will live at `docs/adr/0027-plugin-load-path-stable.md`.
- Integration model: `docs/INTEGRATION_MODEL.md` §7 — full plugin packaging mechanics, three-sources-of-truth rule, cross-plugin dependency rule, signing rationale, discovery / load lifecycle, ABI policy, tooling notes.
- Siblings: `nebula-metadata` (canonical `PluginManifest`), `nebula-plugin-sdk` (authoring SDK on top of these traits), `nebula-sandbox` (loading and isolation), `nebula-core` (`PluginKey` identity type), `nebula-action` (`Action` trait), `nebula-resource` (`AnyResource` trait), `nebula-credential` (`AnyCredential` trait).
