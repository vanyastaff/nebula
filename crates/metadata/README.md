---
name: nebula-metadata
role: Shared catalog metadata authoring, evidence, access, and compatibility
status: stable
last-reviewed: 2026-09-11
canon-invariants: [L2-3.5]
related: [nebula-action, nebula-credential, nebula-resource, nebula-plugin]
---

# nebula-metadata

## Purpose

Every schematized catalog leaf has the same lower metadata fields: typed key,
display name, description, canonical input schema, version, catalog ornaments,
and lifecycle. This crate owns that shared representation and its invariants.
Action, credential, and resource metadata compose it rather than redeclaring
the prefix.

Plugin manifests are container descriptors. They reuse supporting types but do
not compose `BaseMetadata<K>` or carry a canonical input schema (ADR-0018).

## Construction model

Static definitions move in one direction:

```text
MetadataDraft<K> --bind_schema(ValidSchema)--> BaseMetadata<K>
```

`MetadataDraft<K>` owns every lower metadata field except `schema`. Use
`MetadataDraft::new` with a validated `MetadataName`, or
`MetadataDraft::try_new` for dynamic text. All fluent authoring happens on the
draft. `bind_schema` is the only transition to `BaseMetadata<K>`.

`BaseMetadata<K>` has private fields and getters only. It represents a technical
definition admitted from current static Rust code. It implements `Serialize`,
but deliberately does not implement `Deserialize`.

```rust
use nebula_metadata::{BaseMetadata, Metadata, MetadataDraft, metadata_name};
use nebula_schema::ValidSchema;

struct ActionMetadata {
    base: BaseMetadata<String>,
}

impl Metadata for ActionMetadata {
    type Key = String;

    fn base(&self) -> &BaseMetadata<Self::Key> {
        &self.base
    }
}

let metadata = ActionMetadata {
    base: MetadataDraft::new(
        "http.request".to_owned(),
        metadata_name!("HTTP Request"),
        "Send an HTTP request",
    )
    .mark_beta()
    .with_tags(["network", "http"])
    .bind_schema(ValidSchema::empty()),
};

assert_eq!(metadata.name(), "HTTP Request");
assert_eq!(metadata.tags(), ["network", "http"]);
```

The draft vocabulary is intentionally singular:

- `with_version(semver::Version)`
- `with_icon(Icon)`, `with_inline_icon`, `with_url_icon`
- `with_documentation_url`, `with_tags`, `add_tag`
- `mark_experimental`, `mark_beta`, `mark_stable`
- `with_deprecation(DeprecationNotice)`
- `bind_schema(ValidSchema)`

There is no key replacement or final-metadata mutation API. Rebuild a fresh
static definition when any authored field changes.

## Recorded evidence

Persisted or wire metadata is evidence about a prior definition, not authority
to instantiate an admitted definition. Deserialize it as
`RecordedBaseMetadata<K>`, then compare it with a freshly built static
`BaseMetadata<K>` using `readmit_against`. Every field and the schema must match.
The successful value is cloned from the fresh definition; no deserialized field
is promoted into the admitted value.

```rust
use nebula_metadata::{MetadataDraft, RecordedBaseMetadata, metadata_name};
use nebula_schema::ValidSchema;

# fn restore() -> Result<(), Box<dyn std::error::Error>> {
let fresh = MetadataDraft::new(
    "http.request".to_owned(),
    metadata_name!("HTTP Request"),
    "Send an HTTP request",
)
.bind_schema(ValidSchema::empty());

let wire = serde_json::to_string(&fresh)?;
let recorded: RecordedBaseMetadata<String> = serde_json::from_str(&wire)?;
let admitted = recorded.readmit_against(&fresh)?;

assert_eq!(admitted, fresh);
# Ok(())
# }
# restore().unwrap();
```

Invalid typed keys, blank names, and bare `Deprecated` maturity are rejected
while decoding recorded evidence. A deprecation notice always determines
`Deprecated` maturity, including when the recorded `maturity` field says
otherwise. A mismatch returns `MetadataReadmissionError` without definition
payloads in its error or trace output.

Leaf metadata with additional fields needs an explicitly named recorded leaf
DTO containing `RecordedBaseMetadata<K>`. Re-admit the base against the fresh
leaf definition and validate any leaf-specific evidence in the owning crate.
Deriving `Deserialize` on an admitted leaf that contains `BaseMetadata<K>` is
intentionally impossible.

## Public API

- `MetadataDraft<K>` - schema-free authoring state.
- `BaseMetadata<K>` - getter-only admitted technical definition.
- `RecordedBaseMetadata<K>` - validated wire or persistence evidence.
- `MetadataName` and `metadata_name!` - checked dynamic and literal names.
- `MetadataVersion` - nameable alias for `semver::Version`.
- `Metadata` - shared getter trait for composed leaf metadata.
- `Icon`, `MaturityLevel`, `DeprecationNotice` - catalog supporting types.
- `MetadataError`, `MetadataBuildError`, `MetadataReadmissionError` - typed,
  payload-free construction, schema, and restoration errors.
- `BaseCompatError<K>` and `validate_base_compat` - key immutability, SemVer
  monotonicity, and conservative schema-change compatibility.
- `PluginManifest`, `PluginManifestBuilder`, `PluginDependency`, and
  `ManifestError` - checked plugin container metadata.

## Lifecycle invariant

Internally, lifecycle is either an active maturity or a deprecated state that
contains its notice. Active-with-notice and deprecated-without-notice cannot be
represented. Once `with_deprecation` is called, later `mark_*` calls preserve
the deprecated state and its notice.

The serialized `BaseMetadata` wire shape remains flat. Default version, icon,
documentation URL, tags, maturity, and deprecation fields retain their existing
serde omission behavior.

## Plugin manifests

`PluginManifest::builder(key, name).build()` validates normalized `PluginKey`,
name, and lifecycle through the same intrinsic rules. Accepted names are stored
trimmed. Manifest deserialization goes through the builder and therefore cannot
bypass these checks.

## Leaf migration checklist

1. Build a `MetadataDraft<K>` with `new` or `try_new`.
2. Apply version, ornament, and lifecycle methods before schema binding.
3. Replace the old constructor or schema setter with `bind_schema`.
4. Rebuild instead of mutating a bound key or any other bound field.
5. Remove `Deserialize` from admitted leaf metadata containing `BaseMetadata<K>`.
6. Add a recorded leaf DTO containing `RecordedBaseMetadata<K>` where persisted
   restoration is required.
7. Re-admit recorded evidence only against a freshly built static definition,
   then validate leaf-specific evidence in the leaf crate.

No compatibility constructors, mutators, aliases, or deprecated shims remain
in `nebula-metadata`.

## Dependencies and consumers

This core-layer crate depends on `nebula-core`, `nebula-error`,
`nebula-schema`, `semver`, `serde`, `thiserror`, and `tracing`. Its direct leaf
consumers are `nebula-action`, `nebula-credential`, and `nebula-resource`;
`nebula-plugin` consumes the manifest and compatibility surface. The supported
downstream Rust surface is curated by `nebula-sdk`.

## Canon

- Product canon section 3.5: stable catalog identity and revision semantics.
- ADR-0018: plugin manifests are container descriptors, not schematized leaves.
- Root `docs/INTEGRATION_MODEL.md`: cross-crate composition and admission model.
