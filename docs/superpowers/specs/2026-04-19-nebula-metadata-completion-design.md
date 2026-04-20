---
title: nebula-metadata completion + plugin-manifest ADR
status: proposed
date: 2026-04-19
related:
  - crates/metadata/src/lib.rs
  - crates/metadata/src/compat.rs
  - crates/action/src/metadata.rs
  - crates/credential/src/metadata.rs
  - crates/resource/src/resource.rs
  - crates/plugin/src/metadata.rs
  - docs/MATURITY.md
  - docs/PRODUCT_CANON.md#35-integration-model-one-pattern-five-concepts
---

# nebula-metadata — completion + plugin-manifest ADR

## Context

`nebula-metadata` owns the shared shape of Nebula's catalog citizens
(`BaseMetadata<K>`, `Metadata` trait, `Icon`, `MaturityLevel`,
`DeprecationNotice`). Three consumers compose it today:
`nebula-action::ActionMetadata`, `nebula-credential::CredentialMetadata`,
`nebula-resource::ResourceMetadata`.

Three gaps block declaring the crate "complete":

1. **`compat.rs` is a stub.** The module reserves the name but delegates all
   rules to `ActionMetadata::validate_compatibility`. Key-immutability,
   version-monotonicity, and breaking-change-requires-major-bump are generic
   rules that belong on `BaseMetadata`, yet credential and resource today have
   **no** compat validation at all. The duplication risk is real: whoever adds
   it next will copy the action implementation and drift.

2. **No `README.md`, no row in `docs/MATURITY.md`.** Canon §17 definition of
   done requires both for any crate that a consumer depends on. Three
   consumers already do.

3. **`nebula-plugin::PluginMetadata` does not compose `BaseMetadata`.** It
   reinvents `key / name / description / icon / icon_url / documentation_url /
   tags / version / group / color`, invalid combinations included (both
   `icon` and `icon_url` can be set simultaneously — the exact problem
   `Icon` enum was introduced to solve). This is not a `nebula-metadata` bug;
   it is a `nebula-plugin` design issue. The correct resolution is renaming
   `PluginMetadata` → `PluginManifest` and reshaping it as a **bundle
   descriptor**, not a schematized-leaf metadata. That decision lands as an
   ADR; implementation is out of scope for this spec.

## Goals

1. Extract generic compat rules from `ActionMetadata` into
   `nebula-metadata::compat`; reuse in action / credential / resource.
2. Add `crates/metadata/README.md` and a row in `docs/MATURITY.md`.
3. File ADR-0018 capturing the `PluginMetadata` → `PluginManifest` decision,
   the small-types reuse (`Icon`, `MaturityLevel`, `DeprecationNotice`), and
   an explicit non-reuse of `BaseMetadata<K>` (with reasoning).

## Non-goals

- Reshaping `nebula-plugin`. The ADR documents the decision; code lands in a
  later session.
- Changing the wire format of existing `BaseMetadata<K>` serialization.
- Adding new optional fields to `BaseMetadata` (author/license/homepage are a
  plugin-manifest concern, not a leaf-entity concern).
- Builder helpers beyond what the crate already exposes.

## Artifact 1 — `crates/metadata/src/compat.rs`

### New public types

```rust
// Error enum generic over the key type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BaseCompatError<K>
where
    K: std::fmt::Debug + std::fmt::Display + Clone + PartialEq + Eq,
{
    #[error("metadata key changed from `{previous}` to `{current}`")]
    KeyChanged { previous: K, current: K },

    #[error("interface version regressed from {previous} to {current}")]
    VersionRegressed {
        previous: semver::Version,
        current: semver::Version,
    },

    #[error("breaking schema change detected without a major version bump")]
    SchemaChangeWithoutMajorBump,
}

/// Validate the generic, entity-agnostic compatibility rules between two
/// revisions of the same catalog citizen.
///
/// Entity-specific rules (ports, auth pattern, etc.) must be layered on top
/// by the concrete metadata type's own `validate_compatibility`.
pub fn validate_base_compat<K>(
    current: &BaseMetadata<K>,
    previous: &BaseMetadata<K>,
) -> Result<(), BaseCompatError<K>>
where
    K: std::fmt::Debug + std::fmt::Display + Clone + PartialEq + Eq,
{
    if current.key != previous.key {
        return Err(BaseCompatError::KeyChanged {
            previous: previous.key.clone(),
            current: current.key.clone(),
        });
    }
    if current.version < previous.version {
        return Err(BaseCompatError::VersionRegressed {
            previous: previous.version.clone(),
            current: current.version.clone(),
        });
    }
    if current.schema != previous.schema
        && current.version.major == previous.version.major
    {
        return Err(BaseCompatError::SchemaChangeWithoutMajorBump);
    }
    Ok(())
}
```

Key-type bounds: `ActionKey`, `CredentialKey`, `ResourceKey` all satisfy
`Debug + Display + Clone + PartialEq + Eq` today — the generic bound does not
force changes on `nebula-core`.

### Dependency change

Add `thiserror = { workspace = true }` to `crates/metadata/Cargo.toml` —
already used by every other crate in the workspace.

### `nebula-action` migration

`ActionMetadata::MetadataCompatibilityError` becomes a thin wrapper:

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataCompatibilityError {
    #[error(transparent)]
    Base(#[from] BaseCompatError<ActionKey>),

    #[error("ports changed without a major version bump")]
    PortsChangeWithoutMajorBump,
}
```

`validate_compatibility` delegates then layers port-check:

```rust
pub fn validate_compatibility(&self, previous: &Self)
    -> Result<(), MetadataCompatibilityError>
{
    nebula_metadata::compat::validate_base_compat(&self.base, &previous.base)?;
    let ports_changed =
        self.inputs != previous.inputs || self.outputs != previous.outputs;
    if ports_changed && self.base.version.major == previous.base.version.major {
        return Err(MetadataCompatibilityError::PortsChangeWithoutMajorBump);
    }
    Ok(())
}
```

Note: current `KeyChanged { previous, current }` and
`VersionRegressed { previous, current }` variants move **inside** the `Base`
variant. This is a breaking change for any external matcher on the old flat
enum. `nebula-metadata` has no external consumers yet and the three
in-workspace consumers (`nebula-action`, `nebula-credential`, `nebula-resource`)
are updated in this PR, so the rename is acceptable. Existing tests inside
`crates/action/src/metadata.rs` are updated to match on
`MetadataCompatibilityError::Base(BaseCompatError::…)`.

### `nebula-credential` addition

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataCompatibilityError {
    #[error(transparent)]
    Base(#[from] BaseCompatError<CredentialKey>),

    #[error("auth pattern changed without a major version bump")]
    PatternChangeWithoutMajorBump,
}

impl CredentialMetadata {
    pub fn validate_compatibility(&self, previous: &Self)
        -> Result<(), MetadataCompatibilityError>
    {
        nebula_metadata::compat::validate_base_compat(&self.base, &previous.base)?;
        if self.pattern != previous.pattern
            && self.base.version.major == previous.base.version.major
        {
            return Err(MetadataCompatibilityError::PatternChangeWithoutMajorBump);
        }
        Ok(())
    }
}
```

Requires `AuthPattern: PartialEq + Eq` (verify during implementation).

### `nebula-resource` addition

```rust
// No entity-specific fields today; pure delegation.
impl ResourceMetadata {
    pub fn validate_compatibility(&self, previous: &Self)
        -> Result<(), BaseCompatError<ResourceKey>>
    {
        nebula_metadata::compat::validate_base_compat(&self.base, &previous.base)
    }
}
```

If and when `ResourceMetadata` grows entity-specific fields, the wrapper
pattern from action/credential kicks in.

### Tests

Unit tests in `compat.rs`:
- `key_change_rejected` (uses a test `struct FakeKey(&'static str)` — impls
  Debug/Display/Clone/PartialEq/Eq).
- `version_regression_rejected`.
- `schema_change_without_major_rejected`.
- `schema_change_with_major_accepted`.
- `minor_bump_with_same_schema_accepted`.
- `major_bump_with_same_schema_accepted` (regression guard).

Existing action/credential/resource tests that exercise the new wrapper:
- Action: port-change rules unchanged — only matcher shapes update.
- Credential: new `pattern_change_requires_major` test.
- Resource: new `validate_compatibility_roundtrip` test.

## Artifact 2 — `crates/metadata/README.md`

Standard workspace crate-README template (see `crates/error/README.md`,
`crates/log/README.md` for shape):

1. **Purpose** — one-paragraph summary (shared catalog prefix + trait).
2. **Core types** — `BaseMetadata<K>`, `Metadata` trait, `Icon`,
   `MaturityLevel`, `DeprecationNotice`, new `BaseCompatError<K>` +
   `validate_base_compat`.
3. **Composition example** — the `MyEntityMetadata` example from current
   `lib.rs` module doc, lifted and runnable.
4. **Consumers** — explicit list: `nebula-action`, `nebula-credential`,
   `nebula-resource`. Note on `nebula-plugin`: "reshape to `PluginManifest`
   tracked in ADR-0018".
5. **Crosslinks** — `docs/PRODUCT_CANON.md §3.5`, `docs/MATURITY.md`.

## Artifact 3 — `docs/MATURITY.md` row

```
| nebula-metadata      | frontier | stable | stable | n/a | n/a |
```

Inserted in alphabetical position between `nebula-log` and `nebula-metrics`.
`frontier` because:
- `compat.rs` lands in this PR (API surface moves this cycle).
- The `PluginManifest` ADR may imply future additions to `nebula-metadata`
  (e.g., shared `BundleManifest` marker trait) — we do not pre-commit.

Bump to `stable` on a follow-up PR after one release cycle without surface
changes.

Also update the `Last targeted revision` line in `docs/MATURITY.md` per
current repo convention.

## Artifact 4 — `docs/adr/0018-plugin-metadata-to-manifest.md`

Number `0018` (next free; 0017 is `control-queue-reclaim-policy`). Status
`proposed`.

### Content outline

- **Context.** `PluginMetadata` was introduced before `BaseMetadata` existed
  and still carries the pre-consolidation shape: `icon: Option<String>` +
  `icon_url: Option<String>` (two-field invalid-combination problem),
  `version: u32` (conflicts with ADR-0007 / semver canon elsewhere),
  `description: String`, no `maturity`, no `deprecation`, plus bundle-level
  fields (`author`, `license`, `homepage`, `repository`, `nebula_version`,
  `group`, `color`) that do not belong on leaf-entity metadata.
  Meanwhile `nebula-metadata::BaseMetadata<K>` is the canonical shape for
  catalog citizens and requires a `schema: ValidSchema` — which a plugin as
  a **container** does not have (user input lives on the actions/credentials
  /resources it bundles).

- **Decision.**
  - Rename `PluginMetadata` → `PluginManifest` (bundle descriptor).
  - `PluginManifest` **does not compose `BaseMetadata<K>`.** A plugin is not
    a schematized leaf; forcing it into the leaf shape with
    `ValidSchema::empty()` misleads the engine and UI.
  - `PluginManifest` **reuses the small types** from `nebula-metadata`:
    `Icon` (replaces `icon: Option<String>` + `icon_url: Option<String>`),
    `MaturityLevel` (new for plugins), `DeprecationNotice` (new for plugins).
  - `PluginManifest::version` adopts `semver::Version` (consistency with
    `BaseMetadata::version` and ADR-0007 conventions).
  - Plugin-specific fields stay: `author`, `license`, `homepage`,
    `repository`, `nebula_version`, `group`, `color`, `description`, `name`,
    `key`, `tags`.
  - Builder + normalization behavior preserved (`normalize_key`).

- **Consequences.**
  - Positive: `PluginManifest` stops advertising invalid icon combinations;
    plugins can declare `MaturityLevel::Experimental`; semver across the
    codebase.
  - Negative: wire format breaks for any persisted plugin metadata. Scope is
    small (no known production deployments, plugin-registry is frontier).
  - Neutral: `nebula-plugin` gains a direct dep on `nebula-metadata` for the
    three small types.

- **Migration plan (to be executed in a follow-up PR).**
  1. Introduce `PluginManifest` alongside `PluginMetadata`; deprecate the
     latter with a `#[deprecated]` attribute.
  2. Rename `Plugin::metadata() → Plugin::manifest()` directly (no shim —
     `nebula-plugin` is `frontier`-stability per `docs/MATURITY.md`;
     shim-naming is explicitly discouraged in `CLAUDE.md` quick-win trap
     catalog).
  3. Update `nebula-plugin::macros` to emit `PluginManifest::builder(...)`.
  4. Update `nebula-engine` re-export path (`engine::lib.rs:70`).
  5. Delete `PluginMetadata` in the cycle after.

- **Alternatives considered.**
  - **(A)** Make `BaseMetadata::schema` optional (`Option<ValidSchema>`) so
    plugin can fit. Rejected: every leaf-entity consumer gains an
    `Option<ValidSchema>` in its accessor for a single non-leaf case.
  - **(B)** Plugin uses `BaseMetadata` with `ValidSchema::empty()`. Rejected:
    misleading semantics ("empty schema" ≠ "no schema applies").
  - **(C)** Split `CatalogInfo<K>` (cosmetic prefix) from
    `BaseMetadata<K>` (cosmetic + schema). Rejected: adds an abstraction
    layer for a single non-leaf consumer. Revisit only if a second
    container-type (bundle, pack) appears.

- **Follow-ups.**
  - Tracking issue for the migration PR (not filed yet; create during
    implementation).
  - Revisit alternative (C) when bundle / pack descriptor arrives.

## Testing strategy

- `cargo nextest run -p nebula-metadata` covers new compat unit tests.
- `cargo nextest run -p nebula-action -p nebula-credential -p nebula-resource`
  covers the per-consumer wrapper integration.
- `cargo test --workspace --doc` covers the updated README examples in
  `crates/metadata/README.md`.
- `cargo clippy --workspace -- -D warnings` must stay green.
- Doc-test the composition example in `crates/metadata/README.md` via a
  fenced ```rust block, not `no_run`.

## Out of scope / explicit follow-ups

- Implementation of ADR-0018 (plugin → manifest rename).
- Extending `BaseMetadata` with author/license/homepage (plugin-manifest
  concern, not a leaf concern; see alternative A above).
- A generic `Catalog` trait over `M: Metadata` for engine-level listing
  (separate design — tracked separately).
- Sharing a builder across consumers (each family has different required
  fields; a one-size builder loses clarity).
