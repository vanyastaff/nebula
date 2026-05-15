# 0052 — Field visibility/required condition evaluation moves to nebula-validator

**Status:** accepted (2026-05-15)
**Tags:** schema, validator, seam, visibility, required, m11
**Related:** 0002, 0003, 0034, 0011 (extends; supersedes none)

## Context

`nebula-schema::validate_field` evaluated `VisibilityMode::When(Rule)` and
`RequiredMode::When(Rule)` via `Rule::evaluate(&dyn RuleContext)` against a
flat `RootContext`. That path does a flat-key lookup and silently returns
`false` for nested JSON-Pointer paths (documented Known Limitation,
`crates/validator/src/rule/mod.rs:175-185`), so a `required_when` predicate on
a nested sibling fails OPEN — a mandatory field silently becomes optional.
Predicate evaluation and `PredicateContext` (nested-correct) already live in
`nebula-validator`. Two owners of one invariant (schema pattern-matches the
mode, validator owns `Rule`) produced the drift.

## Decision

The condition-evaluation engine for visibility/required moves into
`nebula-validator` as a `policy` module: `Presence`/`Requiredness`
`#[non_exhaustive]` enums, `VisibilityPolicy`/`RequiredPolicy` with a single
`resolve(&PredicateContext)` (no public `bool`), `Rule::matches(&PredicateContext)`,
and `resolve_field_policies(...) -> FieldPolicyResolution { plans, required_failures }`.
`nebula-schema` retains `VisibilityMode`/`RequiredMode` as serde-stable
`Rule`-carrying data and maps them to the borrowed validator policy types at
`validate` time; it consumes the result through a single
`match plan.presence { Skipped => continue, Active => … }` so the field-rule
path is unreachable from `Skipped` by data flow. `Rule::evaluate` and the
`RuleContext` trait are deleted (no shim — their own `TODO(post-refactor)`
scopes removal to exactly this work).

`PredicateContext` construction at this boundary excludes fields whose schema
`Field` is `Field::Secret` — pre-resolve a secret is `FieldValue::Literal`
plaintext, so the runtime-tag scrub in `context.rs` did not catch it (ADR-0034
§3 redaction obligation). `PredicateContext` gains a redacting `Debug`.

`ValidSchema::validate` / `ValidValues::resolve` remain the sole proof-token
mints in `nebula-schema`; this change is signature-invisible to
`ValidValues`/`ResolvedValues`. A seam test (`crates/schema/tests/seam_proof_token_custody.rs`)
asserts the tokens are constructible only via the pipeline. ADR + seam test
land in the same PR (canon §0.1/§17).

`RequiredMode::When` is wired correctly (nested-path-correct) end-to-end in
this PR; the prior fail-open is the §4.5 violation being closed.

## Consequences

- Breaking: `nebula_validator::RuleContext` and `Rule::evaluate` are removed.
  Only in-tree caller was `nebula-schema::validated`, migrated here.
- `crates/schema/src/context.rs` `RootContext`/`ObjectContext` are replaced by
  a `PredicateContext` builder.
- P2 (separate plan) deletes `run_rules`/`run_root_rules`/`validator_bridge.rs`
  and moves report assembly fully validator-side; P1 leaves `run_root_rules`.
- The `slot_bindings` confused-deputy (spec Non-goals) is untouched and
  unworsened — tracked separately.
