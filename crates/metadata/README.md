---
name: nebula-metadata
role: Shared catalog metadata authoring, evidence, access, and compatibility
status: stable
last-reviewed: 2026-09-12
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

## Breaking

These changes require a **0.7 release**; they are not compatible with the 0.6
raw Rust API or metadata record format. Package versions have **not** been
bumped in this change. The supported downstream Rust surface remains
`nebula-sdk`; the following migration details also apply to direct internal
consumers of `nebula-metadata`. There are no compatibility shims or legacy-wire
fallbacks.

### Construction and errors

| Previous use | Required migration |
|---|---|
| `let base = draft.bind_schema(schema);` | Use `let base = draft.bind_schema(schema)?;`. The return type is `Result<BaseMetadata<K>, MetadataBuildError>`, and binding now requires `K: serde::Serialize`. |
| Recorded decoding with only `K: FromStr` | `RecordedBaseMetadata<K>: Deserialize` now requires `K: FromStr + Serialize` for exact canonical accounting. Key parser diagnostics are not exposed. |
| Generic keys serializing as numbers, arrays, or objects | Use a key whose deterministic `Serialize` representation is a JSON string and whose `FromStr` recovers that same key from the string contents. Non-string key representations fail with `MetadataError::InvalidKey`. |
| Deserializing `PluginManifestBuilder` | Deserialize `PluginManifest` instead; its private wire DTO passes through the final build gate. The public builder no longer implements `Deserialize`. |
| Matching `ManifestError::InvalidKey(source)` | Match `ManifestError::InvalidKey`. Its `PluginKeyParseError` payload and error source are removed; use the stable error code rather than parsing diagnostic text. |
| Treating any constructed draft or supporting value as admitted | Handle final admission failures for fields, collection counts, chronology, canonical bytes, and complete-record depth. `MetadataDraft` and `BaseMetadata` do not implement `Deserialize`. |

`MetadataError` adds data-carrying `InvalidVersion(MetadataField)`,
`FieldTooLarge(MetadataField)`, `TooManyEntries(MetadataField)`,
`TooManyRawEntries(MetadataField)`, and `CatalogValue(CatalogValueError)` variants,
plus typed chronology, canonical-budget, record-depth, serialization, and wire
failures. `MetadataField` contains closed canonical locations, never supplied
paths. In particular, an invalid manifest dependency requirement reports
`InvalidVersion(MetadataField::ManifestDependencies)`. Continue to handle the
non-exhaustive error enums. Parser and I/O payloads are deliberately discarded;
do not rely on their former wording or source chains.

### Deprecation and discovery

| Previous use | Required migration |
|---|---|
| Reading or writing public notice fields | Construct with `DeprecationNotice::new(since)` and setters; read through `since()`, `removal()`, `replacement()`, and `reason()`. All fields are private. |
| `.sunset(text)` or the `sunset` field | Use `.with_removal(RemovalSchedule::OnDate(date))`, `AtVersion(version)`, or `Milestone(milestone)` with the checked supporting type. There is no free-form sunset field. |
| `.replacement(key_text)` | Use `.with_replacement(CatalogReference::action(key))` or the appropriate credential/resource/plugin family, optionally with a checked version requirement. |
| `.reason(text)` | Use `.with_reason(text)`; `reason()` is now a getter. |
| Independent `documentation_url` storage | Use typed `links`; `with_documentation_url` replaces the Overview relation and `documentation_url()` reads it. Conflicting Overview targets added through `add_link` fail admission. |
| Relying on discovery insertion order or duplicates | Categories and links are canonically sorted/deduplicated; tags are trimmed, sorted/deduplicated, and cannot be blank. Existing unbounded inputs may now fail the documented limits. |

Notice admission requires `since <= current` by SemVer precedence and any
`AtVersion` removal to follow `since`. Correct an inconsistent current-version
fixture or authored definition rather than bypassing chronology. Elapsed dates
remain valid guidance. Reference and manifest dependency requirements must
retain their exact intent through bounded SemVer formatting and parsing;
manually constructed requirements can therefore fail admission even when their
Rust type is valid. Guidance-only changes do not require an interface major
version, but still change exact readmission evidence. Conservative schema
compatibility remains unchanged, including schema UI-field changes.

### Records and raw ingress

Shared records and plugin manifests now require `metadata_wire_version: 2`.
Leaf records place the shared object under a nested `base`, never serde
flattening. Missing, old, and unknown versions, positional arrays, unknown
fields, legacy `documentation_url`/`sunset`, and raw replacement strings are
rejected. Empty canonical discovery collections are omitted when serialized.
Regenerate records from the fresh static definitions; adding a discriminator to
an old flat leaf does not migrate it. Readmission checks every canonical shared
field and schema and returns only the matching fresh definition.

For raw input, use `decode_json_slice::<RecordedLeaf>(bytes, limits)` or
`decode_json_reader::<RecordedLeaf>(reader, limits)`, returning
`MetadataDecodeError`. `MetadataDecodeLimits::default()` caps the whole envelope
at 4 MiB; `MetadataDecodeLimits::new` accepts only a positive, lower-or-equal
ceiling. Direct serde remains structurally checked but needs external transport
and parser-allocation budgets. Leaf implementors use `deserialize_metadata_object`
for sanitized map-only ingress and `check_json_record` on the whole admitted or
recorded leaf for the shared byte/depth invariant. Shared/schema byte accounting
remains streaming; complete-record depth validation uses fixed-ceiling capture.

The manifest's new record format does not turn packaging into a schematized
leaf or grant publisher/runtime authority. Historical plugin compiler and plan
records do not serialize this shared base or notice and retain their existing
versions and hashes. Shared wire v2 does not complete or replace the separately
versioned integration-catalog export prerequisite.

## Construction model

Static definitions move in one direction:

```text
MetadataDraft<K> --bind_schema(ValidSchema)--> Result<BaseMetadata<K>, MetadataBuildError>
```

`MetadataDraft<K>` owns every lower metadata field except `schema`. Use
`MetadataDraft::new` with a validated `MetadataName`, or
`MetadataDraft::try_new` for dynamic text. All fluent authoring happens on the
draft. `bind_schema` is the only transition to `BaseMetadata<K>` and checks every
shared field, chronology, and byte budget. It requires `K: Serialize` for exact
canonical JSON accounting, including generic keys.

Supported generic keys serialize deterministically as JSON strings. Their
`FromStr` implementation must recover the same key from those string contents;
`String` and the core entity key types satisfy this contract. After streaming
shared-budget validation, admission captures the key independently within
32 KiB and rejects non-string representations with `MetadataError::InvalidKey`.
Recorded ingress enforces the same check after parsing the key. Trait bounds
alone cannot establish a custom serializer/parser's semantic round-trip law.

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
    .bind_schema(ValidSchema::empty())?,
};

assert_eq!(metadata.name(), "HTTP Request");
assert_eq!(metadata.tags(), ["http", "network"]);
# Ok::<(), nebula_metadata::MetadataBuildError>(())
```

The draft vocabulary is intentionally singular:

- `with_version(semver::Version)`
- `with_icon(Icon)`, `with_inline_icon`, `with_url_icon`
- `with_description`, `with_categories`, `with_tags`, `add_tag`
- `with_links`, `add_link`, `with_documentation_url`
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
.bind_schema(ValidSchema::empty())?;

let wire = serde_json::to_string(&fresh)?;
let recorded: RecordedBaseMetadata<String> = serde_json::from_str(&wire)?;
let admitted = recorded.readmit_against(&fresh)?;

assert_eq!(admitted, fresh);
# Ok(())
# }
# restore().unwrap();
```

Invalid fields, counts, aggregate budgets, chronology, and bare `Deprecated`
maturity are rejected while decoding recorded evidence. A notice determines
`Deprecated` maturity, including when the recorded `maturity` field says
otherwise. A mismatch returns `MetadataReadmissionError` without definition
payloads in its error or trace output.

Leaf metadata with additional fields needs an explicitly named recorded leaf
DTO containing a nested `base: RecordedBaseMetadata<K>`. Do not use serde
flattening. Re-admit the base against the fresh
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
- `CatalogCategoryKey`, `CatalogLink`, `CatalogLinkRelation`, `CatalogLinkTarget`,
  `DocumentationOrigin`, `CatalogReference`, `RemovalDate`, `RemovalMilestone`,
  `RemovalSchedule`, `Icon`, `MaturityLevel`, `DeprecationNotice` - supporting types.
- `MetadataError`, `MetadataBuildError`, `MetadataReadmissionError` - typed,
  payload-free construction, schema, and restoration errors.
- `MetadataField` - closed field locations in size and collection diagnostics.
- `MetadataDecodeLimits`, `MetadataDecodeError`, `decode_json_slice`,
  `decode_json_reader` - bounded raw JSON ingress for shared or leaf records.
- `BaseCompatError<K>` and `validate_base_compat` - key immutability, SemVer
  monotonicity, and conservative schema-change compatibility.
- `PluginManifest`, `PluginManifestBuilder`, `PluginDependency`, and
  `ManifestError` - checked plugin container metadata.

## Lifecycle invariant

Internally, lifecycle is either an active maturity or a deprecated state that
contains its notice. Active-with-notice and deprecated-without-notice cannot be
represented. Once `with_deprecation` is called, later `mark_*` calls preserve
the deprecated state and its notice.

`DeprecationNotice::new(since)` supports `with_removal`, `with_replacement`, and
`with_reason`, with private fields and corresponding getters. Require
`since <= current` by SemVer precedence and `AtVersion > since`. Build metadata
does not affect chronology. Elapsed dates and unresolved typed references remain
valid guidance; they grant no bindings, fetch permission, or execution authority.

## Discovery and limits

Categories and trimmed tags are sorted and deduplicated. Links are sorted by
relation and canonical target and deduplicated. Conflicting Overview targets
fail admission. `with_documentation_url` replaces the single Overview link;
there is no independent URL field. Root-relative links resolve only with an
explicit `DocumentationOrigin`; admission performs no network I/O.

| Field | Protocol ceiling |
|---|---|
| Categories | 16 canonical keys, 96 UTF-8 bytes each |
| Tags | 32 canonical tags, 64 UTF-8 bytes each; blank tags rejected |
| Links | 16 canonical links, 2048 UTF-8 bytes per target |
| Description | 8 KiB UTF-8 |
| Removal milestone | 256 UTF-8 bytes |
| All serialized shared authored fields | 32 KiB canonical JSON, excluding schema |
| Serialized bound schema | Separate 2 MiB canonical JSON ceiling |
| Complete record | 4 MiB canonical JSON and raw transport ceiling |

The authored budget uses the actual shared serializer with the schema omitted,
including field names, delimiters, escaping, key, name, version, icon, and notice.
Shared and schema counting writes stop at their limits without building JSON
buffers. A separate key-shape check captures at most 32 KiB, independently of
earlier serializer output. Complete records use `check_json_record`: bounded capture stops at
`MAX_METADATA_JSON_BYTES`, the same ceiling used by the decoder, then serde_json
checks the exact object representation with its unchanged recursion limit. This
also covers nesting in defaults, rule operands, and leaf fields. Leaf owners
apply the check to their complete composed record at admission and recorded
ingress. Successfully admitted canonical records fit the default decoder;
hosts may intentionally reject more by lowering the raw envelope limit.

Authoring accepts at most 64 raw entries per discovery collection, consuming at
most 65 entries to detect overflow, even for an infinite duplicate iterator.
Recorded sequences enforce the same cap before canonicalization. Replace setters
clear errors only for the replaced field. Individual raw tags are capped at
32 KiB before trimming to bound normalization work.

## Versioned wire ingress

Every shared record and plugin manifest requires `metadata_wire_version: 2`.
Missing, old, and unknown versions fail. Shared wire DTOs accept objects only,
never positional arrays, and reject unknown fields;
legacy `documentation_url`, raw replacement strings, and `sunset` are unsupported.
Leaf wire records nest the shared record under `base`; historical flat leaf
records must not be reinterpreted as the new format.

Use `decode_json_slice::<RecordedLeaf>(&bytes, limits)` or
`decode_json_reader::<RecordedLeaf>(reader, limits)` at raw ingress. Limits apply
to the complete leaf envelope, including leaf fields and all schemas. Readers
consume at most the selected maximum plus one sentinel before parsing. Hosts
own framing, read deadlines, and limits on any larger surrounding catalog.

Direct `Deserialize` remains structurally checked, including canonical shared,
schema, and whole-record budgets. Its bounded string/sequence visitors do not
bound a parser's scratch allocation. Direct serde callers must provide external
transport budgets; an owned `serde_json::Value` is already allocated. Public
record/notice/manifest decoding and bounded helpers discard parser and I/O
diagnostic payloads. Errors contain closed codes and field locations only.

`METADATA_WIRE_VERSION` describes the shared record format. The full integration
catalog still requires a separately versioned export with closed schema/policy,
slot, options, and derived leaf requirements owned by plugin integration. This
crate does not claim that prerequisite is complete. Historical plugin compiler,
plan, and schema-envelope versions are independent contracts.

## Plugin manifests

`PluginManifest::builder(key, name).build()` validates normalized `PluginKey`,
name, and lifecycle through the same intrinsic rules. Accepted names are stored
trimmed. Manifest deserialization goes through the builder and therefore cannot
bypass these checks. Manifests share category/link types, canonicalization,
chronology, and the 32 KiB authored-field budget, without composing `BaseMetadata`
or binding a schema. Packaging fields remain separate, with at most 64 group or
dependency entries and a 64 KiB complete manifest ceiling. Structural validity
does not prove publisher trust or tenant availability.

## Leaf migration checklist

1. Build a `MetadataDraft<K>` with `new` or `try_new`.
2. Apply version, ornament, and lifecycle methods before schema binding.
3. Propagate the `Result` from `bind_schema`.
4. Rebuild instead of mutating a bound key or any other bound field.
5. Remove `Deserialize` from admitted leaf metadata containing `BaseMetadata<K>`.
6. Add a recorded leaf DTO containing `RecordedBaseMetadata<K>` where persisted
   restoration is required, nested under `base` with strict shared fields.
7. Re-admit recorded evidence only against a freshly built static definition,
   then validate leaf-specific evidence in the leaf crate.

No compatibility constructors, mutators, aliases, or deprecated shims remain
in `nebula-metadata`.

## Dependencies and consumers

This core-layer crate depends on `nebula-core`, `nebula-error`,
`nebula-schema`, `semver`, `serde`, `serde_json`, `domain-key`, `url`, `chrono`,
`thiserror`, and `tracing`. Its direct leaf
consumers are `nebula-action`, `nebula-credential`, and `nebula-resource`;
`nebula-plugin` consumes the manifest and compatibility surface. The supported
downstream Rust surface is curated by `nebula-sdk`.

## Canon

- Product canon section 3.5: stable catalog identity and revision semantics.
- ADR-0018: plugin manifests are container descriptors, not schematized leaves.
- Root `docs/INTEGRATION_MODEL.md`: cross-crate composition and admission model.
