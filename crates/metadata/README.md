---
name: nebula-metadata
role: Shared catalog-citizen metadata (BaseMetadata + Metadata trait + Icon / MaturityLevel / DeprecationNotice + compat rules)
status: frontier
last-reviewed: 2026-04-19
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
input schema (see [ADR-0018](../../docs/adr/0018-plugin-metadata-to-manifest.md)).

## Role

**Core-layer support crate.** Cross-cutting, no upward dependencies.
Only depends on `nebula-schema` (for `ValidSchema`), `semver`, `serde`,
and `thiserror`. Every other crate in the business layer
(`nebula-action`, `nebula-credential`, `nebula-resource`) composes
`BaseMetadata<K>` via `#[serde(flatten)]` on its own concrete metadata
struct.

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

## Composition

```rust
use nebula_metadata::{BaseMetadata, Metadata};
use nebula_schema::{Schema, ValidSchema};

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
    Schema::builder().build().unwrap()
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
- `nebula-plugin::PluginManifest` — **does not** compose `BaseMetadata`
  by design (plugin is a container, not a schematized leaf). Reuses
  `Icon` / `MaturityLevel` / `DeprecationNotice` from this crate; see
  [ADR-0018](../../docs/adr/0018-plugin-metadata-to-manifest.md) for the
  bundle-descriptor rationale.

## Canon

- `docs/PRODUCT_CANON.md §3.5` — integration model (one pattern, five concepts).
- `docs/MATURITY.md` — crate-state dashboard row.
- `docs/STYLE.md` — idioms, naming, error taxonomy.
