# nebula-metadata — Agent orientation
> Local guide for `crates/metadata/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** Shared catalog-leaf metadata foundation (`MetadataDraft<K>` authoring, getter-only `BaseMetadata<K>` admission, `RecordedBaseMetadata<K>` evidence, `Metadata` access, ornaments, and generic compat rules).
**Layer:** Core — depends only downward (`nebula-core`, `nebula-schema`, `nebula-error`) and shared parsing/serialization libraries (`semver`, `serde`, `serde_json`, `domain-key`, `url`, `chrono`, `thiserror`, `tracing`); no upward deps.

## Commands

- `cargo nextest run -p nebula-metadata`  ·  doctests: `cargo test -p nebula-metadata --doc`

## Key files

- `src/lib.rs` — module wiring + flat re-exports (the public surface)
- `src/base.rs` — draft/bind construction, recorded evidence readmission, `BaseMetadata<K>`, and the `Metadata` trait
- `src/definition.rs` — construction/readmission errors and shared intrinsic checks
- `src/compat.rs` — `BaseCompatError<K>` + `validate_base_compat` (key-immutable / version-monotonic / schema-break→major-bump)
- `src/manifest.rs` — `PluginManifest` + `PluginManifestBuilder` + `ManifestError` (container descriptor; NOT a `BaseMetadata`)
- `src/icon.rs` · `src/maturity.rs` · `src/deprecation.rs` — supporting catalog ornaments

## Conventions & never-do

- Consumers author `MetadataDraft<K>`, apply every fluent option there, and call `bind_schema` once. `BaseMetadata<K>` remains private-field and getter-only; do not add final-state mutators or key replacement.
- `bind_schema` returns `Result` and requires `K: Serialize`. The final gate validates all authored fields, chronology, exact shared JSON (32 KiB excluding schema), separate schema JSON (2 MiB), and complete record JSON (`MAX_METADATA_JSON_BYTES`, 4 MiB). Shared/schema budget accounting uses streaming counting writes. `check_json_record` captures the complete record within the fixed ceiling and checks object shape and depth with serde_json's unchanged recursion limit. Leaf owners apply it to the whole composed record at admission and recorded ingress.
- Discovery collections accept at most 64 raw entries and inspect at most a 65th sentinel. Canonical counts are categories 16, tags 32, links 16. Replace setters clear only the replaced field's pending validation. `with_version_literal` is hidden macro support; `with_version` clears invalid literal intent.
- Shared records and manifests require `metadata_wire_version: 2`; shared DTOs reject unknown fields. Leaf records must nest `base`, never serde-flatten it. Historical flat leaves, `documentation_url`, and `sunset` are unsupported evidence.
- `with_documentation_url` replaces Overview links. No duplicate URL storage. Categories/tags/links canonicalize; deprecation uses typed reference/removal and SemVer precedence chronology. Elapsed guidance never grants or revokes execution authority.
- Use `decode_json_slice`/`decode_json_reader` for raw transport. Direct serde enforces structure, but its visitors do not bound parser scratch; owned JSON is already allocated. Use `deserialize_metadata_object` for map-only DTO ingress; derived structs alone also accept positional sequences. Sanitize private DTO parse failures before exposing errors, including unknown fields/variants and wrong scalar types.
- The shared wire version is not completion of the full catalog protocol. Plugin integration still owns explicit catalog export requirements and derived leaf facts; do not change historical plugin compiler/plan envelopes merely because shared metadata changes.
- `MetadataDraft<K>` and `BaseMetadata<K>` must never implement `Deserialize`. Decode persistence or wire data as `RecordedBaseMetadata<K>` and re-admit it only by exact comparison with a freshly built static definition.
- Leaf-specific admitted metadata must not derive `Deserialize` through `BaseMetadata<K>`. The owning leaf crate defines an explicitly named recorded DTO and validates its own evidence after lower-metadata readmission.
- Consumers compose `BaseMetadata<K>` on their concrete struct and impl `Metadata` with a one-line `base()`; do not re-add the `Icon`/`MaturityLevel`/`DeprecationNotice` fields per crate.
- `Icon` is the single valid representation (`None`/`Inline`/`Url`); never reintroduce the old `icon: Option<String>` + `icon_url` pair.
- `PluginManifest` is a container, not a schematized leaf: it must NOT compose `BaseMetadata` or carry a canonical input schema (ADR-0018).
- Lifecycle is internally either active maturity or deprecated-with-notice. An attached notice takes precedence over every active maturity method and recorded maturity.
- Recorded deserialization parses leaf keys through `K: FromStr`; manifest/dependency wire keys use `PluginKey`. Supported generic keys must serialize deterministically as JSON strings and recover identically through `FromStr`. Shared validation separately captures key JSON within 32 KiB and rejects non-string representations; never replace this with unbounded serialization after a counting pass. Preserve owned JSON and reader support without exposing parser payloads.
- Intrinsic validity and revision compatibility are separate. Keep the conservative schema equality gate, including UI-only changes; edge assignability is not revision compatibility. Revision ordering uses `Version::cmp_precedence`, ignoring build metadata.
- `src/lib.rs` includes this crate's README as rustdoc. README changes can therefore affect shipped docs and doctests, not just repository prose.
- This crate owns only the *generic* base compat rules; each consumer layers entity-specific rules in a thin wrapper enum around `BaseCompatError<K>` — don't push entity rules down here.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Draft/bind, wire evidence, or readmission | [draft_flow](tests/draft_flow.rs), [construction_flow](tests/construction_flow.rs), [construction_properties](tests/construction_properties.rs). |
| Leaf composition or compatibility | [composition_flow](tests/composition_flow.rs), [compat_flow](tests/compat_flow.rs). |
| Maturity/deprecation or manifests | [deprecation_flow](tests/deprecation_flow.rs), [plugin_manifest_flow](tests/plugin_manifest_flow.rs); cover setter order and deserialization, not only the happy-path builder. |
| Intrinsic construction or diagnostics | [construction_flow](tests/construction_flow.rs), [construction_properties](tests/construction_properties.rs), [observability_flow](tests/observability_flow.rs); cover blank names, typed keys, owned/reader serde, bare Deprecated, round-trip/lifecycle laws, and payload-free traces. |
| Catalog budgets, canonicalization, and wire migration | [catalog_admission](tests/catalog_admission.rs), [bounded_wire](tests/bounded_wire.rs), [wire_version](tests/wire_version.rs), [catalog_primitives](tests/catalog_primitives.rs); retain exact JSON escaping, raw iterator/reader sentinel, field-local repair, strict versions, canary diagnostics, and exact readmission coverage. |

## See also

- `README.md` — full design, composition example, consumer list
- the ADR history (maintainers' private design vault) (ADR-0018) — plugin bundle-descriptor carve-out · `docs/PRODUCT_CANON.md §3.5` — integration model
