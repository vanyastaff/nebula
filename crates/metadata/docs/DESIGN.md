# nebula-metadata design

## Boundary

`nebula-metadata` owns the common static-definition prefix for schematized
catalog leaves. It owns intrinsic metadata validity and generic revision
compatibility, but not action, credential, or resource-specific admission.
Plugin manifests are separate container descriptors and have no input schema.

## State transitions

The public construction states encode authority:

```text
authored fields                    admitted technical definition
MetadataDraft<K> + ValidSchema  -> BaseMetadata<K>
                         bind_schema

untrusted persistence/wire         current static definition
RecordedBaseMetadata<K> + BaseMetadata<K> -> BaseMetadata<K>
                                  exact readmission
```

`MetadataDraft<K>` contains key, checked name, description, version, icon,
documentation URL, tags, and lifecycle. It never contains a schema.
`bind_schema(ValidSchema)` is the sole transition to `BaseMetadata<K>`.

`BaseMetadata<K>` is private-field and getter-only. It serializes in the
existing flat shape but cannot be deserialized. This prevents wire or authored
data from manufacturing an admitted definition.

`RecordedBaseMetadata<K>` is the only lower-metadata deserialization target.
Its deserializer parses `K: FromStr`, validates the name, and validates the
lifecycle. `readmit_against` compares every recorded field and schema with a
fresh definition. On success it returns a clone of the fresh definition; on
mismatch it returns a payload-free `MetadataReadmissionError`.

An outer leaf with additional fields owns a corresponding recorded DTO and its
leaf-specific evidence comparison. This crate cannot admit an action,
credential, or resource definition on behalf of its owner.

## Lifecycle representation

Lifecycle is represented internally as:

```text
Active(Experimental | Beta | Stable) | Deprecated(DeprecationNotice)
```

The representation cannot express active metadata with a notice or deprecated
metadata without a notice. A notice takes precedence over recorded maturity.
Active maturity methods do not remove an attached notice.

## Wire contract

`BaseMetadata<K>` and `RecordedBaseMetadata<K>` serialize with the same flat
field names and default omission behavior:

```text
key, name, description, schema, version, icon,
documentation_url, tags, maturity, deprecation
```

Recorded decoding accepts owned JSON and readers by decoding the key as a
string before invoking the typed key parser. Parser details and submitted
metadata values are not included in metadata errors or tracing fields.

## Compatibility

Intrinsic validity and revision compatibility are separate:

- the key cannot change;
- version precedence cannot regress (build metadata is ignored);
- any schema inequality requires a major version bump.

The schema rule is deliberately conservative. Edge assignability is not a
substitute for static-definition revision compatibility.

## Plugin manifests

Manifest deserialization produces a builder and calls `build`, so normalized
key, name, and lifecycle checks have one implementation. Valid names are
trimmed before storage. A deprecation notice forces deprecated maturity, and
bare deprecated maturity is rejected.

## Observability

Construction, recorded decoding, readmission, manifest building, and
compatibility rejection emit named tracing events or spans. They include stable
error codes and exclude keys, names, descriptions, schemas, URLs, tags, and
deprecation reasons.

## Non-goals

- No final-metadata mutators or key replacement.
- No `Deserialize` for `MetadataDraft<K>` or `BaseMetadata<K>`.
- No compatibility constructors, aliases, or deprecated shims.
- No leaf-specific admission logic.
- No runtime credential, resource, action, or plugin behavior.
