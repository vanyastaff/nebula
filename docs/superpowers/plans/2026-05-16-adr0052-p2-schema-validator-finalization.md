# ADR-0052 P2 — schema↔validator seam finalization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the ADR-0052 schema↔validator seam: make `nebula-validator` the sole `required` emitter, scrub secrets from the root-rule predicate context, collapse schema→validator behavioral crossings to one symbol surface, make plan↔field desync type-unrepresentable, and add the no-payload-mode-variant expression lint.

**Architecture:** P1 moved field visibility/required evaluation into `nebula_validator::policy` (`resolve_field_policies`). P2 finishes the seam: (a) `FieldPolicyDecl`/`FieldPlan`/`FieldPolicyResolution` gain a generic opaque payload `P` threaded 1:1 so the schema runner reads `plan.payload` instead of a parallel `.zip(entries)`; (c) the hidden+present+required+empty `required` emission **moves** into `resolve_field_policies` (validator = sole emitter) via a ternary `FieldPlan` directive + two independent decl bits; (item 2) the root-rule predicate context is built by the **same** addressable-path traversal the secret lint uses, scrubbing `Field::Secret` recursively; (b) `run_rules`/`run_root_rules`/`validator_bridge.rs`'s error-mapping/`translate_validator_code` are deleted, the lint-only path helpers relocate, and schema crosses into the validator through exactly the `validate_rules_with_ctx` + `resolve_field_policies` symbols (errors flow verbatim — a documented public rule-code vocabulary change); (d) a new `mode.no_payload_variant_must_forbid_expression` schema lint.

**Tech Stack:** Rust 1.95 (edition 2024), `nebula-schema` + `nebula-validator` crates, `cargo nextest`, trybuild compile-fail tests, `task`/lefthook gate.

---

## Authority & context (read before Task 0)

- `docs/adr/0052-schema-validator-condition-seam.md` — **read in full incl. the Amendment (lines 59–89)**. The hidden+present `required` carve-out is intentional and behaviour-required; a later phase MUST **move** (not delete) it; the root-rule path runs predicates on the full submitted JSON with no `Field::Secret`-by-type scrub and "a later phase scrubs the root-rule context too".
- `docs/superpowers/specs/2026-05-15-nebula-schema-finalization-design.md` — Decision table, Phasing P2 (lines 326–336: **"signature/postcondition unchanged, INTEGRATION_MODEL §29/§33"** — binding), Plan-lockdown #1/#2/#3.
- `docs/superpowers/plans/2026-05-15-nebula-schema-finalization-p1.md` — the **P2 backlog** (≈ line 1707), items (a)–(e). This is the authoritative scope source.
- `AGENTS.md` / `CLAUDE.md` — branch/commit/PR rules are binding. Per-commit lefthook gate (workspace `clippy -D warnings` + per-crate fmt + typos): commit only at workspace-green points; coarse atomic commits. **Never** `cargo fmt --all`/`task fmt` (Windows path length on worktree paths) — use per-crate `cargo fmt -p <crate>`.

### P1 landed signatures this plan is written against (verified on `origin/main` e40dab42 / P1 = 90ce100c)

`crates/validator/src/policy/mod.rs`:
- `Presence { Active, Skipped }` `#[non_exhaustive]` `Copy`; `Requiredness { Required, Optional }` `#[non_exhaustive]` `Copy`.
- `VisibilityPolicy<'a> { Always, Never, When(&'a Rule) }`, `RequiredPolicy<'a> { Optional, Always, When(&'a Rule) }`, `#[non_exhaustive]`, each `fn resolve(&self, ctx: &PredicateContext) -> Presence/Requiredness`.
- `FieldPolicyDecl<'a> { path: &'a FieldPath, visibility: VisibilityPolicy<'a>, required: RequiredPolicy<'a>, value_present: bool }` `#[non_exhaustive]`, `fn new(path, visibility, required, value_present) -> Self`.
- `FieldPlan<'a> { path: &'a FieldPath, presence: Presence, requiredness: Requiredness }` `#[non_exhaustive]`.
- `FieldPolicyResolution<'a> { plans: Vec<FieldPlan<'a>>, required_failures: ValidationErrors }` `#[derive(Debug, Default)] #[non_exhaustive]`.
- `resolve_field_policies<'a, I: IntoIterator<Item = FieldPolicyDecl<'a>>>(decls: I, ctx: &PredicateContext) -> FieldPolicyResolution<'a>` — currently emits `required_failures` via `ValidationError::new("required", "field is required").with_field_path(d.path.clone())` **only** for `presence == Active && requiredness == Required && !value_present` (mod.rs:165–173). In-crate `#[cfg(test)] mod tests` (mod.rs:187–277) builds decls with struct literals (allowed in defining crate).
- `crates/validator/src/lib.rs:81,85-90` re-exports `ExecutionMode, validate_rules, validate_rules_with_ctx` and `policy::{FieldPlan, FieldPolicyDecl, FieldPolicyResolution, Presence, RequiredPolicy, Requiredness, VisibilityPolicy, resolve_field_policies}`.
- `crates/validator/src/engine.rs:67-82`: `validate_rules(value, rules, mode) == validate_rules_with_ctx(value, rules, None, mode)`; both `-> Result<(), ValidationErrors>`. `ExecutionMode { StaticOnly(default), Deferred, Full }` `#[non_exhaustive]`.

`crates/schema/src/validated.rs`:
- `ValidSchema::validate(&self, values:&FieldValues) -> Result<ValidValues, ValidationReport>` (333): builds `ctx = crate::context::predicate_context_for(&self.0.fields, values)` (342), top-level `entries: Vec<LevelEntry>` (347–361), `gate_and_validate_level(&entries, &ctx, &mut report)` (362), `run_root_rules(&self.0.root_rules, values, &mut report)` (364), mints `ValidValues` (380–384).
- `struct LevelEntry<'a> { field:&'a Field, raw:Option<&'a FieldValue>, schema_path:FieldPath, validator_path:nebula_validator::foundation::FieldPath }` (1072–1080).
- `fn gate_and_validate_level(entries:&[LevelEntry], ctx:&nebula_validator::PredicateContext, report:&mut ValidationReport)` (1096): `vis_policy`/`req_policy` mappers (1106–1119); `decls = entries.iter().map(|e| FieldPolicyDecl::new(&e.validator_path, vis_policy(e.field.visible()), req_policy(e.field.required()), !is_absent_for_required(e.field, e.raw)))` (1121–1128); `resolve_field_policies(decls, ctx)` (1129); `push_validator_rule_errors(&resolution.required_failures, &FieldPath::root(), report)` (1135); `debug_assert_eq!(resolution.plans.len(), entries.len())` (1144); `for (plan, entry) in resolution.plans.iter().zip(entries)` (1150); `hidden = !matches!(plan.presence, Presence::Active)` (1155); `if hidden && entry.raw.is_none() { tracing::debug!; continue }` (1162–1176); **the carve-out** `if plan.requiredness == Requiredness::Required && is_absent_for_required(entry.field, entry.raw) { if hidden { report.push(ValidationError::builder("required").at(entry.schema_path.clone()).message(format!("field `{}` is required", entry.schema_path)).build()) } tracing::debug!; continue }` (1184–1206); else `tracing::debug!; validate_field(entry.field, entry.raw, &entry.schema_path, ctx, report)` (1210–1218).
- 4 `gate_and_validate_level` call sites: **362** (top-level), **1573** (List items), **1609** (Object children), **1707** (Mode payload, single-entry array).
- 11 `run_rules(...)` call sites: 1310, 1348, 1364, 1380, 1410, 1464, 1503, 1592, 1672, 1767, 1770 — all inside `validate_literal_value` (per `Field` variant).
- `fn run_rules(rules:&[Rule], value:&serde_json::Value, path:&FieldPath, report:&mut ValidationReport)` (1895) → `validate_rules(value, rules, ExecutionMode::StaticOnly)` → `push_validator_rule_errors`.
- `fn run_root_rules(rules:&[Rule], values:&FieldValues, report:&mut ValidationReport)` (1908): `json = values.to_json(); pred_ctx = PredicateContext::from_json(&json); validate_rules_with_ctx(&json, rules, Some(&pred_ctx), StaticOnly)` → `push_validator_rule_errors(&errs, &FieldPath::root(), report)`. **`PredicateContext::from_json(&values.to_json())` (1919–1920) is the UNSCRUBBED root context — the item-2 hole.**
- `fn translate_validator_code(raw_code:&str, params:&[(Cow,Cow)]) -> String` (1861–1892): `min_length→length.min`, `max_length→length.max`, `min→range.min`, `max→range.max`, `invalid_format→pattern|email|url` (param-driven), `other→other`.
- `fn push_validator_rule_errors(errs:&nebula_validator::foundation::ValidationErrors, path:&FieldPath, report:&mut ValidationReport)` (1931): per error `code = translate_validator_code(e.code, e.params()); issue_path = schema_path_from_validator_error(path, e); report.push(ValidationError::builder(code).at(issue_path).message(msg).build())`.
- `fn is_absent_for_required(field:&Field, raw:Option<&FieldValue>) -> bool` (1029–1053): null→true; empty string for String/Secret/Code→true; empty File/List/multi-Select→true; **no `Field::Mode` arm → `_ => false`** (so a hidden Mode with a present payload has `value_present == true`; the runner only avoids the escape today via the separate `entry.raw.is_none()` bit at 1162).
- `fn validate_field(field, raw, path, ctx, report)` (1225): `let Some(value)=raw else {return};` then `FieldValue::Expression` ⇒ `match field.expression() { Forbidden ⇒ push "expression.forbidden"; Allowed|Required ⇒ expr.parse_at(path) }` then `return`; `if matches!(field.expression(), Required) ⇒ push "expression.required"; return;` else `validate_literal_value`.
- `fn validator_path_from_schema_path(path:&FieldPath) -> nebula_validator::foundation::FieldPath` (1060).
- `use crate::validator_bridge::schema_path_from_validator_error;` at validated.rs:29.

`crates/schema/src/validator_bridge.rs` (`pub(crate) mod validator_bridge;` at `lib.rs:166`):
- Lint-only pure helpers (NO rule execution): `resolve_rule_dependency(&str)->Option<FieldPath>` (12), `referenced_root_key(&str)->Option<FieldKey>` (28), `normalize_rule_target_path(&FieldPath)->FieldPath` (40), priv `validator_path_to_schema_path`, priv `field_path_from_json_pointer`, priv `decode_json_pointer_segment` + a `#[cfg(test)] mod tests`.
- Error-mapping helper (rule-execution seam): `schema_path_from_validator_error(&FieldPath, &nebula_validator::foundation::ValidationError)->FieldPath` (53).
- **Consumers:** `lint.rs:12` imports `{normalize_rule_target_path, referenced_root_key, resolve_rule_dependency}` used at lint.rs 80, 93, 637, 932, 935, 1055, 1173, 1174. `validated.rs:29,1940` uses `schema_path_from_validator_error`. ⇒ the file cannot be deleted wholesale; the lint helpers **relocate**, only `schema_path_from_validator_error` is deleted.

`crates/schema/src/context.rs`:
- `pub fn predicate_context_for(fields:&[Field], values:&FieldValues)->PredicateContext` (34) → `collect_non_secret(fields, values.as_map(), None, &mut pairs); PredicateContext::from_fields(pairs)`.
- `fn collect_non_secret(fields, values:&IndexMap<FieldKey,FieldValue>, prefix:Option<&FieldPath>, out:&mut Vec<(FieldPath,Value)>)` (45–80): skip `Field::Secret`; `(Object,Object)` recurse; `(Object|List|Mode, _) => {}` (structured-typed non-matching = non-addressable); `(_, Literal(v)) => out.push((path, v.clone()))`; else skip. **Does NOT descend `Field::List` or `Field::Mode`** — coarser than the lint's traversal (item-2 fail-open risk if reused verbatim for root rules).

`crates/schema/src/lint.rs`:
- `pub(crate) fn lint_tree(fields:&[Field], prefix:&FieldPath, report:&mut ValidationReport)` (29); `pub(crate) fn lint_root_rules(rules, fields, report)` (52); per-field dispatch incl. `Field::Mode` arms (143, 241, 1121); `fn lint_mode_new(...)` (492); `secret.default_forbidden` emitted in `lint_default_type` (173, `ValidationError::builder("secret.default_forbidden")`).
- `fn collect_secret_pointer_segments(fields:&[Field])->HashSet<Vec<String>>` (1101–1150): inner `walk_field`/`walk_scope` — descends `Field::Object` (walk_scope on `obj.fields`), `Field::List` whose `list.item` is `Field::Object` (children under the **list path**, anonymous items), `Field::Mode` (each `variant`, payload visited *at* `segs + variant.key`). **This is the canonical addressable-path traversal item-2 must share.**
- `const fn predicate_reads_value(&Predicate)->bool` (1155): `!matches!(Set|Empty)`.
- `fn normalized_predicate_key_segments(&Predicate)->Option<Vec<String>>` (1172) (uses `resolve_rule_dependency`+`normalize_rule_target_path`); `fn walk_rule_for_secret_value_predicates(rule, secrets, path, report)` (1192); `fn lint_secret_predicate_on_value(fields, report)` (1251) → `collect_secret_pointer_segments` + `walk_schema_fields(fields, |node| { field_visible_rule(node.field); field_required_rule(node.field) })` (1257). `const fn field_visible_rule(&Field)->Option<&Rule>` (766), `field_required_rule` (773).

`crates/schema/src/mode.rs` + `field.rs`:
- `pub enum ExpressionMode { Forbidden, Allowed, Required }` `#[non_exhaustive]` `Default = Allowed` (mode.rs:51–62).
- `pub struct ModeVariant { pub key:String, pub label:String, pub field:Box<Field> }` `#[non_exhaustive]` (field.rs:600–609).
- `ModeField::EMPTY_PLACEHOLDER_KEY = "_nebula_mode_empty"` (field.rs:613); `ModeField::variant_empty(key,label)` builds `field = Field::try_string(EMPTY_PLACEHOLDER_KEY).visible(VisibilityMode::Never).no_expression()…` (field.rs:641–649) — i.e. the canonical no-payload variant placeholder is keyed `EMPTY_PLACEHOLDER_KEY`.
- `Field::expression(&self)->&ExpressionMode` (field.rs:1186); `Field::Mode(ModeField{ variants, default_variant, rules, .. })`.

### Adjudicated forks (from the 4-seat hostile panel; output is a proposal, scope-checked against the backlog — these are decided)

- **F1 (item a):** generic opaque payload `P` on `FieldPolicyDecl`/`FieldPlan`/`FieldPolicyResolution`, `P = &'a LevelEntry` schema-side. Validator stays schema-agnostic. **`#[derive(Default)]` on `FieldPolicyResolution` must be removed** (`&LevelEntry: !Default`) → hand-rolled `P`-free constructor. Restated INVARIANT (cross-wiring → type-unrepresentable; residual = omission, keep "never filter/dedupe plans"). Visitor-callback that also kills omission = future, **out of P2**.
- **F2+F3 (items b/3):** `ValidSchema::validate` signature & `ValidationReport` ownership **unchanged** (binding: design-spec Phasing P2). Schema keeps emitting its own structural codes (`type_mismatch`/`items.*`/`option.*`/`mode.*`/`expression.*`/`required` — validator has no equivalent). Only `translate_validator_code` is deleted ⇒ validator **rule** codes flow verbatim (`min_length`/`invalid_format`/… instead of `length.min`/`pattern`/…). Scoped public breaking change for 7 detail rule codes — `frontier`/pre-1.0 ⇒ canon-legal but requires `!` commit + ADR-0052 amendment text + `flow/all_error_codes.rs` rewritten **in this PR** + README/`STANDARD_CODES` rustdoc updated. "Single crossing" = **symbol-level** assertion ("schema calls no `nebula_validator` evaluation fn other than `validate_rules_with_ctx` and `resolve_field_policies`"), not "one runtime call". Full "schema returns `validator::Errors`" = **P3+, rejected here**.
- **F4 (item c):** **fail-open if naive.** `FieldPolicyDecl` needs **two independent bits**: `value_present` (= `!is_absent_for_required`) AND `raw_present` (= `entry.raw.is_some()`). `FieldPlan` exposes a **ternary directive** (`Skip` / `RequiredAbsent` / `Validate`) where `Validate` is reachable **while hidden** (hidden+present must still be structurally validated — ADR Amendment line 66; else a smuggled `{{$expr}}` in a hidden Mode payload escapes to `resolve`). New runtime regression required (the existing `validate_schema.rs:726` anchor is blind to the Mode-expr vector — P1-class trap).
- **F4 (item 2):** **BLOCK the naive `predicate_context_for` swap** — it doesn't descend List/Mode ⇒ legal non-secret nested root predicates fail **OPEN**; the `root_value_predicate_on_*` anchors are build-time lint tests, blind to this runtime regression (P1-class trap). Build the scrubbed root context with the **same traversal** as `collect_secret_pointer_segments` (single owner of the addressable-path invariant); add a runtime seam test for a legal nested non-secret root predicate; keep `secret.predicate_on_value` lint as additive defence; only then mark ADR Amendment line 81 done.
- **F4 (item d):** keep `mode.no_payload_variant_must_forbid_expression` a `Severity::Error` lint (backlog-faithful; `Schema::builder().build()` is the only mint path ⇒ a build-fatal lint is fail-closed via the proof-token typestate). Add a runtime regression proving the realistic smuggle (`{"$expr":…}` → `FieldValue::Expression` → `expression.forbidden`) is also rejected at runtime. Verify-first whether a genuine runtime escape exists with the lint absent; if confirmed unreachable past `build()`, lint + regression is sufficient and the reasoning goes in the ADR amendment.

### Scope (P2 = backlog (a)–(e) + item-2 root scrub + scoped `translate_validator_code` deletion)

In scope: Tasks 1–5 below. **Out of scope, flagged (do NOT do):**
- `0042`/`0052` ADR filename collisions (`0042-layered-retry` vs `0042-node-binding-mechanism`; `0052-action-surface-hybrid` vs `0052-schema-validator-condition-seam`) — separate housekeeping per spec lines 313–314/435. Flag in the PR body, do not fix.
- `slot_bindings` confused-deputy — spec Non-goal.
- `derive_schema.rs:95/:126`, JSON-Schema secret-default refusal (#3), bounded+cached regex (#4) — the design-spec's loose "P2 — schema cleanup" prose bundled these, but the **recorded P2 backlog dropped them**; the backlog is authoritative (task instruction + feedback_adr_revisable). Out of P2; note explicitly in the PR body so they are not mistaken for forgotten.
- The "schema returns `nebula_validator::Errors` / validator owns the report container" maximal refactor — P3+ (contradicts the pinned P2 signature).

### Cross-cutting merge-blockers (verify in the Task 6 gate; each maps to a panel must-do)

1. `crates/schema/tests/validate_schema.rs::hidden_present_required_empty_emits_single_required` green with the `required` error **sourced from `resolve_field_policies`** (moved, not deleted; exactly one; code `required`; path `secret_slot`).
2. New runtime regression: hidden `Field::Mode` variant + submitted `{"mode":<v>,"value":{"$expr":"…"}}` ⇒ `expression.forbidden` (carve-out fold preserves "hidden+present still structurally validated"). [F4 C3]
3. `crates/schema/tests/lint_and_loader.rs::root_value_predicate_on_list_indexed_secret_is_rejected` + `…_mode_secret_under_list_is_rejected` green; PLUS a new **runtime** seam test: a legal non-secret nested root predicate (`/items/0/region == "eu"` ⇒ require sibling) still fires under `.validate()` after the scrub; PLUS a runtime test that a root predicate cannot read scrubbed secret plaintext. [F4 I1–I3]
4. `crates/schema/tests/seam_proof_token_custody.rs::valid_values_only_minted_by_validate` green unchanged. [F3]
5. Symbol-level "single crossing" assertion test added and green. [F2/backlog b]
6. `crates/schema/tests/flow/all_error_codes.rs` rewritten to validator vocabulary **in this PR**; codes `required`/`expression.forbidden`/`expression.required`/`type_mismatch` pinned unchanged by a dedicated test. [F3/F4 V1]
7. trybuild compile-fail: `FieldPolicyResolution` plan cannot be reordered to desync field (payload is in the plan); `Presence` match still must handle non-`Active`. [F1]
8. ADR-0052 amendment + the seam tests land **in this PR**; `!` breaking commit; README/rustdoc updated. Full Task 6 gate green and confirmed before squash-merge.

## File structure

- Modify: `crates/validator/src/policy/mod.rs` — generic `P`; ternary `FieldDirective`; two decl bits; sole-emitter fold; non-`Default` resolution ctor; in-crate tests.
- Modify: `crates/schema/src/validated.rs` — `gate_and_validate_level` consumes `plan.payload` + `plan.directive` (dumb dispatcher, no `required` builder); delete `run_rules`/`run_root_rules`/`translate_validator_code`/`push_validator_rule_errors`; route value rules + scrubbed root rules through `validate_rules_with_ctx`; 4 call sites; delete the bridge import.
- Create: `crates/schema/src/rule_ref.rs` — relocated pure path helpers (`resolve_rule_dependency`/`referenced_root_key`/`normalize_rule_target_path` + privs + their tests).
- Delete: `crates/schema/src/validator_bridge.rs`.
- Modify: `crates/schema/src/context.rs` — add `root_predicate_context_for` built on a shared addressable-path traversal.
- Modify: `crates/schema/src/lint.rs` — `collect_secret_pointer_segments` refactored onto the shared traversal; new `lint_mode_no_payload_variant_must_forbid_expression`; import the relocated `rule_ref` helpers.
- Modify: `crates/schema/src/lib.rs` — `mod` lines (`validator_bridge` → `rule_ref`).
- Modify: `docs/adr/0052-schema-validator-condition-seam.md` — P2 amendment.
- Modify: `crates/schema/README.md`, `STANDARD_CODES` rustdoc — vocabulary change note.
- Tests: `crates/schema/tests/{validate_schema.rs, lint_and_loader.rs, seam_proof_token_custody.rs, flow/all_error_codes.rs}`, new `crates/schema/tests/seam_single_crossing.rs`, new `crates/schema/tests/compile_fail/*` (+ register in `compile_fail.rs`), validator `policy/mod.rs` `#[cfg(test)]`.

---

### Task 0: Branch baseline, ADR-0052 amendment, plan doc

**Files:**
- Modify: `docs/adr/0052-schema-validator-condition-seam.md`
- This plan: `docs/superpowers/plans/2026-05-16-adr0052-p2-schema-validator-finalization.md`

- [ ] **Step 1: Confirm worktree + baseline.** Worktree `C:\Users\vanya\RustroverProjects\nebula\.worktrees\adr0052-p2`, branch `refactor/schema-adr0052-p2` (base `origin/main`). Run `cargo check -p nebula-schema -p nebula-validator --all-targets` — expect clean (P1 is merged green). If it fails, STOP and report (do not build on a red baseline).

- [ ] **Step 2: Append the P2 amendment to ADR-0052.** Add a new section after the existing Amendment. Content (exact prose; adjust only file:line if drift found):

```markdown
## Amendment (2026-05-16) — P2: validator sole emitter, root-rule scrub, single crossing

P2 lands the deferred moves. (1) The hidden+present+required+empty `required`
emission is **moved** from the `nebula-schema` field gate into
`resolve_field_policies` (the validator is now the sole `required` emitter for
both `Presence::Active` and the bounded non-`Active` carve-out); the behaviour
is preserved exactly (one `required` error for a hidden+present+required+empty
field) — the carve-out is moved, not deleted. `FieldPolicyDecl` now carries two
independent bits (`value_present` = not-absent-for-required, and `raw_present`
= a raw value is syntactically present); `FieldPlan` carries a ternary
directive so a hidden-but-present field is still structurally validated
(a smuggled expression in a no-payload mode-variant placeholder cannot escape
to resolve). (2) The root-rule predicate context is now built by the same
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
schema `STANDARD_CODES` remap (`length.min`, `pattern`, `email`, `url`,
`range.min/max`) to the native validator vocabulary (`min_length`,
`max_length`, `min`, `max`, `invalid_format`). `nebula-schema` is `frontier`
/ pre-1.0 (no UPGRADE_COMPAT contract), so this is canon-legal; it ships as a
breaking change with the seam test `flow/all_error_codes.rs` updated in the
same PR. Schema-owned structural codes (`type_mismatch`, `items.*`,
`option.*`, `mode.*`, `expression.*`, `required`) are unchanged. The
`ValidSchema::validate` / `ValidValues::resolve` signatures and proof-token
custody (INTEGRATION_MODEL §29/§33) are unchanged.
```

Also update the prior Amendment's line "a later phase scrubs the root-rule context too" → append " (done in the 2026-05-16 P2 amendment)".

- [ ] **Step 3: Commit (docs-only, green).**

```bash
git -C C:/Users/vanya/RustroverProjects/nebula/.worktrees/adr0052-p2 add docs/adr/0052-schema-validator-condition-seam.md docs/superpowers/plans/2026-05-16-adr0052-p2-schema-validator-finalization.md
bash scripts/worktree.sh commit docs adr "ADR-0052 P2 amendment + P2 implementation plan"
```
Expected: `convco`-valid `docs(adr): ADR-0052 P2 amendment + P2 implementation plan`.

---

### Task 1: Generic opaque payload on policy types (backlog a / F1)

**Files:**
- Modify: `crates/validator/src/policy/mod.rs`
- Modify: `crates/validator/src/lib.rs:85-88` (re-export unchanged names; verify still compile)
- Modify: `crates/schema/src/validated.rs:1096-1219` (`gate_and_validate_level`)
- Test: `crates/validator/src/policy/mod.rs` `#[cfg(test)]`; `crates/schema/tests/compile_fail/` + `compile_fail.rs`

> This task changes a cross-crate public signature; `nebula-schema` will not compile until its consumer is updated in the **same commit** (lefthook runs workspace `clippy -D warnings` per commit). Do all edits, then `cargo check -p nebula-validator -p nebula-schema`, then the gate, then one commit.

- [ ] **Step 1: Write the failing validator unit test** (append to `policy/mod.rs` tests):

```rust
#[test]
fn payload_is_threaded_one_to_one_into_plans() {
    use crate::rule::context::PredicateContext;
    let ctx = PredicateContext::from_json(&serde_json::json!({}));
    let p0 = FieldPath::parse("a").unwrap();
    let p1 = FieldPath::parse("b").unwrap();
    let decls = vec![
        FieldPolicyDecl::new(&p0, VisibilityPolicy::Always, RequiredPolicy::Optional, true, true, "A"),
        FieldPolicyDecl::new(&p1, VisibilityPolicy::Never, RequiredPolicy::Optional, false, false, "B"),
    ];
    let res = resolve_field_policies(decls, &ctx);
    assert_eq!(res.plans.len(), 2);
    assert_eq!(res.plans[0].payload, "A");
    assert_eq!(res.plans[1].payload, "B");
}
```

(Signature `FieldPolicyDecl::new(path, visibility, required, value_present, raw_present, payload)` is finalized in Task 2; for Task 1 use the 5-arg form `new(path, visibility, required, value_present, payload)` and adjust this test's `new(...)` call to match — Task 2 adds `raw_present`. To avoid churn, **do Task 1 and Task 2 as one combined commit** if executing inline; if subagent-per-task, Task 1 uses 5-arg `new` and Task 1's test omits the 5th `false`/`raw_present` arg accordingly.)

- [ ] **Step 2: Run it — expect FAIL** (`payload` field/arg does not exist):

```
cargo test -p nebula-validator --lib policy::tests::payload_is_threaded_one_to_one_into_plans
```
Expected: compile error `no field `payload``.

- [ ] **Step 3: Make the policy types generic.** In `policy/mod.rs`:
  - `pub struct FieldPolicyDecl<'a, P> { pub path:&'a FieldPath, pub visibility:VisibilityPolicy<'a>, pub required:RequiredPolicy<'a>, pub value_present:bool, pub payload:P }` (keep `#[non_exhaustive]`).
  - `impl<'a, P> FieldPolicyDecl<'a, P> { #[must_use] pub fn new(path:&'a FieldPath, visibility:VisibilityPolicy<'a>, required:RequiredPolicy<'a>, value_present:bool, payload:P) -> Self {...} }`
  - `pub struct FieldPlan<'a, P> { pub path:&'a FieldPath, pub presence:Presence, pub requiredness:Requiredness, pub payload:P }` (keep `#[non_exhaustive]`).
  - `pub struct FieldPolicyResolution<'a, P> { pub plans:Vec<FieldPlan<'a, P>>, pub required_failures:ValidationErrors }` — **remove `#[derive(Default)]`** (keep `#[derive(Debug)]` only if `P: Debug`; drop `Debug` derive too and `#[non_exhaustive]` stays). Replace the `FieldPolicyResolution::default()` call in `resolve_field_policies` with an explicit `FieldPolicyResolution { plans: Vec::new(), required_failures: ValidationErrors::default() }` (verify `ValidationErrors: Default` — it is; it was `.add`-ed into at mod.rs:169).
  - `pub fn resolve_field_policies<'a, P, I>(decls:I, ctx:&PredicateContext) -> FieldPolicyResolution<'a, P> where I: IntoIterator<Item = FieldPolicyDecl<'a, P>>` — push `payload: d.payload` into each `FieldPlan`.
  - Restate the INVARIANT doc-comment (mod.rs:148-155): cross-wiring is now type-enforced (the payload is minted into the plan it was computed for); the residual risk is **omission** — callers MUST NOT filter/dedupe/drop `plans` (a dropped plan = a field silently never validated). Remove the now-false "positional `plans[i]`↔`decls[i]`" rationale.
  - Update the existing in-crate tests (mod.rs:204-276): struct-literal decls gain `payload: ()` (or a marker); `FieldPolicyResolution` construction sites updated.

> VERIFY before coding (name confirmation): `ValidationErrors: Default` and `ValidationError::new(_, _).with_field_path(_)` exact names at `crates/validator/src/foundation/` — already used at mod.rs:169-172, copy that exact form.

- [ ] **Step 4: Update `gate_and_validate_level` to consume `plan.payload`.** In `validated.rs:1096-1219`:
  - The decl closure (1121-1128) passes `payload: e` (i.e. `&LevelEntry`): `FieldPolicyDecl::new(&e.validator_path, vis_policy(e.field.visible()), req_policy(e.field.required()), !is_absent_for_required(e.field, e.raw), e)`.
  - Delete the `debug_assert_eq!` (1144-1148) and the `.zip(entries)` (1150). New loop: `for plan in &resolution.plans { let entry = plan.payload; <existing hidden/carve-out/validate_field body, replacing `entry.` field accesses unchanged> }`.
  - The 4 call sites (362, 1573, 1609, 1707) pass `&entries` unchanged (the generic is inferred). Confirm each still compiles.

- [ ] **Step 5: Add the trybuild compile-fail proof.** Create `crates/schema/tests/compile_fail/plan_payload_not_separable.rs`:

```rust
//! ADR-0052 P2: the field reference is carried *inside* the plan; there is no
//! parallel decls/entries collection to desync. A reordered `plans` carries
//! its payload with it, so positional cross-wiring is unrepresentable.
fn main() {
    // FieldPlan has no public constructor; a runner cannot fabricate a plan
    // pointing at a different field's payload.
    let _ = nebula_validator::policy::FieldPlan {
        path: todo!(),
        presence: todo!(),
        requiredness: todo!(),
        payload: todo!(),
    };
}
```

Register in `crates/schema/tests/compile_fail.rs`: `t.compile_fail("tests/compile_fail/plan_payload_not_separable.rs");`. Expected trybuild failure: cannot construct `#[non_exhaustive]` `FieldPlan` outside `nebula-validator`.

> VERIFY: read `crates/schema/tests/compile_fail.rs` for the exact registration idiom (`trybuild::TestCases`); mirror the two P1 cases already there.

- [ ] **Step 6: Verify.** `cargo test -p nebula-validator --lib policy::tests` (PASS), `cargo check -p nebula-schema --all-targets` (clean), `cargo test -p nebula-schema --test compile_fail` (the new case fails-to-compile as expected).

- [ ] **Step 7: Two-stage review** (spec compliance: payload threaded 1:1, `Default` removed, INVARIANT restated; then code quality) → fix-loop until clean.

- [ ] **Step 8: Per-crate fmt + commit.**

```bash
cargo fmt -p nebula-validator
cargo fmt -p nebula-schema
git add crates/validator/src/policy/mod.rs crates/validator/src/lib.rs crates/schema/src/validated.rs crates/schema/tests/compile_fail.rs crates/schema/tests/compile_fail/plan_payload_not_separable.rs
bash scripts/worktree.sh commit refactor validator "thread opaque field payload through policy plans (ADR-0052 P2)"
```
(Conventional `refactor(validator)!:` — `FieldPolicyDecl::new`/`FieldPlan` are public-breaking; the `!` is carried by the PR title and is acceptable here.) Run `cargo clippy --workspace --all-targets -q -- -D warnings` BEFORE commit (lefthook will).

---

### Task 2: Two decl bits + ternary directive; validator sole `required` emitter (backlog c / F4 C1–C3)

**Files:**
- Modify: `crates/validator/src/policy/mod.rs`
- Modify: `crates/schema/src/validated.rs:1096-1219`
- Test: `crates/schema/tests/validate_schema.rs`, new `crates/schema/tests/seam_required_emitter.rs`, `policy/mod.rs` `#[cfg(test)]`

- [ ] **Step 1: Pin the existing behaviour — read `validate_schema.rs:726-758` (`hidden_present_required_empty_emits_single_required`).** It MUST stay byte-identical and green; the only change is the error now originates in `resolve_field_policies`.

- [ ] **Step 2: Write the failing runtime regression** (the F4 escape the existing anchor is blind to). New file `crates/schema/tests/seam_required_emitter.rs`:

```rust
//! ADR-0052 P2 seam: validator is the sole `required` emitter, and a hidden
//! field that nonetheless carries a present value is STILL structurally
//! validated (a smuggled expression in a no-payload mode-variant placeholder
//! must not escape to resolve). The carve-out is moved, not deleted.
use nebula_schema::{Field, FieldValues, Schema, field_key};
use nebula_schema::mode::{ExpressionMode, VisibilityMode};
use serde_json::json;

#[test]
fn hidden_mode_present_expr_payload_is_rejected_not_skipped() {
    // A Mode field, hidden (Never). Variant `flag` is no-payload. An attacker
    // submits an expression payload to the hidden mode. Pre-P2 the runner
    // reached validate_field via the raw.is_some() branch and rejected the
    // expression. Post-fold (validator sole emitter) the hidden+present field
    // MUST still be structurally validated → expression.forbidden.
    let schema = Schema::builder()
        .add(
            Field::mode(field_key!("auth"))
                .visible(VisibilityMode::Never)
                .variant_empty("flag", "Flag"),
        )
        .build()
        .expect("schema builds");

    let values = FieldValues::from_json(json!({
        "auth": { "mode": "flag", "value": { "$expr": "{{ $secrets.leak }}" } }
    }))
    .unwrap();

    let report = schema.validate(&values).expect_err("must reject");
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "hidden+present mode payload must still be structurally validated, got: {:?}",
        report.errors().map(|e| (e.code.to_string(), e.path.to_string())).collect::<Vec<_>>()
    );
}
```

> VERIFY before coding (name confirmation, not design gap): `Field::mode`, `field_key!`, `.variant_empty`, `nebula_schema::mode::{ExpressionMode,VisibilityMode}` public paths — confirm against `crates/schema/src/lib.rs` re-exports + `field.rs:620,641`. Adjust imports to the real public path; do NOT change the asserted behaviour.

- [ ] **Step 3: Run it — expect FAIL** after a *naive* fold (it must be written to fail if the fold is naive). First run against current code to confirm it PASSES today (proves the invariant exists), note that, then proceed — the regression's job is to stay green through the fold.

```
cargo test -p nebula-schema --test seam_required_emitter
```
Expected NOW: PASS (current runner reaches `validate_field`). After Step 4 it must still PASS.

- [ ] **Step 4: Add the second bit + ternary; fold the emission.** In `policy/mod.rs`:
  - `FieldPolicyDecl<'a,P>` gains `pub raw_present: bool`; `new(path, visibility, required, value_present, raw_present, payload)`.
  - Add `#[derive(Debug,Clone,Copy,PartialEq,Eq)] #[non_exhaustive] pub enum FieldDirective { Skip, RequiredAbsent, Validate }`.
  - `FieldPlan<'a,P>` gains `pub directive: FieldDirective` (keep `presence`/`requiredness` for diagnostics).
  - `resolve_field_policies`: per decl compute `presence`/`requiredness`; `let active = presence == Presence::Active;`
    - emit one `required` ValidationError when `requiredness == Required && !value_present && (active || raw_present)` — i.e. **both** the `Active` case and the hidden-but-syntactically-present carve-out.
    - `directive = if !active && !d.raw_present { Skip } else if requiredness == Required && !value_present { RequiredAbsent } else { Validate }` — note `Validate` is reachable while hidden (hidden ∧ raw_present ∧ not-required-absent).
    - push `FieldPlan { path, presence, requiredness, directive, payload }`.
  - Update in-crate tests for the new arg + assert the carve-out emits exactly one `required` for `Never`+`Required`+`raw_present`+`!value_present`.
- [ ] **Step 5: Make the schema runner a dumb dispatcher.** In `gate_and_validate_level` (validated.rs), replace the hidden/carve-out body (1155-1218) with:

```rust
for plan in &resolution.plans {
    let entry = plan.payload;
    match plan.directive {
        nebula_validator::policy::FieldDirective::Skip => {
            tracing::debug!(target: "nebula_schema::validate", field=%entry.schema_path, presence=?plan.presence, requiredness=?plan.requiredness, decision="skipped", "field-gate decision");
        }
        nebula_validator::policy::FieldDirective::RequiredAbsent => {
            tracing::debug!(target: "nebula_schema::validate", field=%entry.schema_path, presence=?plan.presence, requiredness=?plan.requiredness, decision="required-emitted", "field-gate decision");
        }
        nebula_validator::policy::FieldDirective::Validate => {
            tracing::debug!(target: "nebula_schema::validate", field=%entry.schema_path, presence=?plan.presence, requiredness=?plan.requiredness, decision="value-validated", "field-gate decision");
            validate_field(entry.field, entry.raw, &entry.schema_path, ctx, report);
        }
    }
}
```
  **Delete** the gate-side `ValidationError::builder("required")…` (old 1187-1194). `is_absent_for_required` / `entry.raw.is_some()` are now computed only in the **decl closure** to set `value_present` / `raw_present` (schema-owned emptiness semantics fed to the validator as data; the validator decides + emits). `required_failures` continue to flow via `push_validator_rule_errors` (until Task 4 deletes it; then via the unified merge).
  Decl closure: `FieldPolicyDecl::new(&e.validator_path, vis_policy(e.field.visible()), req_policy(e.field.required()), !is_absent_for_required(e.field, e.raw), e.raw.is_some(), e)`.

- [ ] **Step 6: Verify.**
```
cargo test -p nebula-schema --test validate_schema hidden_present_required_empty_emits_single_required
cargo test -p nebula-schema --test seam_required_emitter
cargo test -p nebula-validator --lib policy::tests
cargo test -p nebula-schema --test validate_schema
```
Expected: all PASS (carve-out moved, exactly-one `required`, hidden+present still validated). If `seam_required_emitter` fails → the directive collapsed `Validate`-while-hidden into `Skip` (the F4 fail-open) — fix the ternary, do not weaken the test.

- [ ] **Step 7: Adversarial review.** Re-engage the security panelist (verify-first): does any (presence, requiredness, value_present, raw_present) combination drop a required error, double-emit, or skip structural validation of a hidden+present field? Build a truth table; assert each row against `resolve_field_policies`. Fix-loop until clean (hold the P1 bar: assume the first fold is incomplete).

- [ ] **Step 8: Per-crate fmt + commit.**
```bash
cargo fmt -p nebula-validator
cargo fmt -p nebula-schema
git add crates/validator/src/policy/mod.rs crates/schema/src/validated.rs crates/schema/tests/seam_required_emitter.rs
bash scripts/worktree.sh commit refactor schema "validator is sole required emitter; ternary field directive (ADR-0052 P2)"
```

---

### Task 3: Shared addressable-path traversal + scrubbed root-rule context (item 2 / F4 I1–I3)

**Files:**
- Modify: `crates/schema/src/lint.rs` (refactor `collect_secret_pointer_segments` onto a shared walker)
- Modify: `crates/schema/src/context.rs` (add `root_predicate_context_for`)
- Modify: `crates/schema/src/validated.rs` (`run_root_rules` uses the scrubbed context — interim, deleted in Task 4 preserving the scrub)
- Test: new `crates/schema/tests/seam_root_rule_scrub.rs`

- [ ] **Step 1: Write the failing runtime seam tests.** New file `crates/schema/tests/seam_root_rule_scrub.rs`:

```rust
//! ADR-0052 P2 item-2: root-rule predicates run against a context scrubbed of
//! Field::Secret (by schema type, recursively) — BUT legal non-secret nested
//! values (object/list-item/mode-variant) remain addressable so a legitimate
//! root guard does NOT fail open.
use nebula_schema::{Field, FieldValues, Schema, field_key};
use serde_json::json;

#[test]
fn legal_non_secret_nested_root_predicate_still_fires_after_scrub() {
    // root rule: if /items has an item with region == "eu", require `dpa`.
    // Must still enforce after the scrub (no fail-open).
    // (Build the schema with one Field::List of objects {region}, a sibling
    //  String `dpa`, and a root rule encoding the predicate. VERIFY the exact
    //  root-rule builder API against crates/schema/src/schema.rs / root_rules;
    //  the asserted behaviour is fixed: a submission with region=="eu" and no
    //  dpa MUST be Err; with dpa present MUST be Ok.)
    todo!("construct schema+root rule per real builder API — see VERIFY note");
}

#[test]
fn root_predicate_cannot_read_scrubbed_secret_plaintext() {
    // A Field::Secret `api_key` + a root rule whose predicate compares
    // /api_key by value. The scrub removes the secret from the context, so
    // the value-predicate evaluates as absent (fails closed), never reads
    // plaintext. (This complements the build-time lint, which still rejects
    // the schema; assert the runtime context, not just the lint.)
    todo!("construct per real API — asserted behaviour fixed: no plaintext reachable");
}
```

> VERIFY before coding: the root-rule builder API (`Schema::builder().root_rule(...)` / `with_root_rules` / `RootRule`) — read `crates/schema/src/schema.rs` + how `lint_and_loader.rs:1070` constructs root rules. These two tests' **asserted behaviour is fixed** (legal predicate fires; secret unreadable); only the construction API is to be confirmed. Replace `todo!()` with the concrete schema/values before Step 3.

- [ ] **Step 2: Run — expect FAIL** (`legal_non_secret_nested_root_predicate_still_fires_after_scrub` will currently PASS with `from_json`; it must continue to PASS after the scrub — its job is to catch the F4 fail-open. `root_predicate_cannot_read_scrubbed_secret_plaintext` will FAIL today because `from_json` exposes the secret).

```
cargo test -p nebula-schema --test seam_root_rule_scrub
```

- [ ] **Step 3: Extract the shared addressable-path walker.** In `lint.rs`, refactor `collect_secret_pointer_segments` (1101-1150) so its `walk_field`/`walk_scope` traversal is a reusable internal walker that yields, for every addressable leaf, `(segments: Vec<String>, kind: {Secret, NonSecretLeaf})` — descending `Field::Object`, `Field::List` whose item is `Field::Object` (children under the list path), `Field::Mode` variant payloads (under `segs+variant.key`), exactly as today. `collect_secret_pointer_segments` becomes "filter the walker to `Secret`". This is the **single owner** of the addressable-path invariant.

> Keep `lint_and_loader.rs:1070` + `:1104` green — the secret-key set must be byte-identical. This is a pure refactor of the lint; run those tests after.

- [ ] **Step 4: Add `root_predicate_context_for`.** In `context.rs`, add:

```rust
/// Build the predicate context for ROOT rules: every non-secret addressable
/// leaf (objects, list-item objects, mode-variant payloads — the same
/// traversal as the secret lint), excluding Field::Secret by schema type and
/// any container node that has a secret descendant. Closes the root-rule
/// secret-plaintext exposure (ADR-0052 P2) without making legal non-secret
/// nested root predicates fail open.
#[must_use]
pub fn root_predicate_context_for(fields:&[Field], values:&FieldValues) -> PredicateContext { /* walk via the shared walker; for each NonSecretLeaf addressable path, look up the value in `values` by that path and push (FieldPath, value); never emit a container node whose subtree contains a Secret */ }
```

> Design note: the shared walker yields the **addressable schema paths**; for each non-secret leaf, resolve the corresponding value from `values` (use the existing path lookup, mirroring `collect_non_secret`'s pairing but with List/Mode descent). VERIFY the value-lookup-by-path helper on `FieldValues` (`get_path` exists per validated.rs:438) — reuse it; do not hand-roll pointer parsing.

- [ ] **Step 5: Wire `run_root_rules` to the scrubbed context.** In `validated.rs:1908-1929`, replace `let json = values.to_json(); let pred_ctx = PredicateContext::from_json(&json);` with `let pred_ctx = crate::context::root_predicate_context_for(&self_fields, values);` — note `run_root_rules` currently takes `rules,&values,report`; it needs `&[Field]` too. Change its signature to `run_root_rules(fields:&[Field], rules:&[Rule], values:&FieldValues, report)` and update the call at `validate` (364) to pass `&self.0.fields`. The `&json` value passed to `validate_rules_with_ctx` stays `values.to_json()` (the value tree under test is unchanged; only the predicate **context** is scrubbed).

- [ ] **Step 6: Verify.**
```
cargo test -p nebula-schema --test seam_root_rule_scrub
cargo test -p nebula-schema --test lint_and_loader root_value_predicate_on_list_indexed_secret_is_rejected
cargo test -p nebula-schema --test lint_and_loader root_value_predicate_on_mode_secret_under_list_is_rejected
cargo test -p nebula-schema
```
Expected: all PASS. `legal_non_secret_…` PASS (no fail-open); `root_predicate_cannot_read_…` now PASS (scrub holds); lint anchors unchanged.

- [ ] **Step 7: Adversarial review** (re-engage the security panelist, verify-first): construct a root predicate on a list-indexed non-secret (`/items/0/region`) and a mode-variant non-secret payload — confirm each still resolves post-scrub (no fail-open). Construct `Predicate::Contains("/cfg", "<plaintext-substr>")` where `cfg` is a `Field::Object` containing a `Field::Secret` — confirm the container node is NOT in the scrubbed context. Fix-loop until clean.

- [ ] **Step 8: Per-crate fmt + commit.**
```bash
cargo fmt -p nebula-schema
git add crates/schema/src/lint.rs crates/schema/src/context.rs crates/schema/src/validated.rs crates/schema/tests/seam_root_rule_scrub.rs
bash scripts/worktree.sh commit fix schema "scrub Field::Secret from root-rule predicate context (ADR-0052 P2)"
```

---

### Task 4: Single validator crossing; delete legacy execution + error mapping; relocate lint helpers; vocabulary change (backlog b / item 3 / F2/F3)

**Files:**
- Create: `crates/schema/src/rule_ref.rs`
- Delete: `crates/schema/src/validator_bridge.rs`
- Modify: `crates/schema/src/lib.rs` (`mod validator_bridge;` → `mod rule_ref;`)
- Modify: `crates/schema/src/lint.rs` (import path `validator_bridge::` → `rule_ref::`)
- Modify: `crates/schema/src/validated.rs` (delete `run_rules`/`run_root_rules`/`translate_validator_code`/`push_validator_rule_errors`/`schema_path_from_validator_error` use; inline the 11 value-rule calls + the root-rule call through `validate_rules_with_ctx`; merge validator errors verbatim)
- Test: rewrite `crates/schema/tests/flow/all_error_codes.rs`; new `crates/schema/tests/seam_single_crossing.rs`; new `crates/schema/tests/seam_security_codes.rs`

- [ ] **Step 1: Relocate the pure path helpers.** Create `crates/schema/src/rule_ref.rs` containing **verbatim** `resolve_rule_dependency`, `referenced_root_key`, `normalize_rule_target_path`, `validator_path_to_schema_path`, `field_path_from_json_pointer`, `decode_json_pointer_segment`, and the file's `#[cfg(test)] mod tests` (move the tests too — do not drop them). Drop ONLY `schema_path_from_validator_error`. Module doc: "Pure schema-path/rule-reference parsing for the dependency-graph and secret lints (no validator coupling)."

- [ ] **Step 2: Rewire imports.** `lib.rs:166` `pub(crate) mod validator_bridge;` → `pub(crate) mod rule_ref;`. `lint.rs:12` `validator_bridge::{...}` → `rule_ref::{...}`. Delete `crates/schema/src/validator_bridge.rs`. Delete `use crate::validator_bridge::schema_path_from_validator_error;` (validated.rs:29).

- [ ] **Step 3: Run the lint anchors — expect PASS (behaviour byte-identical).**
```
cargo test -p nebula-schema --test lint_and_loader
```

- [ ] **Step 4: Write the single-crossing assertion test.** New `crates/schema/tests/seam_single_crossing.rs` — a source-level assertion (read `crates/schema/src/validated.rs` as text; assert it references no `nebula_validator` evaluation entry point other than `validate_rules_with_ctx` and `resolve_field_policies`; explicitly assert the strings `validate_rules(` (bare), `run_rules`, `run_root_rules`, `translate_validator_code`, `validator_bridge`, `PredicateContext::from_json` do NOT appear):

```rust
//! ADR-0052 P2 lockdown #1: the ONLY schema→validator behavioral crossing
//! symbols are `validate_rules_with_ctx` and `resolve_field_policies`.
#[test]
fn schema_crosses_into_validator_through_one_surface_only() {
    let src = include_str!("../src/validated.rs");
    for forbidden in ["fn run_rules", "fn run_root_rules", "fn translate_validator_code",
                      "fn push_validator_rule_errors", "validator_bridge",
                      "PredicateContext::from_json"] {
        assert!(!src.contains(forbidden), "validated.rs must not contain `{forbidden}` after P2");
    }
    assert!(src.contains("validate_rules_with_ctx"));
    assert!(src.contains("resolve_field_policies"));
}
```

- [ ] **Step 5: Collapse the crossings + drop translation.** In `validated.rs`:
  - Delete `translate_validator_code` (1861-1892), `push_validator_rule_errors` (1931-1948), `run_rules` (1895-1905), `run_root_rules` (1908-1929).
  - Add one private helper `fn merge_validator_errors(errs:&nebula_validator::foundation::ValidationErrors, fallback:&FieldPath, report:&mut ValidationReport)` that, **without `translate_validator_code`**, maps each validator error into the schema report verbatim: `code = e.code` (no remap), path = the validator error's RFC-6901 field pointer parsed via `rule_ref::` helpers (reuse the parsing that `schema_path_from_validator_error` used — move that small body inline here using `rule_ref::` parsing, fallback to `fallback`). Message unchanged.
  - Replace each of the 11 `run_rules(rules, &v, path, report)` sites with: `if let Err(errs) = nebula_validator::validate_rules_with_ctx(&v, rules, None, nebula_validator::ExecutionMode::StaticOnly) { merge_validator_errors(&errs, path, report); }`.
  - Replace the (Task-3 interim) root-rule call: `if let Err(errs) = nebula_validator::validate_rules_with_ctx(&values.to_json(), rules, Some(&crate::context::root_predicate_context_for(&self.0.fields, values)), nebula_validator::ExecutionMode::StaticOnly) { merge_validator_errors(&errs, &FieldPath::root(), report); }` inlined into `validate` (replacing the old `run_root_rules(...)` call at 364).
  - `resolution.required_failures` (validator `ValidationErrors`) now also merge via `merge_validator_errors(&resolution.required_failures, &FieldPath::root(), report)` (replacing the old `push_validator_rule_errors` call at 1135).

> The 11 `run_rules` sites currently differ only in `(rules, transformed_value, path)`. Introduce a tiny local closure `let mut run = |rules:&[Rule], v:&serde_json::Value, p:&FieldPath| { if let Err(errs)=nebula_validator::validate_rules_with_ctx(v, rules, None, nebula_validator::ExecutionMode::StaticOnly){ merge_validator_errors(&errs,p,report); } };` if borrow-checker permits (report is `&mut`), else a free fn `run_value_rules(rules, v, p, report)`. DRY — do not paste the call 11×.

- [ ] **Step 6: Rewrite `flow/all_error_codes.rs` to validator vocabulary** (the §0.1 "updated seam test in the same PR"). Read it fully first. Change every `length.min`→`min_length`, `length.max`→`max_length`, `range.min`→`min`, `range.max`→`max`, `pattern`/`email`/`url`→`invalid_format` (and adjust any param-based disambiguation assertions). Rewrite the module doc (it currently says "Validator codes are translated in `run_rules` via `translate_validator_code`"). Delete/invert the negative `type_mismatch` remap assertion (validated.rs:428-432 region of that test) — `type_mismatch` is now schema-emitted and unchanged.

> VERIFY before editing: read `crates/schema/tests/flow/all_error_codes.rs` in full; map each asserted code to its validator-native counterpart from `translate_validator_code`'s table (now deleted) — that table IS the migration spec. `email`/`url`/`pattern` all collapse to `invalid_format` (disambiguated by params); update assertions to match validator output (confirm via a scratch run).

- [ ] **Step 7: Pin security-relevant codes unchanged.** New `crates/schema/tests/seam_security_codes.rs`: assert a schema that triggers `required`, `expression.forbidden`, `expression.required` still reports exactly those code strings through the single-crossing path (these are schema/policy-emitted, not remapped — must be invariant across P2). [F4 V1]

- [ ] **Step 8: Verify.**
```
cargo test -p nebula-schema --test seam_single_crossing --test seam_security_codes
cargo test -p nebula-schema --test flow
cargo test -p nebula-schema
cargo test -p nebula-schema --test seam_proof_token_custody
cargo check --workspace --all-targets
```
Expected: all PASS; `seam_proof_token_custody` unchanged green; workspace compiles (no other crate consumed the deleted symbols — confirm).

> VERIFY: `rg "translate_validator_code|validator_bridge|push_validator_rule_errors" --type rust` across the workspace returns nothing outside this PR's deletions; if another crate referenced them, STOP and reassess scope.

- [ ] **Step 9: Two-stage review + adversarial security review** (verify-first: did dropping translation silently change a security-relevant code? did `merge_validator_errors` lose the field pointer / fallback correctly?). Fix-loop.

- [ ] **Step 10: Docs + per-crate fmt + commit.** Update `crates/schema/README.md` Contract section + the `STANDARD_CODES` rustdoc to state rule codes are now validator-native (link the ADR amendment).
```bash
cargo fmt -p nebula-schema
git add crates/schema/src/rule_ref.rs crates/schema/src/lib.rs crates/schema/src/lint.rs crates/schema/src/validated.rs crates/schema/README.md crates/schema/tests/flow/all_error_codes.rs crates/schema/tests/seam_single_crossing.rs crates/schema/tests/seam_security_codes.rs
git rm crates/schema/src/validator_bridge.rs
bash scripts/worktree.sh commit refactor schema "single validator crossing; drop code translation (ADR-0052 P2)"
```

---

### Task 5: `mode.no_payload_variant_must_forbid_expression` lint (backlog d / F4 D1–D2)

**Files:**
- Modify: `crates/schema/src/lint.rs`
- Test: `crates/schema/tests/lint_and_loader.rs`, `crates/schema/tests/seam_required_emitter.rs` (extend) or new `crates/schema/tests/seam_no_payload_variant.rs`

- [ ] **Step 1: Verify-first whether a runtime escape exists.** Construct (scratch test, not committed) a `Field::Mode` whose variant payload field is keyed `ModeField::EMPTY_PLACEHOLDER_KEY` but has `ExpressionMode::Allowed` (NOT via `variant_empty`, which forces `no_expression`; build the `ModeVariant` to bypass), submit `{"mode":<k>,"value":{"$expr":"…"}}`, call `.validate()`. Record: does it reach `resolve` (escape) or get rejected? This determines whether the lint alone (build-fatal) suffices or a runtime fail-closed is also required (security finding — never dismiss; if it escapes past a non-erroring build, it is in scope).

- [ ] **Step 2: Write the failing lint test** in `lint_and_loader.rs`:

```rust
#[test]
fn no_payload_mode_variant_without_forbidden_expression_is_rejected() {
    use nebula_schema::field::ModeField;
    // A no-payload variant placeholder (keyed EMPTY_PLACEHOLDER_KEY) whose
    // ExpressionMode is not Forbidden must fail to build.
    let bad_variant = nebula_schema::field::ModeVariant {
        key: "flag".into(),
        label: "Flag".into(),
        field: Box::new(
            Field::string(field_key!("_nebula_mode_empty"))
                .visible(VisibilityMode::Never), // ExpressionMode defaults to Allowed
        ),
    };
    let mode = /* ModeField with `bad_variant` pushed — VERIFY constructor */;
    let err = Schema::builder().add(Field::Mode(mode)).build().expect_err("must reject");
    assert!(err.errors().any(|e| e.code == "mode.no_payload_variant_must_forbid_expression"),
        "got: {:?}", err.errors().map(|e| e.code.to_string()).collect::<Vec<_>>());
}

#[test]
fn variant_empty_builds_clean() {
    // The canonical no-payload constructor pins Forbidden → no finding.
    let schema = Schema::builder()
        .add(Field::mode(field_key!("auth")).variant_empty("none", "None"))
        .build();
    assert!(schema.is_ok(), "variant_empty must satisfy the lint");
}
```

> VERIFY: `ModeVariant`/`ModeField` public construction path (`#[non_exhaustive]` — `ModeVariant` may not be struct-literal-constructible outside the crate; if so, build via `ModeField::variant(key,label,field)` with a no-payload-keyed placeholder field instead). `field_key!("_nebula_mode_empty")` must equal `ModeField::EMPTY_PLACEHOLDER_KEY`. The asserted behaviour (reject when not Forbidden; accept `variant_empty`) is fixed.

- [ ] **Step 3: Run — expect FAIL** (no such lint yet).
```
cargo test -p nebula-schema --test lint_and_loader no_payload_mode_variant_without_forbidden_expression_is_rejected
```

- [ ] **Step 4: Implement the lint.** In `lint.rs`, add:

```rust
/// A no-payload Mode variant's placeholder field (keyed `EMPTY_PLACEHOLDER_KEY`)
/// must pin `ExpressionMode::Forbidden`; otherwise a `{"$expr":…}` smuggled
/// into the hidden placeholder parses and is evaluated at resolve.
fn lint_mode_no_payload_variant_must_forbid_expression(
    fields:&[Field], prefix:&FieldPath, report:&mut ValidationReport,
) {
    for field in fields {
        if let Field::Mode(mode) = field {
            for v in &mode.variants {
                if v.field.key().as_str() == crate::field::ModeField::EMPTY_PLACEHOLDER_KEY
                    && *v.field.expression() != crate::mode::ExpressionMode::Forbidden
                {
                    report.push(
                        ValidationError::builder("mode.no_payload_variant_must_forbid_expression")
                            .at(prefix.clone().join(field.key().clone()))
                            .message(format!(
                                "no-payload mode variant `{}` placeholder must forbid expressions",
                                v.key
                            ))
                            .build(),
                    );
                }
            }
        }
    }
}
```
Register it in `lint_tree` (29) alongside the other per-scope lints (mirror how `lint_secret_predicate_on_value`/`secret.default_forbidden` are invoked — Severity defaults to Error via `ValidationError::builder` per the existing `secret.*` lints; confirm `builder(code)` ⇒ `Severity::Error` like lint.rs:173/1218). Recurse nested scopes the same way `lint_tree` already recurses Object/List/Mode (match the existing recursion).

> VERIFY: how `lint_tree` enumerates scopes + recurses (read lint.rs:29-160) — register the new lint exactly where `secret`/`mode` lints already run so nested Mode fields are covered (mirror `collect_secret_pointer_segments`'s descent expectations).

- [ ] **Step 5: Add the runtime defence-in-depth regression** (F4 D2) in `seam_no_payload_variant.rs`: a no-payload variant built via `variant_empty` (Forbidden) + submitted `{"value":{"$expr":"…"}}` ⇒ `.validate()` errs with `expression.forbidden` (proves the runtime path also fails closed for the realistic smuggle, independent of the lint).

- [ ] **Step 6: Verify.**
```
cargo test -p nebula-schema --test lint_and_loader
cargo test -p nebula-schema --test seam_no_payload_variant
cargo test -p nebula-schema
```
Expected: PASS. Document the Step-1 verify-first finding in the ADR amendment (lint is build-fatal ⇒ fail-closed via the `build()`-only proof-token typestate; runtime test is additive proof). If Step 1 found a genuine escape past a non-erroring build, add the runtime fail-closed fix here and a regression for it (in scope — security).

- [ ] **Step 7: Two-stage review + commit.**
```bash
cargo fmt -p nebula-schema
git add crates/schema/src/lint.rs crates/schema/tests/lint_and_loader.rs crates/schema/tests/seam_no_payload_variant.rs
bash scripts/worktree.sh commit feat schema "lint no-payload mode variant must forbid expression (ADR-0052 P2)"
```

---

### Task 6: Full verification gate + PR + triage + merge

**Files:** none (gate + PR).

- [ ] **Step 1: Full gate (per-crate fmt — never `cargo fmt --all` on the worktree path).**
```
cargo fmt -p nebula-schema -- --check
cargo fmt -p nebula-validator -- --check
cargo clippy --workspace --all-targets -q -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
cargo deny check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```
All must pass. If `cargo fmt -p` per-crate `--check` flags drift, run `cargo fmt -p <crate>` (no `--all`). Fix any failure; re-run.

- [ ] **Step 2: Confirm cross-cutting merge-blockers** (the 8-item list in Authority). Each test named and green; ADR amendment present; `validator_bridge.rs` gone; lint helpers relocated; `seam_proof_token_custody` unchanged.

- [ ] **Step 3: Push + PR.**
```bash
git -C C:/Users/vanya/RustroverProjects/nebula/.worktrees/adr0052-p2 push -u origin refactor/schema-adr0052-p2
```
`gh pr create` against `vanyastaff/nebula` base `main`, following `.github/PULL_REQUEST_TEMPLATE.md`. Title: `refactor(schema)!: ADR-0052 P2 — validator sole emitter, root-rule scrub, single crossing (#…)`. Body: tick "L2 invariant changed → ADR + seam test in this PR"; list the public rule-code vocabulary change explicitly; list the flagged out-of-scope items (`0042`/`0052` collisions, slot_bindings, derive_schema/json-schema/regex) as known-not-done with rationale; end with the Claude Code trailer.

- [ ] **Step 4: Triage bot reviews (CodeRabbit/Copilot/Codex) verify-first.** For each comment: verify the claim against code before agreeing; reply per `comment_id`; resolve threads only when addressed. Hold the P1 bar — a real fail-open survived a first fix in P1; do not dismiss security comments.

- [ ] **Step 5: Squash-merge only when CI is fully green and confirmed** (do not merge blind). Then post-merge cleanup:
```bash
bash scripts/worktree.sh finish adr0052-p2
```

- [ ] **Step 6: Spawn P3** (HasSchema convergence) the same way this task was spawned — a self-contained follow-up against P2's landed signatures. Out of scope to implement here.

---

## Self-Review

**1. Spec coverage (P2 backlog (a)–(e) + item-2 + scoped translate deletion):**
- (a) `FieldPlan` opaque payload → Task 1 (+ trybuild). ✓
- (b) single schema→validator crossing + test; delete `run_root_rules`/`validator_bridge.rs` (relocate lint helpers, delete error-mapping) → Task 4 (`seam_single_crossing`). ✓
- (c) sole-emitter fold (move, not delete; exactly-one; ternary + two bits) → Task 2 (`validate_schema.rs:726` + `seam_required_emitter`). ✓
- (d) `mode.no_payload_variant_must_forbid_expression` lint (+ runtime defence) → Task 5. ✓
- item 2 root-rule scrub (shared traversal, no fail-open) → Task 3 (`seam_root_rule_scrub`). ✓
- scoped `translate_validator_code` deletion + vocabulary change (ADR amendment, `all_error_codes.rs` rewrite, README) → Task 0 + Task 4. ✓
- proof-token custody unchanged → Task 4 Step 8 (`seam_proof_token_custody` green). ✓

**2. Placeholder scan:** `todo!()` appears only in Task 3 Step 1 / Task 5 Step 2 test scaffolds, each paired with an explicit "VERIFY … the asserted behaviour is fixed; confirm only the construction API" note — these are name-confirmations of real builder APIs, not design gaps (the P1-bar convention, blessed by the P1 plan self-review). No "TBD"/"add error handling"/"similar to Task N".

**3. Type consistency:** `FieldPolicyDecl::new(path, visibility, required, value_present, raw_present, payload)` and `FieldPlan { path, presence, requiredness, directive, payload }` and `FieldDirective { Skip, RequiredAbsent, Validate }` and `FieldPolicyResolution { plans, required_failures }` (no `Default`) used identically across Tasks 1/2/4. `root_predicate_context_for(fields, values)` identical Tasks 3/4. `merge_validator_errors(errs, fallback, report)` Task 4. `rule_ref::{resolve_rule_dependency, referenced_root_key, normalize_rule_target_path}` Tasks 4/lint.

**4. Ordering/lefthook:** every Task commits at a workspace-green point (each ends with the full per-crate fmt + the lefthook-mirrored `clippy --workspace -D warnings`). Task 1 atomically updates validator+schema (cross-crate signature). Task 3's interim `run_root_rules` wiring is deleted in Task 4 **preserving** `root_predicate_context_for` (TDD: close the security hole with a test first, then refactor). Tasks 1→2→4 are dependency-ordered; Task 3 is independent of 1/2; Task 5 is independent.
