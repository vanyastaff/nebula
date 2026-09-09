---
name: nebula-metadata
role: Shared catalog-citizen metadata (BaseMetadata + Metadata trait + Icon / MaturityLevel / DeprecationNotice + compat rules)
status: stable
last-reviewed: 2026-09-08
canon-invariants: [L2-3.5]
related: [nebula-action, nebula-credential, nebula-resource, nebula-plugin]
---

# nebula-metadata

## Purpose

Every catalog leaf in Nebula — such as an action, a credential, or a
resource — shares the same surface: a typed key, a human-readable name
and description, a canonical input schema, optional catalog ornaments
(icon, documentation URL, tags), a declared maturity level, and an
optional deprecation notice. `nebula-metadata` owns those shared
concerns as concrete types and a small trait, so each business-layer
crate composes them instead of redeclaring the same prefix with
incompatible field names. Plugins are described separately as container
descriptors: they may reuse the small supporting types from this crate,
but they do not compose `BaseMetadata<K>` and do not carry a canonical
input schema (see ADR-0018).

## Role

**Core-layer support crate.** Cross-cutting, no upward dependencies.
Depends on `nebula-core` (for `PluginKey`, used by `PluginManifest`),
`nebula-error` (for the `Classify` derive on `ManifestError`),
`nebula-schema` (for `ValidSchema`), `semver`, `serde`, and `thiserror`.
Every other crate in the business layer (`nebula-action`,
`nebula-credential`, `nebula-resource`) composes `BaseMetadata<K>` via
`#[serde(flatten)]` on its own concrete metadata struct.

## Public API

- `BaseMetadata<K>` — shared catalog prefix (`key`, `name`, `description`,
  `schema`, `version`, `icon`, `documentation_url`, `tags`, `maturity`,
  `deprecation`). Composed on each concrete entity metadata.
- `Metadata` trait — one-line impl on each concrete metadata
  (`fn base(&self) -> &BaseMetadata<Self::Key>`); all other accessors
  default-delegate through it.
- `Icon` — `None` / `Inline(String)` / `Url { url: String }` enum;
  replaces the earlier `icon: Option<String>` + `icon_url: Option<String>`
  pair.
- `MaturityLevel` — `Experimental` / `Beta` / `Stable` / `Deprecated`.
- `DeprecationNotice` — `since` / `sunset` / `replacement` / `reason`.
- `BaseCompatError<K>` + `validate_base_compat` — entity-agnostic compat
  rules shared by every catalog citizen (`key` immutable, `version`
  monotonic, schema-break-requires-major-bump). Each consumer layers
  entity-specific rules on top via a thin wrapper enum.
- `PluginManifest` + `PluginManifestBuilder` — plugin bundle descriptor and
  its builder (`PluginManifest::builder(key, name)` then chained setters,
  `.build()`); see [Consumers](#consumers) for why it lives here instead
  of composing `BaseMetadata`.
- `PluginDependency` — one declared plugin-on-plugin dependency (`key` +
  semver `req`) inside a `PluginManifest`.
- `ManifestError` — `PluginManifestBuilder::build()` failure: `InvalidKey`
  (normalized key fails `PluginKey` validation) or `MissingRequiredField`
  (currently: an empty or whitespace-only `name`).

## Composition

```rust
use nebula_metadata::{BaseMetadata, Metadata};
use nebula_schema::ValidSchema;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyKey(&'static str);

pub struct MyEntityMetadata {
    pub base: BaseMetadata<MyKey>,
    pub extra_field: u32,
}

impl Metadata for MyEntityMetadata {
    type Key = MyKey;
    fn base(&self) -> &BaseMetadata<Self::Key> {
        &self.base
    }
}

fn empty_schema() -> ValidSchema {
    ValidSchema::empty()
}

let md = MyEntityMetadata {
    base: BaseMetadata::new(MyKey("k"), "My Entity", "desc", empty_schema()),
    extra_field: 7,
};
assert_eq!(md.name(), "My Entity");
```

## Consumers

- `nebula-action::ActionMetadata` — composes `BaseMetadata<ActionKey>`;
  adds `inputs`, `outputs`, `isolation_level`, `category`; wraps
  `BaseCompatError<ActionKey>` in its own `MetadataCompatibilityError`.
- `nebula-credential::CredentialMetadata` — composes
  `BaseMetadata<CredentialKey>`; adds `pattern`; wraps `BaseCompatError`
  similarly.
- `nebula-resource::ResourceMetadata` — composes
  `BaseMetadata<ResourceKey>`; no entity-specific fields today; wraps
  `BaseCompatError<ResourceKey>` in a single-variant
  `MetadataCompatibilityError` for shape parity with the other
  consumers.
- `PluginManifest` — **lives in this crate** (`src/manifest.rs`) and
  **does not** compose `BaseMetadata` by design (plugin is a container,
  not a schematized leaf); it reuses `Icon` / `MaturityLevel` /
  `DeprecationNotice` from here too. See ADR-0018 for the
  bundle-descriptor rationale. `nebula_plugin::PluginManifest` is a
  re-export of this type, not a second definition.

## Maturity semantics

`MaturityLevel` is **declarative catalog data an author states**, not a
guarantee the engine enforces. As of this writing nothing in the engine
reads `maturity` to gate dispatch, warn on activation, or change retry/
timeout behavior — it is metadata for the catalog UI and for humans
reading a manifest, not a runtime contract.

- `Experimental` — actively iterated; the author is telling integrators
  the public surface may break without notice.
- `Beta` — stabilizing; the author is committing to a deprecation cycle
  before a breaking change.
- `Stable` (default) — breaking changes require a major version bump.
  Because it is the default, an author who never touches the field ships
  as `Stable` — state `Experimental`/`Beta` explicitly if that is not
  true yet.
- `Deprecated` — scheduled for removal; pair with a `DeprecationNotice`.
  `deprecate()`/`with_deprecation()` on `BaseMetadata` and `.deprecation()`
  on `PluginManifestBuilder` both force `maturity = Deprecated` — but this
  holds only when construction goes through **exactly those entry
  points**, and only for `PluginManifest` does it hold unconditionally
  after that: `PluginManifestBuilder::build()` re-derives `maturity` from
  `deprecation` at build time, so a later `.maturity(Stable)` call cannot
  win. `BaseMetadata` has no such build step, so the invariant does *not*
  hold in at least three other places — pinned by the crate's
  `deprecation_flow` integration tests, not left implicit: (a)
  deserializing hand-written JSON, since `maturity` and `deprecation` are
  independent fields with no `Deserialize`-time invariant; (b) builder
  call order — `BaseMetadata::new(..).with_deprecation(n).with_maturity(
  MaturityLevel::Stable)` leaves `deprecation: Some(..)` with
  `maturity: Stable`, because `with_maturity` unconditionally overwrites
  and nothing re-derives it afterward; (c) direct field assignment — all
  ten `BaseMetadata` fields are `pub`, so a caller can set `maturity` and
  `deprecation` independently without calling either builder method at
  all. Treat "deprecation implies `Deprecated`" as a convention the
  `with_deprecation()`/`deprecate()` call enforces at the moment you call
  it, not a standing invariant of the type.

## SDK surface

`nebula-sdk`'s `prelude` re-exports: `BaseMetadata`, `Metadata`, `Icon`,
`MaturityLevel`, `DeprecationNotice`, `BaseCompatError`,
`validate_base_compat`, `PluginManifest`, `PluginManifestBuilder`,
`ManifestError`, `PluginDependency` — effectively this crate's entire
public surface. See `crates/sdk/docs/DESIGN.md` for the re-export
rationale and its interaction with issue 1000 (prelude contraction into
persona modules).

## Canon

- `docs/PRODUCT_CANON.md §3.5` — integration model (one pattern, five concepts).
- `docs/MATURITY.md` — crate-state dashboard row.
