---
id: 0003
title: consolidated-field-enum
status: accepted
date: 2026-04-17
supersedes: []
superseded_by: []
tags: [schema, field-types, api-surface]
related: [crates/schema/src/lib.rs, docs/adr/0001-schema-consolidation.md, docs/superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md]
---

# 0003. Consolidated Field enum — 13 variants, remove Date / DateTime / Time / Color / Hidden

## Context

`nebula-parameter` carried a `Field` type hierarchy with per-kind structs and
an enum wrapper, plus five "convenience" variants — `Date`, `DateTime`, `Time`,
`Color`, and `Hidden` — that were not honored by the engine. They existed as
first-class variants but were either no-ops at runtime or required callers to
implement separate rendering logic that the engine never provided.

The `v4` design spec (extended in the Phase 1 foundation spec §3) called this
out: public variants that name operations the engine does not deliver violate
canon §4.5. The proliferation also created pattern-match fragility — every new
variant required update in all match arms across `action`, `credential`, and
`sdk`.

## Decision

Replace the old multi-struct field hierarchy with a single `#[non_exhaustive]`
`Field` enum with exactly 13 variants:
`String`, `Secret`, `Number`, `Boolean`, `Select`, `Object`, `List`, `Mode`,
`Code`, `File`, `Computed`, `Dynamic`, `Notice`.

Remove `Date`, `DateTime`, `Time`, `Color`, and `Hidden` entirely:
- Temporal fields → `Field::String` with `InputHint::Date / DateTime / Time`.
- Color pickers → `Field::String` with `InputHint::Color`.
- Hidden fields → `Field::String` with `VisibilityMode::Never` on the field.

The consolidation removes variants the engine did not honor without adding new
surface area. `#[non_exhaustive]` guards against future match exhaustiveness
regressions when variants are added.

## Consequences

Positive:

- Honors canon §4.5: every public variant names an operation the engine
  actually delivers.
- Pattern-match fragility reduced: a match over 13 variants with `#[non_exhaustive]`
  is maintainable; the old hierarchy required updating 5+ crates per new kind.
- `InputHint` and `VisibilityMode` encode display semantics orthogonally to
  the field's data type — a cleaner separation of concerns.

Negative:

- Breaking change: callers using `Field::Date(…)` / `Field::Color(…)` must
  migrate to `Field::String` with an appropriate hint. All internal callers
  (`action`, `credential`, `sdk`) were migrated in the same Phase 1 PR.
- `#[non_exhaustive]` requires external crates to add `_ => {}` arms; this is
  the correct trade-off for a library in alpha.

Follow-up:

- `InputHint` variants for richer display modes (date range, time with zone,
  rich color picker with opacity) can be added without changing `Field`.
- Phase 2 derive macros will generate `Field` constructors from struct
  attributes, eliminating boilerplate at definition sites.

## Alternatives considered

- **Keep the removed variants, mark them `#[deprecated]`.** Rejected: deprecated
  variants still require match arms; they don't reduce fragility. Canon §14
  ("delete over deprecate") and user memory note both prohibit shims.
- **Introduce a separate `DisplayHint` crate.** Rejected: premature decomposition.
  `InputHint` + `VisibilityMode` on the per-field structs is sufficient and
  keeps the schema self-contained.

## Seam / verification

Seam: `crates/schema/src/field.rs` — `Field` enum definition. The variants
`Date`, `DateTime`, `Time`, `Color`, `Hidden` do not appear; `InputHint` and
`VisibilityMode` carry their semantics.

`grep -r "Field::Date\|Field::Color\|Field::Hidden" crates/ apps/` returns zero
production hits (as of Phase 1 landing).

Landed in commit `ed3a0ce0` (feat(schema): Phase 1 Foundation).
