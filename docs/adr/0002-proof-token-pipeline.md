---
id: 0002
title: proof-token-pipeline
status: accepted
date: 2026-04-17
supersedes: []
superseded_by: []
tags: [schema, validation, type-safety]
related: [crates/schema/src/lib.rs, docs/adr/0001-schema-consolidation.md, docs/superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md]
---

# 0002. Proof-token pipeline — ValidSchema / ValidValues / ResolvedValues

## Context

Before `nebula-schema`, callers could construct a schema and then pass
unvalidated value maps to an action. Nothing in the type system prevented
skipping validation, forgetting expression resolution, or mixing values from
two different schemas. Bugs were caught only at runtime or by convention.

The Phase 1 spec (`docs/superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md`)
identified this as a core correctness gap: the type system should make it
impossible to reach action execution with values that have not gone through
the full validation + resolution pipeline.

## Decision

Encode each pipeline stage as a distinct owned type (a "proof token"):

- `ValidSchema` — produced by `Schema::builder()…build()`. Witnesses that the
  schema structure is lint-clean (no duplicate keys, no dangling references, no
  contradictory rules).
- `ValidValues<'s>` — produced by `ValidSchema::validate(&values)`. Witnesses
  that the values satisfy schema-time rules and that the lifetime `'s` binds
  the values to the schema that produced them.
- `ResolvedValues<'s>` — produced by `ValidValues::resolve(&ctx).await`.
  Witnesses that every `FieldValue::Expression` leaf has been replaced by a
  `Literal`; value-rules that were deferred during expression mode are now
  enforced.

Callers cannot construct any of these tokens themselves; the only constructors
are the pipeline methods. This eliminates an entire class of "did you remember
to validate?" bugs via the type system.

## Consequences

Positive:

- Validation and resolution omissions become compile errors, not runtime panics.
- The lifetime `'s` in `ValidValues<'s>` prevents cross-schema value mixing at
  compile time.
- Action handlers receive `ResolvedValues<'s>` — no expression leaves in the
  tree; uniform handling.

Negative:

- Pipeline chaining is required at every call site; short-circuit is not
  possible without explicitly constructing intermediate tokens (which is
  intentionally impossible for callers).
- Some Phase 1 code paths covering expression-mode rules remain `frontier`
  until Phase 2/3 integration tests cover them fully.

Follow-up:

- Phase 2 derive macros will generate schema builders; `ValidSchema` will be a
  const at compile time for statically-known schemas.
- Expression runtime errors (parse failure, type mismatch) deferred to Phase 4
  (6 of 36 standard error codes).

## Alternatives considered

- **Plain `validate() -> Result<(), Errors>` with unchanged value types.**
  Rejected: callers can still pass the original, unguarded values onward after
  calling validate and ignoring the result.
- **Phantom-type brand on a single wrapper type.** Rejected: a single
  `Validated<T, Stage>` generic is less readable than purpose-named types and
  cannot encode the schema-lifetime relationship as naturally.

## Seam / verification

Seam: `crates/schema/src/lib.rs` — `ValidSchema`, `ValidValues`, `ResolvedValues`
are the only public pipeline types. Compile-fail fixtures in
`crates/schema/tests/compile_fail/` assert that:
- `ValidValues` cannot be constructed without `ValidSchema::validate`.
- `ResolvedValues` cannot be constructed without `ValidValues::resolve`.
- Values from schema A cannot be passed to schema B's validate call.

Landed in commit `ed3a0ce0` (feat(schema): Phase 1 Foundation).
