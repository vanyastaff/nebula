---
id: 0018
title: plugin-metadata-to-manifest
status: accepted
date: 2026-04-19
supersedes: []
superseded_by: []
tags: [plugin, metadata, canon-3.5]
related:
  - crates/plugin/src/metadata.rs
  - crates/metadata/src/lib.rs
  - docs/PRODUCT_CANON.md#35-integration-model-one-pattern-five-concepts
  - docs/superpowers/specs/2026-04-20-plugin-load-path-stable-design.md
linear: []
---

# ADR-0018 — `PluginMetadata` → `PluginManifest`

## Context

`nebula-plugin::PluginMetadata` was introduced before `nebula-metadata`
existed and still carries the pre-consolidation shape:

- `icon: Option<String>` + `icon_url: Option<String>` — the two-field
  invalid-combination problem that `nebula_metadata::Icon` was introduced
  to solve.
- `version: u32` — conflicts with `semver::Version` used everywhere else
  in Nebula (and with ADR-0007 identifier conventions spirit).
- No `maturity`, no `deprecation` — a plugin can never be marked
  experimental or scheduled for removal.
- `author`, `license`, `homepage`, `repository`, `nebula_version`,
  `group`, `color` — all bundle-level / provenance fields that do not
  apply to leaf entities.

Meanwhile `nebula_metadata::BaseMetadata<K>` is the canonical shape for
catalog citizens (§3.5). It requires `schema: ValidSchema`, which a
plugin — being a **container** for actions / credentials / resources —
does not have: user input lives on the leaves it bundles, not on the
container itself. Forcing a plugin into `BaseMetadata` would require
either making `schema` optional (uglifying every leaf consumer's
accessors) or passing `ValidSchema::empty()` (misleading semantics:
"empty schema" ≠ "no schema applies").

The right fix is not to bend the leaf shape around a container — it is
to give the container its own, honest type.

## Decision

1. Rename `nebula-plugin::PluginMetadata` → `nebula-plugin::PluginManifest`
   (a **bundle descriptor**, not entity metadata).

2. `PluginManifest` **does not compose `BaseMetadata<K>`.** A plugin is
   not a schematized leaf.

3. `PluginManifest` **reuses the small types** from `nebula-metadata`:

   - `Icon` — replaces `icon: Option<String>` + `icon_url: Option<String>`.
   - `MaturityLevel` — new for plugins (experimental / beta / stable /
     deprecated).
   - `DeprecationNotice` — new for plugins.

4. `PluginManifest::version` adopts `semver::Version` (consistency with
   `BaseMetadata::version`).

5. Plugin-specific fields stay on the manifest: `author`, `license`,
   `homepage`, `repository`, `nebula_version`, `group`, `color`,
   `description`, `name`, `key`, `tags`. Builder + `normalize_key`
   behavior is preserved.

## Consequences

**Positive.**
- Manifest stops advertising invalid icon combinations.
- Plugins can declare `MaturityLevel::Experimental` / mark deprecations.
- Semver is used uniformly.
- The conceptual split (container vs. leaf) becomes visible in types.

**Negative.**
- Wire format breaks for any persisted plugin metadata. Scope is small:
  no known production deployments, `nebula-plugin` is `frontier` per
  `docs/MATURITY.md`.
- `nebula-plugin` gains a direct dep on `nebula-metadata` (for `Icon` /
  `MaturityLevel` / `DeprecationNotice`).

**Neutral.**
- `Plugin::metadata() → Plugin::manifest()` rename propagates through
  `nebula-plugin::macros`, `nebula-engine::lib.rs` re-exports, and every
  plugin test fixture.

## Alternatives considered

- **(A) Make `BaseMetadata::schema` optional** (`Option<ValidSchema>`).
  *Rejected:* every leaf consumer — action, credential, resource, and
  any future leaf — would gain an `Option<ValidSchema>` in its accessor
  to support a single non-leaf case.

- **(B) Plugin uses `BaseMetadata` with `ValidSchema::empty()`.**
  *Rejected:* misleading semantics. "Empty schema" implies "this entity
  has a schema, and it happens to have zero fields", not "this entity
  has no schema concept".

- **(C) Split `CatalogInfo<K>` (cosmetic prefix: name / description /
  icon / documentation_url / tags / maturity / deprecation) from
  `BaseMetadata<K>` (= `CatalogInfo<K>` + schema).** *Rejected* for now:
  adds a new abstraction layer for a single non-leaf consumer. Revisit
  only if a second container type (bundle, pack, preset) appears.

- **(D) Leave `PluginMetadata` as-is; extract only the shared small types
  (`Icon`, `MaturityLevel`, `DeprecationNotice`) and adopt them
  field-by-field without renaming the struct or touching its accessors.**
  *Rejected:* the shape problem is not only cosmetic. `version: u32`
  contradicts semver everywhere else in Nebula, `icon` + `icon_url` as
  two `Option<String>` fields is exactly the invalid-state bug `Icon`
  was introduced to fix, and `PluginMetadata` as a *name* now mis-signals
  that a plugin is a catalog-leaf like `ActionMetadata` /
  `CredentialMetadata` / `ResourceMetadata` — which it is not. Piecemeal
  extraction leaves a type whose name lies about its role; the rename to
  `PluginManifest` is the part that prevents the next contributor from
  composing `BaseMetadata` into it on reflex. The ADR scope is
  intentionally wider than "swap the small types".

## Migration plan (executed in a follow-up PR)

1. Introduce `PluginManifest` alongside `PluginMetadata`; mark the old
   type `#[deprecated]`.
2. Rename `Plugin::metadata() → Plugin::manifest()` directly — no shim.
   `nebula-plugin` is `frontier` per `docs/MATURITY.md`; the
   `CLAUDE.md` quick-win trap catalog explicitly discourages shim-naming.
3. Update `nebula-plugin::macros` so `#[plugin]` emits
   `PluginManifest::builder(...)`.
4. Update `nebula-engine` re-exports (`crates/engine/src/lib.rs:70`) and
   `crates/engine/README.md:63`.
5. Delete `PluginMetadata` in the following cycle.

## Follow-ups

- Track migration PR against this ADR (issue created at implementation
  time).
- Revisit alternative (C) when a second container-shape entity arrives.

- **2026-04-20 (slice B of plugin load-path stabilization).**
  `PluginManifest` moved from `nebula-plugin` to `nebula-metadata` so the
  plugin-author SDK can import it without breaking canon §7.1's
  "zero-engine-side-deps" invariant. `nebula-plugin` keeps a thin
  `pub use nebula_metadata::PluginManifest;` re-export for source
  compatibility. See
  [the slice-B design spec](../superpowers/specs/2026-04-20-plugin-load-path-stable-design.md)
  and the forthcoming ADR-0027.
