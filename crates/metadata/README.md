---
name: nebula-metadata
role: Shared catalog-metadata vocabulary (key + name + description + schema + icon + maturity)
status: stable
last-reviewed: 2026-04-19
canon-invariants: [L1-§3.5]
related: [nebula-action, nebula-credential, nebula-resource, nebula-plugin, nebula-schema]
---

# nebula-metadata

## Purpose

Every "catalog citizen" in Nebula — an action, a credential, a resource, a trigger, a plugin
— shares the same outward shape: a typed key, a human-readable name, a description, a
canonical [`ValidSchema`](https://docs.rs/nebula-schema) of user-configurable inputs, and a
small set of optional ornaments (icon, documentation URL, tags, maturity, deprecation notice).
Without a shared vocabulary, every business-layer crate would redeclare the same prefix with
slightly different field names, blocking a uniform Action/Credential/Resource catalog UI and
forcing every plugin author to learn the same fields five times.

`nebula-metadata` owns those shared concerns as concrete types and a small trait, so each
business-layer crate composes them instead of redefining them.

## Role

**Core-layer shared metadata.** Imported by `nebula-action`, `nebula-credential`,
`nebula-resource`, and (planned) `nebula-plugin` for their `*Metadata` types. Depends only on
`nebula-schema` and `semver`; no upward dependencies.

Pattern: **composition, not inheritance** — each business crate exposes its own
`*Metadata` struct that contains a `BaseMetadata<K>` and adds domain-specific fields.

## Public API

| Item                       | Purpose                                                                                         |
| -------------------------- | ----------------------------------------------------------------------------------------------- |
| `Metadata` trait           | Single-method trait (`base() -> &BaseMetadata<Self::Key>`) shared across catalog citizens       |
| `BaseMetadata<K>`          | Concrete struct with `key`, `name`, `description`, `version`, `parameters`, `icon`, `tags`, `maturity`, `deprecation` |
| `Icon`                     | Enum — one valid representation for catalog icons (URL, SVG, named, none)                       |
| `MaturityLevel`            | `Experimental` / `Beta` / `Stable` / `Deprecated`                                               |
| `DeprecationNotice`        | Standard deprecation payload (since version, replacement, removal target, message)              |
| `compat`                   | Version-compatibility helpers shared across metadata families                                   |

## Shape

```rust,no_run
use nebula_metadata::{BaseMetadata, Icon, MaturityLevel, Metadata};

pub struct MyKey;

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
```

## Non-goals

- **Identifier validation.** Key shapes are owned by `nebula-core` (e.g. `ActionKey`,
  `CredentialKey`); this crate is generic over `K`.
- **Schema construction.** `ValidSchema` is built and validated by `nebula-schema`.
- **Lifecycle / runtime state.** `BaseMetadata` is *static* description, not runtime status —
  see `nebula-execution` for execution state and `nebula-credential::CredentialRecord` for
  per-instance operational state.
- **Persistence format choice.** Catalog storage layers serialize via `serde`; this crate
  defines the shape, not the storage backend.

## Maturity

`stable`. Public surface is small and deliberately frozen — extending it requires a canon
revision (canon §3.5 trait family). Folding into `nebula-core::metadata` is tracked as a
separate audit follow-up; the public API would not change.
