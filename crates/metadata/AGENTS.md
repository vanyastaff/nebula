# nebula-metadata — Agent orientation
> Local guide for `crates/metadata/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** Shared catalog-leaf metadata foundation (`MetadataDraft<K>` authoring, getter-only `BaseMetadata<K>` admission, `RecordedBaseMetadata<K>` evidence, `Metadata` access, ornaments, and generic compat rules).
**Layer:** Core — depends only downward (`nebula-core`, `nebula-schema`, `nebula-error`, `semver`, `serde`, `thiserror`, `tracing`); no upward deps.

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
- `MetadataDraft<K>` and `BaseMetadata<K>` must never implement `Deserialize`. Decode persistence or wire data as `RecordedBaseMetadata<K>` and re-admit it only by exact comparison with a freshly built static definition.
- Leaf-specific admitted metadata must not derive `Deserialize` through `BaseMetadata<K>`. The owning leaf crate defines an explicitly named recorded DTO and validates its own evidence after lower-metadata readmission.
- Consumers compose `BaseMetadata<K>` on their concrete struct and impl `Metadata` with a one-line `base()`; do not re-add the `Icon`/`MaturityLevel`/`DeprecationNotice` fields per crate.
- `Icon` is the single valid representation (`None`/`Inline`/`Url`); never reintroduce the old `icon: Option<String>` + `icon_url` pair.
- `PluginManifest` is a container, not a schematized leaf: it must NOT compose `BaseMetadata` or carry a canonical input schema (ADR-0018).
- Lifecycle is internally either active maturity or deprecated-with-notice. An attached notice takes precedence over every active maturity method and recorded maturity.
- Recorded deserialization parses leaf keys through `K: FromStr`; manifest/dependency wire keys use `PluginKey`. Preserve owned JSON and reader support without exposing parser payloads.
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

## See also

- `README.md` — full design, composition example, consumer list
- the ADR history (maintainers' private design vault) (ADR-0018) — plugin bundle-descriptor carve-out · `docs/PRODUCT_CANON.md §3.5` — integration model
