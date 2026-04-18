---
id: 0001
title: schema-consolidation
status: accepted
date: 2026-04-17
supersedes: []
superseded_by: []
tags: [schema, parameter, integration-model]
related: [docs/INTEGRATION_MODEL.md, crates/schema/src/lib.rs, docs/superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md]
---

# 0001. Schema consolidation — delete `nebula-parameter`, adopt `nebula-schema`

## Context

The `nebula-parameter` crate provided the typed configuration schema for Actions,
Credentials, and Resources: `Parameter`, `ParameterCollection`, validation rules,
conditions. Over time it accumulated scope creep (transformer pipelines, dynamic
fields, display modes) and duplicated functionality already present in
`nebula-validator`. The Field type hierarchy was an enum plus many per-kind
structs, leading to pattern-match fragility and a public API surface that
outpaced engine honoring.

The Phase 1 spec (`docs/superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md`)
proposed a consolidation — replace the old crate with a new `nebula-schema` crate
built around a proof-token pipeline.

## Decision

Delete `nebula-parameter`. Create `nebula-schema` with:

- A single consolidated `Field` enum covering all field kinds (string, number, bool, enum, nested, ...).
- `Schema` builder with structural lint (`Schema::lint`).
- Proof-token pipeline: schema-time validation via `ValidSchema::validate` returns `ValidValues`; runtime expression resolution via `ValidValues::resolve` returns `ResolvedValues`.
- Strongly typed error and path types.

Each integration concept (Action / Credential / Resource) composes `*Metadata + Schema`.

The proof-token types make "this schema has been validated" and "these values resolve" compile-time-evident — a caller cannot skip validation or resolution because the types enforce the sequence.

## Consequences

Positive:

- One schema system instead of two (nebula-parameter validation + nebula-validator rules).
- Proof-tokens remove an entire class of "did you remember to validate?" bugs — the type system does the checking.
- Public API surface shrinks; pattern-match fragility eliminated for common field kinds.
- Alignment with canon §4.5 ("public => honored") — the types cannot name operations the engine doesn't deliver.

Negative:

- Breaking change for any consumer of `nebula-parameter` (none outside the workspace at time of consolidation).
- All docs referencing `nebula-parameter` must be updated (handled in Pass 2 canon surgery + Pass 4 crate sweep of the docs architecture redesign).
- Some Phase 1 code is still `frontier` — not all surfaces are test-covered yet.

Follow-up:

- Phase 2 (DX layer), Phase 3 (security), Phase 4 (advanced) per the source specs. Separate ADRs will cover individual decisions as those phases land.
- Crate README for `nebula-schema` created in Pass 4 of the docs architecture redesign.

## Alternatives considered

- **Keep `nebula-parameter`, incrementally adopt proof-tokens inside it.** Rejected: the surface area was already too large; proof-token adoption without a crate boundary change would not reduce complexity and would hide the breaking change in partial migrations.
- **Split into `nebula-schema` + `nebula-fields` + `nebula-validation`.** Rejected: premature decomposition. One crate with clear module boundaries beats three crates with unclear edges at this stage.

## Seam / verification

Seam: `crates/schema/src/lib.rs` exports `Field`, `Schema`, `ValidSchema`, `ValidValues`, `ResolvedValues`. Tests: `crates/schema/tests/` (populated during Phase 1 landing).

Canon references:
- `docs/PRODUCT_CANON.md §1` names `nebula-schema` in the one-liner.
- `docs/INTEGRATION_MODEL.md` structural-contract section uses `*Metadata + Schema`.
- `docs/GLOSSARY.md §5` lists `Field`, `Schema`, `ValidValues`, `ResolvedValues` as the canonical schema-layer types.
