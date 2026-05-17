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
scrubs the root-rule context too — **done in the 2026-05-16 P2 amendment
below**). Until then the build-time
`secret.predicate_on_value` lint is the security boundary that stops a
value-comparing root predicate from reading secret plaintext, so its
secret-key collection must mirror every addressable shape (object, list-item
object including indexed instances, mode variant payload) and the predicate
target must be matched after list-index normalization. Seam anchors: the
`root_value_predicate_on_list_indexed_secret_is_rejected` and
`root_value_predicate_on_mode_secret_under_list_is_rejected` regression tests.

## Amendment (2026-05-16) — P2: validator sole emitter, root-rule scrub, single crossing

P2 lands the deferred moves. (1) The hidden+present+required+empty `required`
emission is **moved** from the `nebula-schema` field gate into
`resolve_field_policies` (the validator is now the sole `required` emitter for
both `Presence::Active` and the bounded non-`Active` carve-out); the behaviour
is preserved exactly (one `required` error for a hidden+present+required+empty
field) — the carve-out is moved, not deleted. `FieldPolicyDecl` now carries two
independent bits (`value_present` = not-absent-for-required, and `raw_present`
= a raw value is syntactically present); `FieldPlan` carries a ternary
directive so a hidden-but-present field is still structurally validated (a
smuggled expression in a no-payload mode-variant placeholder cannot escape to
resolve). (2) The root-rule predicate context is now built by the same
addressable-path traversal the `secret.predicate_on_value` lint uses, scrubbing
`Field::Secret` by schema type recursively (objects, list-item objects, mode
variant payloads); the root-rule path no longer evaluates predicates against
unscrubbed submitted JSON. The build-time `secret.predicate_on_value` lint is
retained as defence-in-depth (no longer the sole boundary). (3)
`run_rules`/`run_root_rules`, `validator_bridge.rs`'s error mapping, and
`translate_validator_code` are deleted; `nebula-schema` crosses into
`nebula-validator` only through `validate_rules_with_ctx` and
`resolve_field_policies`, and merges validator errors verbatim. Consequence:
the public validation error vocabulary for rule failures changes from the
schema `STANDARD_CODES` remap (`length.min`, `length.max`, `range.min`,
`range.max`, `pattern`, `email`, `url`) to the native validator vocabulary
(`min_length`, `max_length`, `min`, `max`, `invalid_format`).
`nebula-schema` is `frontier` / pre-1.0 (no UPGRADE_COMPAT contract), so this
is canon-legal; it ships as a breaking change with the seam test
`crates/schema/tests/flow/all_error_codes.rs` updated in the same PR.
Schema-owned structural codes (`type_mismatch`, `items.*`, `option.*`,
`mode.*`, `expression.*`, `required`) are unchanged. The lint-only path
helpers (`resolve_rule_dependency`/`referenced_root_key`/
`normalize_rule_target_path`) relocate to `crates/schema/src/rule_ref.rs`
(intra-crate; lint behaviour byte-identical). The `ValidSchema::validate` /
`ValidValues::resolve` signatures and proof-token custody
(INTEGRATION_MODEL §29/§33) are unchanged.

Seam anchors added/kept green in the P2 PR:
`hidden_present_required_empty_emits_single_required` (now sourced
validator-side), a new runtime regression for a hidden+present mode payload
with a smuggled expression, a new runtime root-rule scrub seam test (legal
non-secret nested root predicate still fires; secret plaintext unreadable),
the `root_value_predicate_on_*` lint anchors, `valid_values_only_minted_by_validate`,
a symbol-level single-crossing assertion, and the rewritten
`flow/all_error_codes.rs`.

## Amendment (2026-05-17) — P3: HasSchema convergence (Action/Credential ISP fold)

P3 of the recorded cascade converges the three business traits onto one
schema-access shape. `Action::input_schema()` / `output_schema()` (required
methods — ISP fat-interface redundancy: every in-tree body was an `OnceLock`
wrapping `<Self::Input as HasSchema>::schema()`, with zero custom overrides
and zero production callers; the real consumer `ActionMetadata::for_*::<A>`
already used the associated type directly) and `Credential::properties_schema()`
(a provided method whose body already was `<Self::Properties as
HasSchema>::schema()`) are **deleted, not deprecated** (no-shim discipline).
The `type Input` / `Output` / `Properties: HasSchema` associated-type bound
is the sole source of truth; `Resource` already had this clean shape and is
untouched (the convergence reference). A free
`nebula_schema::schema_of::<T: HasSchema>() -> ValidSchema` helper (ratified
shape per ADR-0061: owned, no object-safe companion) lets call sites avoid
restating the trait-qualified path; it is re-exported from `nebula-credential`
so `#[derive(Credential)]` emits a path resolvable without forcing plugin
authors onto a direct `nebula-schema` dependency. Behaviorally lossless (the
deleted bodies were pure redundancy); signature-invisible to the
`ValidSchema → ValidValues → ResolvedValues` proof-token pipeline
(INTEGRATION_MODEL §29/§33 unchanged). Object safety unaffected: `Action` is
`Sized` (never `dyn`; the erased path is `ErasedAction`/`ActionFactory` and
does not call the removed methods), and every `Credential` method is
`where Self: Sized` (outside any vtable). Breaking: the public trait surface
of `nebula-action` and `nebula-credential` loses three methods — canon-legal
because both are `frontier` / pre-1.0 (no UPGRADE_COMPAT contract); ships `!`
with the seam tests in the same PR. This amends ADR-0043 §4 (which defined
`input_schema` / `output_schema` as `= Self::Input::schema()`); a truthful
forward-pointer is added there. Zero new crates, zero `deny.toml` change
(`HasSchema` / `schema_of` stay in `nebula-schema` Core, already importable
by the three Business crates).

Seam anchors landed in the P3 PR:
`crates/action/tests/probes/action_input_schema_removed.rs` +
`crates/action/tests/seam_action_schema_method_removed.rs` (trybuild:
`<Probe as Action>::input_schema()` no longer resolves — `E0576`),
`crates/credential/tests/probes/credential_properties_schema_removed.rs` +
`compile_fail_credential_properties_schema_removed.rs` (trybuild:
`<NoCredential as Credential>::properties_schema()` no longer resolves —
`E0576`), the runtime convergence guards
`derive_action::input_schema_derives_from_input_via_schema_of` and
`properties_pipeline::metadata_schema_is_schema_of_properties`
(metadata schema == `schema_of::<Properties>()`), and the
`nebula-schema` `schema_of_equals_has_schema_schema` unit anchor.

P4 (API write-path validation V2 / catalog `json_schema()` V3 / public
OpenAPI DTO `x-nebula-root-rules` strip / ADR-0047 amendment) is the
remaining cascade phase, out of P3 scope.

## Amendment (2026-05-17) — P4: API write-path validation + catalog json_schema() + public projection (cascade close-out)

P4 closes the cascade. An api-owned, object-safe `CredentialSchemaPort`
(`crates/api/src/ports/credential_schema.rs`; api-safe types only —
`serde_json::Value` + api-owned structs, **no `ValidSchema` in any DTO**)
is added to `AppState` as `Option<Arc<dyn …>>`, mirroring the
`action_registry` precedent (absent ⇒ honest 503, canon §4.5). The
concrete `RegistryCredentialSchema`
(`crates/api/src/ports/credential_schema_registry.rs`) resolves
`credential_key → CredentialMetadata.base.schema` via a
`nebula_credential::CredentialRegistry`, runs `FieldValues::from_json` +
`ValidSchema::validate` (authority = validator; **never `.resolve()`** —
canon §12.5: credential `data` must not be expression-resolved against
workflow state; INTEGRATION_MODEL §29/§33 proof-token custody unchanged),
and exports `ValidSchema::json_schema()` for the catalog.

- **V2 (write-path).** `create_credential`/`update_credential` validate
  `data` against the type's resolved schema before persist. **This closed
  a verified live fail-open**: the path persisted `data` unvalidated while
  the handler docstring claimed it was validated (§4.5 [L1] + §10 [L2]).
  No port ⇒ 503 (never silent unvalidated persist). Rejection ⇒ the
  api-wide validation status (400) carrying only RFC-6901 path + validator
  code + static message — never a submitted value (ADR-0034; secret-safe
  by construction, P2's value-free `ValidationReport`).
- **V3 (catalog).** `list_credential_types`/`get_credential_type` are
  port-backed (no longer engine-owned-503): populated 200 when wired,
  honest 503 when not, 404 for an unknown (public) type key.
- **#6 (public projection).** An api-owned recursive mapper
  (`crates/api/src/domain/credential/schema_projection.rs`) strips
  `x-nebula-root-rules` + the predicate-bearing
  `x-nebula-{required,visibility}-mode` family from the catalog schema;
  standard JSON-Schema keywords and non-predicate structural hints are
  kept. Not a raw `json_schema()` passthrough.

**Six pre-#671 spec premises were false and are corrected here** (the
verify-first discipline, same as P1–P3): (1) the write path persisted
`data` unvalidated while documenting otherwise (a live fail-open, not a
benign TODO); (2) `schemars` was enabled by no crate; (3)
`crates/api`'s `nebula-schema` was a dev-dep only; (4) no
credential-registry port existed in `AppState` (`list/get_credential_type`
were honest 503s); (5) no `json_schema()`-over-a-port precedent (the
action catalog uses hand-wrapped `ToSchema` DTOs); (6) **"zero deny.toml
change" was infeasible as written** — post-#671 the composition root is a
separate `nebula-server` crate not in `nebula-credential`'s wrapper
allowlist. The user adjudicated #6: keep the harder "zero deny.toml
change" constraint and host the concrete impl in `nebula-api` (already an
allow-listed `nebula-credential` consumer; `nebula-schema` is Core-tier
with no `deny.toml` wrapper rule — freely importable, so **no `deny.toml`
change** is required).
Consequence: `nebula-api` takes a `nebula-schema` **production** dep +
the `schemars` feature. ADR-0047's actual DTO-purity rule (no lower-layer
**types in DTOs**) remains intact — the port returns only
`serde_json::Value`/api-owned structs; only ADR-0047's informal "api
never imports `nebula-schema`" prose is relaxed (amended in-place in
ADR-0047). Zero new crates; zero `deny.toml` change.

Seam anchors (this PR): `crates/api/tests/seam_credential_schema_port.rs`
(port object-safe + `Option`/builder), `…/seam_credential_write_path_validation.rs`
(V2: reject⇒400 secret-safe, valid⇒200, no-port⇒503-never-persist),
`…/seam_credential_catalog_schema.rs` (V3 populated + #6 strip + 404 +
no-port⇒503), `credential_schema::tests` projection unit tests,
`credential_schema_registry::tests` (validate reject/accept/unknown +
default port registers the first-party set). OpenAPI honesty tests
reconciled faithfully (type-discovery 503→200/404 reflects the new honest
reality — port present ⇒ truthful catalog; no-port⇒503 retained in the
seam test — never silenced).

**The ADR-0052 cascade (P1 #670 / P2 #672 / P3 #676 / P4 this PR) is
COMPLETE. There is no P5.** **Non-goal still OPEN:** the `slot_bindings`
confused-deputy — credential/resource resolution still has no
owner/tenant/workspace authorization; a crafted workflow JSON can resolve
any credential id. Credential resolution remains confused-deputy-exposed
after P4. "Cascade complete" is **NOT** "that is closed"; it is a
broader engine-authorization refactor tracked separately.
