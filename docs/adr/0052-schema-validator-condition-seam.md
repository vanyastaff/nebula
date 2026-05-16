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

## Amendment (2026-05-15) — hidden-but-present `required` emission seam

The validator policy engine owns the required-ness *decision*:
`resolve_field_policies` computes `Requiredness` for every field and emits
`required_failures` only for `Presence::Active`. The legacy safety contract is
preserved: a field is skipped only when hidden AND absent; a hidden field with
a present value is still structurally validated (so a smuggled `{{expr}}` in a
no-payload Mode-variant placeholder cannot escape to `resolve`). Consequently,
for the bounded corner `Presence != Active` ∧ value-present ∧
`Requiredness::Required` ∧ absent-for-required, the **schema gate** emits the
single `required` error, because the policy engine deliberately does not
self-report for non-`Active`. This is a split of emission *site*, not of
*ownership* (the validator remains the sole required-ness decision authority);
it is intentional and behaviour-required. A later phase that centralises
report assembly validator-side MUST **move** this emission into
`resolve_field_policies` (validator as sole emitter) while **preserving** the
behaviour — exactly one `required` error for a hidden+present+required+empty
field — and MUST NOT delete the carve-out. Seam anchor: the
`hidden_present_required_empty_emits_single_required` regression test.

For the root-rule path specifically, `run_root_rules` evaluates predicates
against the full submitted JSON with no `Field::Secret`-by-type scrub (the
field-level path uses the scrubbed `predicate_context_for`; a later phase
scrubs the root-rule context too). Until then the build-time
`secret.predicate_on_value` lint is the security boundary that stops a
value-comparing root predicate from reading secret plaintext, so its
secret-key collection must mirror every addressable shape (object, list-item
object including indexed instances, mode variant payload) and the predicate
target must be matched after list-index normalization. Seam anchors: the
`root_value_predicate_on_list_indexed_secret_is_rejected` and
`root_value_predicate_on_mode_secret_under_list_is_rejected` regression tests.
