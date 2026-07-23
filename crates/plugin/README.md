---
name: nebula-plugin
role: Plugin Distribution Unit (registry + manifest re-export; canon §7.1 — unit of registration, not size)
status: partial
last-reviewed: 2026-07-23
canon-invariants: [L1-7.1, L2-7.1, L2-13.1]
related: [nebula-core, nebula-error, nebula-metadata, nebula-action, nebula-resource, nebula-credential]
---

# nebula-plugin

## Purpose

Actions, Resources, and Credentials need a versioned distribution unit — one that the engine can catalog without re-inventing per-integration registration. `nebula-plugin` provides that unit: the `Plugin` trait (returning runnable trait objects per canon §3.5), a `ResolvedPlugin` per-plugin wrapper with eager component indices, and an in-memory `PluginRegistry`. The `PluginManifest` bundle descriptor lives in `nebula-metadata` and is re-exported here for source compatibility. Plugin authors implement `Plugin` in Rust, return their actions/credentials/resources from the trait methods, and register them **in-process**; the Rust compiler enforces the cross-plugin dependency closure at link time (ADR-0091 — out-of-process execution retired).

## Role

**Plugin Distribution Unit.** A plugin is the unit of **registration**, not the unit of size — a full integration crate ("Slack plugin" with many actions, credentials, and resources) and a micro-plugin (one resource + one credential) use the exact same contract: `Rust crate + plugin.toml marker + impl Plugin`. See canon §7.1 and `docs/INTEGRATION_MODEL.md`.

## Public API

- `Plugin` — base trait every plugin implements. Methods: `manifest() -> &PluginManifest`, `actions() -> Vec<Arc<dyn Action>>`, `credentials() -> Vec<Arc<dyn AnyCredential>>`, `resources() -> Vec<Arc<dyn AnyResource>>`, `on_load()`, `on_unload()` (default no-ops). Returns the runnable trait objects directly, matching canon §3.5.
- `PluginManifest` — re-exported from `nebula-metadata` (canonical home after ADR-0018 follow-up in slice B of the plugin load-path stabilization). Bundle descriptor with builder API: key, human name, semver version, group, `Icon`, maturity, deprecation, author/license/homepage/repository metadata. Does **not** compose `BaseMetadata<K>` — a plugin is a container, not a schematized leaf. New code should prefer importing from `nebula_metadata`.
- `ResolvedPlugin` — per-plugin wrapper with eager component caches. Constructed via `ResolvedPlugin::from(impl Plugin)`, which calls `actions()` / `credentials()` / `resources()` exactly once, validates the namespace invariant (every component key starts with `{plugin.key()}.`), and catches within-plugin duplicate keys; O(1) `action()` / `credential()` / `resource()` lookups thereafter. See ADR-0027.
- `PluginRegistry` — in-memory `PluginKey → Arc<ResolvedPlugin>` registry. Accessors: `all_actions()` / `all_credentials()` / `all_resources()` flat iterators across every registered plugin; `resolve_action()` / `resolve_credential()` / `resolve_resource()` lookups by full key.
- `PluginRegistry::freeze` — consuming activation-foundation boundary. It rejects empty or invalid dependency graphs and returns a mutation-free `FrozenPluginRegistry` with the same read API, deterministic dependency order, a canonical `PluginSet`, and an immutable `WorkerFlavorRevision`.
- `PluginSet` / `PluginContractDescriptor` — normalized registered-surface descriptor. Identity includes sorted plugin keys, component keys, dependency keys and normalized semver requirements; prerelease is logical identity while build metadata is excluded.
- `WorkerFlavorRevision` — combines the logical plugin-set identity with trusted artifact-set provenance and the logical runtime contract version.
- `WorkerFlavorContext::from_registry` — derives a canonically ordered execution-facing view from a successfully frozen registry.
- `ComponentKind` — discriminant for namespace-mismatch and duplicate-component errors.
- `PluginError` — typed error for plugin operations (including `NamespaceMismatch`, `DuplicateComponent`, `AlreadyExists`).
- `PluginKey` — re-exported from `nebula-core`; stable identity type.
- `#[derive(Plugin)]` — proc-macro derivation for `Plugin` boilerplate.

## Contract

- **[L1-§7.1]** Plugin is the unit of **registration**, not the unit of size. Full plugins and micro-plugins use the same contract. No secondary manifest duplicating `fn actions()` / `fn credentials()` / `fn resources()` — `impl Plugin` is the single runtime source of truth for what is registered.
- **[L2-§7.1]** Three sources of truth, no drift: `Cargo.toml` (Rust package identity + dependency graph), `plugin.toml` (trust + compatibility boundary, read without compiling), `impl Plugin + PluginManifest` (runtime registration and bundle metadata). This crate owns the `impl Plugin` surface; `plugin.toml` parsing belongs to tooling.
- **[L2-§13.1]** Plugin load → registry: a plugin loads; Actions / Resources / Credentials from `impl Plugin` appear in the catalog without a second manifest that duplicates `fn actions()` / `fn resources()` / `fn credentials()`. Seam: `PluginRegistry::register(Arc<ResolvedPlugin>)` — construction of `ResolvedPlugin` enforces the `{plugin.key()}.` namespace invariant and rejects within-plugin duplicate keys before the entry reaches the registry. Test: unit tests in `crates/plugin/`.
- **Cross-plugin dependency rule** — types from another plugin come in only via `Cargo.toml [dependencies]` on the provider plugin crate. In the in-process model the Rust compiler enforces this at link time: a type not in the declared dependency closure does not resolve.

## Immutable activation foundation (staged)

This crate now contains the ADR-0115 foundation for immutable worker-flavor
identity. `PluginRegistry::freeze` consumes the mutable assembly registry,
re-validates its dependency graph, and derives:

1. a canonical `PluginSetId` from logical plugin versions, registered component
   keys, and declared dependency contracts; and
2. a `WorkerFlavorRevisionId` that additionally binds the runtime contract
   version and artifact-set digest.

The encoding is domain-separated and length-framed. Registration order,
component order, dependency order, and comparator order do not affect the
result; exact duplicate dependency declarations and comparators are collapsed.
Parsed semantic versions are encoded structurally with fixed operator tags and
big-endian numeric fields, so the v1 fingerprint does not depend on a
dependency crate's display formatting. Semver prerelease data remains part of
logical identity; build metadata is artifact provenance and is excluded from
logical versions.

This is deliberately a foundation slice. It does **not** claim that engine
dispatch, API transport, queue persistence, or exact-flavor routing have
already migrated to `FrozenPluginRegistry`; those consumers move in later
ADR-0115 slices. Until then, the mutable registry remains available to existing
composition code.

The composition root must supply `ArtifactSetDigest` and
`RuntimeContractVersion` from trusted activation state. Hash derivation does
not authenticate caller-provided bytes. Likewise, `PluginSetId` is registered
surface identity and audit metadata—not proof of schema compatibility,
capability possession, artifact authenticity, or execution authorization.

## Non-goals

- Not process/WASM isolation — out-of-process plugin execution was retired (ADR-0091, canon §12.6). Plugins run in-process as trusted code; process / OS / WASM isolation is a non-goal, not a deferred 1.0 capability.
- Not responsible for `plugin.toml` parsing or signature verification — those belong to pre-compile tooling (`cargo-nebula`); see canon §7.1.
- Not a runtime runtime catalog with persistence — this is an in-memory registry; persistence lives in `nebula-storage`.

## Maturity

See `docs/MATURITY.md` row for `nebula-plugin`.

- API stability: `partial`. `Plugin`, `ResolvedPlugin`, and the mutable/frozen registry read surface are implemented; engine/API/persistence adoption of exact worker-flavor revisions remains staged. `PluginManifest` is canonical in `nebula-metadata` and re-exported here. Cross-plugin type resolution remains the Rust compiler's job (in-process link-time closure, ADR-0091).
- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]` enforced.
- Signing / trust boundary (`[signing]` in `plugin.toml`): `planned` — not enforced at runtime yet. See canon §7.1 and `docs/INTEGRATION_MODEL.md` signing section.

## Related

- Canon: `docs/PRODUCT_CANON.md` §1 (plugin as integration surface), §3.5 (Plugin = distribution + registration unit, returns runnable trait objects), §7.1 (packaging: `Cargo.toml` + `plugin.toml` + `impl Plugin`; unit of registration not size), §13.1 (plugin load → registry contract).
- ADRs: ADR-0018 (rename rationale), ADR-0027 (`ResolvedPlugin`, namespace invariant, registry accessors) — historical, indexed in the maintainers' private design vault.
- Integration model: `docs/INTEGRATION_MODEL.md` §7 — full plugin packaging mechanics, three-sources-of-truth rule, cross-plugin dependency rule, signing rationale, discovery / load lifecycle, ABI policy, tooling notes.
- Siblings: `nebula-metadata` (canonical `PluginManifest`), `nebula-core` (`PluginKey` identity type), `nebula-action` (`Action` trait), `nebula-resource` (`AnyResource` trait), `nebula-credential` (`AnyCredential` trait).
