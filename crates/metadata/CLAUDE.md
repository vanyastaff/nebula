# nebula-metadata — Claude Code orientation
> Agent quick-map for `crates/metadata/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** Shared catalog-leaf metadata surface (`BaseMetadata<K>` + `Metadata` trait + `Icon`/`MaturityLevel`/`DeprecationNotice` + generic compat rules) that every schematized entity composes instead of redeclaring.
**Layer:** Core — depends only downward (`nebula-core`, `nebula-schema`, `nebula-error`, `semver`, `serde`, `thiserror`); no upward deps.

## Commands
- `cargo check -p nebula-metadata`
- `cargo nextest run -p nebula-metadata`  ·  doctests: `cargo test -p nebula-metadata --doc`
- `#![warn(missing_docs)]` + `#![forbid(unsafe_code)]` — keep every public item documented.

## Key files
- `src/lib.rs` — module wiring + flat re-exports (the public surface)
- `src/base.rs` — `BaseMetadata<K>` struct + `Metadata` trait (default-delegating accessors)
- `src/compat.rs` — `BaseCompatError<K>` + `validate_base_compat` (key-immutable / version-monotonic / schema-break→major-bump)
- `src/manifest.rs` — `PluginManifest` + `PluginManifestBuilder` + `ManifestError` (container descriptor; NOT a `BaseMetadata`)
- `src/icon.rs` · `src/maturity.rs` · `src/deprecation.rs` — supporting catalog ornaments

## Conventions & never-do
- Consumers compose `BaseMetadata<K>` via `#[serde(flatten)]` on their own concrete struct and impl `Metadata` with a one-line `base()`; do NOT re-add the `Icon`/`MaturityLevel`/`DeprecationNotice` fields per-crate.
- `Icon` is the single valid representation (`None`/`Inline`/`Url`); never reintroduce the old `icon: Option<String>` + `icon_url` pair.
- `PluginManifest` is a container, not a schematized leaf: it must NOT compose `BaseMetadata` or carry a canonical input schema (ADR-0018).
- This crate owns only the *generic* base compat rules; each consumer layers entity-specific rules in a thin wrapper enum around `BaseCompatError<K>` — don't push entity rules down here.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design, composition example, consumer list
- `docs/adr/HISTORICAL.md` (ADR-0018) — plugin bundle-descriptor carve-out · `docs/PRODUCT_CANON.md §3.5` — integration model
