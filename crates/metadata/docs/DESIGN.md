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
typed categories and links, tags, and lifecycle. It never contains a schema.
`bind_schema(ValidSchema) -> Result<BaseMetadata<K>, MetadataBuildError>` is the
sole admission transition, metering canonical authored fields and the full record.

`BaseMetadata<K>` is private-field and getter-only. It serializes in the
nested shared `base` record shape but cannot be deserialized. This prevents wire or authored
data from manufacturing an admitted definition.

`RecordedBaseMetadata<K>` is the only lower-metadata deserialization target.
Its deserializer requires `K: FromStr + Serialize`, validates all canonical shared
fields, chronology, and byte budgets. `readmit_against` compares every field and schema with a
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

`BaseMetadata<K>` and `RecordedBaseMetadata<K>` serialize the same versioned
shared object, nested under `base` by leaf records:

```text
metadata_wire_version: 2, key, name, description, schema, version, icon,
categories, tags, links, maturity, deprecation
```

Recorded decoding accepts owned JSON and readers by decoding the key as a
string before invoking the typed key parser. Generic keys must serialize
deterministically as JSON strings and recover identically through `FromStr`;
the trait bounds alone do not prove that semantic law. Shared admission and
recorded ingress capture key JSON independently within 32 KiB to reject
non-string representations without trusting an earlier serializer invocation.
Parser details and submitted
metadata values are not included in metadata errors or tracing fields. Unknown
fields, positional arrays, and unsupported versions fail. Raw transport uses bounded slice/reader
helpers; direct serde needs an external allocation budget. See the README for
the canonical shared, schema, raw collection, and complete-record limits.

Shared authored and schema byte budgets use streaming counting writes. The
complete canonical record is captured within the fixed 4 MiB ceiling and parsed
with serde_json's unchanged recursion limit. `check_json_record` checks the
whole shared or composed leaf object at admission and recorded ingress; schema
field depth alone cannot account for defaults, rule operands, or leaf nesting.

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
