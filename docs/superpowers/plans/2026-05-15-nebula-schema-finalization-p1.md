# nebula-schema Finalization — P1 (Q3 core seam) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move field visibility/required condition evaluation into `nebula-validator` behind a typed, no-`bool` policy API; delete the fail-open `Rule::evaluate`/`RuleContext` legacy path; scrub secret-typed fields out of the predicate context by schema type; land ADR-0052 + a proof-token-custody seam test in this same PR.

**Architecture:** `nebula-validator` gains a `policy` module (`Presence`/`Requiredness` `#[non_exhaustive]` enums, `VisibilityPolicy`/`RequiredPolicy` with a single `resolve(&PredicateContext)`, `Rule::matches(&PredicateContext)`, and `resolve_field_policies` returning `FieldPlan`s + validator-owned `required_failures`). `nebula-schema::ValidSchema::validate` builds one `PredicateContext` that **excludes `Field::Secret` fields by schema type** (pre-resolve they are `FieldValue::Literal(plaintext)`, so the old runtime-tag scrub did not catch them), calls `resolve_field_policies` once, and consumes the result through a single `match plan.presence { Skipped => continue, Active => … }` — the field-rule path is unreachable from `Skipped` by data flow, not call order. The legacy `Rule::evaluate(&dyn RuleContext)` (silent-`false` for nested JSON-Pointer paths → fail-open `required`) and the `RuleContext` trait are deleted, no shim.

**Tech Stack:** Rust 1.95 / edition 2024, `cargo nextest`, `trybuild` (compile-fail), `serde_json`, `tracing`. Crates touched: `nebula-validator` (Core), `nebula-schema` (Core). No new dependencies. No `deny.toml` change (schema→validator is an existing legal Core→Core data edge).

**Scope note:** This is P1 of a 4-phase cascade (spec: `docs/superpowers/specs/2026-05-15-nebula-schema-finalization-design.md`). P2 (delete `run_rules`/`run_root_rules`/`validator_bridge.rs`/`translate_validator_code`, `derive_schema.rs:95/126`, json-schema secret-default, regex bounding), P3 (HasSchema convergence), P4 (API) get their own plans authored against P1's landed signatures. Do **not** attempt P2–P4 work here. `run_root_rules` stays as-is in P1.

**Per-commit gate (read before every commit).** `lefthook.yml` `pre-commit`
runs, on EVERY commit touching `*.rs`, the full `cargo fmt --check`,
`cargo clippy --workspace --all-targets -q -- -D warnings`, `taplo`, and
`cargo-deny`; `commit-msg` runs `convco`. Therefore **every task's commit
must leave the whole workspace fmt-clean and clippy-clean with zero
warnings** (unused imports/dead code are hard errors). Consequences for
implementers: (a) never add an import, type, or fn before the task that
consumes it — add each `use` line in the task that first uses it; (b) never
silence with `#[allow(...)]` or `git commit --no-verify` (forbidden by
AGENTS.md / lefthook-mirrors-CI); (c) if a task's code only compiles cleanly
once a later task lands, the two tasks form one commit unit — defer the
commit to the later task and say so explicitly (Task 6→7 already does this).

---

## File Structure

| File | Responsibility | Action |
|---|---|---|
| `crates/validator/src/policy/mod.rs` | New: `Presence`, `Requiredness`, `VisibilityPolicy`, `RequiredPolicy`, `FieldPolicyDecl`, `FieldPlan`, `FieldPolicyResolution`, `resolve_field_policies` | Create |
| `crates/validator/src/rule/mod.rs` | Add `Rule::matches(&PredicateContext) -> bool`; delete `RuleContext` trait + `impl RuleContext for HashMap` + `Rule::evaluate` + `evaluate_predicate_via_rule_context` | Modify |
| `crates/validator/src/rule/context.rs` | Replace derived `Debug` on `PredicateContext` with a redacting hand impl | Modify `:12` |
| `crates/validator/src/lib.rs` | Register `pub mod policy;`; re-export policy types; drop `RuleContext` from the `rule::{…}` re-export | Modify `:57-86` |
| `crates/schema/src/context.rs` | Replace `RootContext`/`ObjectContext` (`impl RuleContext`) with a schema-type-aware `PredicateContext` builder that excludes `Field::Secret` | Rewrite |
| `crates/schema/src/validated.rs` | `ValidSchema::validate` builds scrubbed `PredicateContext`, calls `resolve_field_policies`, drains `required_failures`, iterates `FieldPlan` via `match plan.presence`; `validate_field` loses the visibility/required block and the `&dyn RuleContext` param | Modify `:333-365`, `:1038-1110`, `:1117-1122` |
| `crates/schema/src/lint.rs` | New `secret.predicate_on_value` lint: reject value-comparing predicate whose `FieldPath` targets a `Field::Secret` | Modify (add fn + call site near `:51`/`:605`) |
| `crates/schema/tests/seam_proof_token_custody.rs` | New: ADR-0052 seam test — `ValidValues`/`ResolvedValues` mintable only via pipeline | Create |
| `crates/schema/tests/compile_fail/policy_presence_non_exhaustive.rs` | New trybuild: `match Presence` missing `Skipped` fails | Create |
| `crates/schema/tests/compile_fail/rule_context_removed.rs` | New trybuild: `nebula_validator::RuleContext` no longer resolves | Create |
| `crates/schema/tests/compile_fail.rs` | Register the two new trybuild cases | Modify |
| `docs/adr/0052-schema-validator-condition-seam.md` | ADR-0052 (P1 merge-blocker, canon §0.1/§17) | Create |

---

## Task 0: ADR-0052 (P1 merge-blocker — author first)

Per canon §0.1/§17 and panel lockdown #3, the ADR + seam test ship in **this** PR. Write the ADR before code so the decision is fixed.

**Files:**
- Create: `docs/adr/0052-schema-validator-condition-seam.md`

- [ ] **Step 1: Write the ADR**

Create `docs/adr/0052-schema-validator-condition-seam.md` with exactly:

```markdown
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
```

- [ ] **Step 2: Update the ADR index**

In `docs/adr/README.md`, add this row to the `## Index` table immediately after the `0050` row (keep the table's column order `# | Title | Status | Tags`):

```markdown
| [0052](./0052-schema-validator-condition-seam.md) | Field visibility/required condition evaluation moves to nebula-validator | accepted (2026-05-15) | schema, validator, seam, m11 |
```

- [ ] **Step 3: Commit**

```bash
git -C "$WT" add docs/adr/0052-schema-validator-condition-seam.md docs/adr/README.md
git -C "$WT" -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "docs(adr): ADR-0052 schema/validator condition-evaluation seam"
```

Expected: lefthook `typos` + `convco` pass; one commit created.

---

## Task 1: `policy` module — `Presence` / `Requiredness`

**Files:**
- Create: `crates/validator/src/policy/mod.rs`
- Modify: `crates/validator/src/lib.rs:57-73` (register module)
- Test: in `crates/validator/src/policy/mod.rs` (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Create the module file with the two enums + a failing test**

Create `crates/validator/src/policy/mod.rs`:

```rust
//! Field visibility / required policy evaluation (ADR-0052).
//!
//! Owns the *engine* for `When(Rule)` conditions. Callers get typed
//! `Presence`/`Requiredness` verdicts — never a raw `bool` they could
//! forget to branch on.
//!
//! Imports are added by later tasks as each type is first consumed
//! (Task 3 adds `crate::rule::{PredicateContext, Rule}`; Task 4 adds
//! `crate::foundation::{FieldPath, ValidationError, ValidationErrors}`).
//! Do NOT add a `use` before the task that uses it — the per-commit
//! clippy `-D warnings` gate rejects unused imports.

/// Whether a field participates in this validation round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Presence {
    /// Field is visible; its value rules must run.
    Active,
    /// Field is hidden; its value rules MUST be skipped.
    Skipped,
}

/// Resolved required-ness for a field in this round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Requiredness {
    /// Absence is an error.
    Required,
    /// Absence is allowed.
    Optional,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_variants_are_copy_and_eq() {
        let p = Presence::Active;
        let q = p; // Copy
        assert_eq!(p, q);
        assert_ne!(Presence::Active, Presence::Skipped);
    }

    #[test]
    fn requiredness_variants_are_copy_and_eq() {
        let r = Requiredness::Required;
        let s = r;
        assert_eq!(r, s);
        assert_ne!(Requiredness::Required, Requiredness::Optional);
    }
}
```

- [ ] **Step 2: Register the module so it compiles**

In `crates/validator/src/lib.rs`, in the `// ── Public modules ──` block (currently lines 58-73, alphabetical), add after the `pub mod foundation;` line and before `pub mod prelude;`:

```rust
/// Field visibility / required policy evaluation (ADR-0052).
pub mod policy;
```

- [ ] **Step 3: Run the test to verify it passes (enums compile)**

Run: `cargo nextest run -p nebula-validator policy::tests`
Expected: PASS (2 tests). The file has **no `use` block** (the two enums are self-contained; tests use `super::*`). The crate must compile clippy-clean — the lefthook pre-commit gate runs `cargo clippy --workspace --all-targets -- -D warnings` on this commit (see "Per-commit gate" near the top). There must be zero unused-import warnings because there are no imports yet.

- [ ] **Step 4: Commit**

```bash
git -C "$WT" add crates/validator/src/policy/mod.rs crates/validator/src/lib.rs
git -C "$WT" -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(validator): add policy module with Presence/Requiredness enums"
```

---

## Task 2: `Rule::matches(&PredicateContext) -> bool`

This is the nested-correct replacement for `Rule::evaluate`. It uses the
already-correct `Predicate::evaluate(&PredicateContext)` (`crates/validator/src/rule/predicate.rs:69`).

**Files:**
- Modify: `crates/validator/src/rule/mod.rs` (add method in the `impl Rule` block that ends at `:161`)
- Test: `crates/validator/src/rule/tests.rs` (existing `#[cfg(test)] mod tests;` declared at `mod.rs:27-28`)

- [ ] **Step 1: Write the failing test**

Append to `crates/validator/src/rule/tests.rs`:

```rust
#[test]
fn matches_resolves_nested_pointer_paths() {
    use crate::rule::{context::PredicateContext, predicate::Predicate};
    use crate::foundation::FieldPath;

    // The exact case the deleted `Rule::evaluate` failed: a predicate on a
    // NESTED path. Old flat-key lookup silently returned false (fail-open).
    let rule = Rule::Predicate(Predicate::Eq(
        FieldPath::parse("/auth/mode").unwrap(),
        serde_json::json!("oauth"),
    ));
    let ctx = PredicateContext::from_json(&serde_json::json!({
        "auth": { "mode": "oauth" }
    }));
    assert!(rule.matches(&ctx), "nested predicate must evaluate true via PredicateContext");

    let ctx_no = PredicateContext::from_json(&serde_json::json!({
        "auth": { "mode": "apikey" }
    }));
    assert!(!rule.matches(&ctx_no));
}

#[test]
fn matches_value_and_deferred_are_true() {
    use crate::rule::{context::PredicateContext, value::ValueRule};
    let ctx = PredicateContext::new();
    assert!(Rule::Value(ValueRule::Email).matches(&ctx));
}

#[test]
fn matches_logic_all_any_not() {
    use crate::rule::{context::PredicateContext, predicate::Predicate, logic::Logic};
    use crate::foundation::FieldPath;
    let ctx = PredicateContext::from_json(&serde_json::json!({"a": 1, "b": 2}));
    let a = Rule::Predicate(Predicate::Eq(FieldPath::parse("a").unwrap(), serde_json::json!(1)));
    let b = Rule::Predicate(Predicate::Eq(FieldPath::parse("b").unwrap(), serde_json::json!(9)));
    assert!(Rule::Logic(Box::new(Logic::Any(vec![a.clone(), b.clone()]))).matches(&ctx));
    assert!(!Rule::Logic(Box::new(Logic::All(vec![a.clone(), b.clone()]))).matches(&ctx));
    assert!(Rule::Logic(Box::new(Logic::Not(b))).matches(&ctx));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p nebula-validator rule::tests::matches_resolves_nested_pointer_paths`
Expected: FAIL — `no method named `matches` found for enum `Rule``.

- [ ] **Step 3: Implement `Rule::matches`**

In `crates/validator/src/rule/mod.rs`, inside the `impl Rule { … }` block, immediately after the `field_references` method (which ends at line 161 with the closing `}` of the match) and before the block's closing `}` at line 162, insert:

```rust
    /// Boolean predicate evaluation against a structured context.
    ///
    /// Replaces the removed `Rule::evaluate(&dyn RuleContext)`, which did a
    /// flat-key lookup and silently returned `false` for nested JSON-Pointer
    /// paths. `Value`/`Deferred` are not predicates and evaluate to `true`.
    #[must_use]
    pub fn matches(&self, ctx: &PredicateContext) -> bool {
        match self {
            Self::Value(_) | Self::Deferred(_) => true,
            Self::Predicate(p) => p.evaluate(ctx),
            Self::Logic(l) => match l.as_ref() {
                Logic::All(rules) => rules.iter().all(|r| r.matches(ctx)),
                Logic::Any(rules) => rules.iter().any(|r| r.matches(ctx)),
                Logic::Not(inner) => !inner.matches(ctx),
            },
            Self::Described(inner, _) => inner.matches(ctx),
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p nebula-validator rule::tests::matches`
Expected: PASS (3 tests: `matches_resolves_nested_pointer_paths`, `matches_value_and_deferred_are_true`, `matches_logic_all_any_not`).

- [ ] **Step 5: Commit**

```bash
git -C "$WT" add crates/validator/src/rule/mod.rs crates/validator/src/rule/tests.rs
git -C "$WT" -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(validator): add Rule::matches over PredicateContext (nested-correct)"
```

---

## Task 3: `VisibilityPolicy` / `RequiredPolicy` + `resolve`

Borrowed (`&'a Rule`) so schema maps its serde enums with zero clones.

**Files:**
- Modify: `crates/validator/src/policy/mod.rs`
- Test: same file `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing test**

In `crates/validator/src/policy/mod.rs`, inside `mod tests`, add:

```rust
    #[test]
    fn visibility_policy_resolves_to_presence() {
        use crate::rule::{context::PredicateContext, predicate::Predicate};
        let ctx = PredicateContext::from_json(&serde_json::json!({"enabled": true}));
        assert_eq!(VisibilityPolicy::Always.resolve(&ctx), Presence::Active);
        assert_eq!(VisibilityPolicy::Never.resolve(&ctx), Presence::Skipped);
        let rule = Rule::Predicate(Predicate::IsTrue(
            crate::foundation::FieldPath::parse("enabled").unwrap(),
        ));
        assert_eq!(VisibilityPolicy::When(&rule).resolve(&ctx), Presence::Active);
    }

    #[test]
    fn required_policy_resolves_to_requiredness() {
        use crate::rule::{context::PredicateContext, predicate::Predicate};
        let ctx = PredicateContext::from_json(&serde_json::json!({"mode": "oauth"}));
        assert_eq!(RequiredPolicy::Optional.resolve(&ctx), Requiredness::Optional);
        assert_eq!(RequiredPolicy::Always.resolve(&ctx), Requiredness::Required);
        let rule = Rule::Predicate(Predicate::Eq(
            crate::foundation::FieldPath::parse("mode").unwrap(),
            serde_json::json!("oauth"),
        ));
        assert_eq!(RequiredPolicy::When(&rule).resolve(&ctx), Requiredness::Required);
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p nebula-validator policy::tests::visibility_policy_resolves_to_presence`
Expected: FAIL — `cannot find type `VisibilityPolicy` in this scope`.

- [ ] **Step 3: Implement the policy enums**

First, add the imports this task consumes — at the top of
`crates/validator/src/policy/mod.rs`, immediately after the module
doc-comment block and before the `Presence` enum, insert:

```rust
use crate::rule::{PredicateContext, Rule};
```

Then, in `crates/validator/src/policy/mod.rs`, after the `Requiredness` enum and before `#[cfg(test)]`, add:

```rust
/// A field's visibility policy, borrowed from the schema's serde enum.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum VisibilityPolicy<'a> {
    /// Always visible.
    Always,
    /// Never visible.
    Never,
    /// Visible only when the borrowed rule matches the context.
    When(&'a Rule),
}

/// A field's required policy, borrowed from the schema's serde enum.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum RequiredPolicy<'a> {
    /// Never required.
    Optional,
    /// Always required.
    Always,
    /// Required only when the borrowed rule matches the context.
    When(&'a Rule),
}

impl VisibilityPolicy<'_> {
    /// The only way to turn a visibility policy into a decision.
    #[must_use]
    pub fn resolve(&self, ctx: &PredicateContext) -> Presence {
        match self {
            Self::Always => Presence::Active,
            Self::Never => Presence::Skipped,
            Self::When(r) => {
                if r.matches(ctx) {
                    Presence::Active
                } else {
                    Presence::Skipped
                }
            },
        }
    }
}

impl RequiredPolicy<'_> {
    /// The only way to turn a required policy into a decision.
    #[must_use]
    pub fn resolve(&self, ctx: &PredicateContext) -> Requiredness {
        match self {
            Self::Optional => Requiredness::Optional,
            Self::Always => Requiredness::Required,
            Self::When(r) => {
                if r.matches(ctx) {
                    Requiredness::Required
                } else {
                    Requiredness::Optional
                }
            },
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p nebula-validator policy::tests`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git -C "$WT" add crates/validator/src/policy/mod.rs
git -C "$WT" -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(validator): VisibilityPolicy/RequiredPolicy resolve to typed verdicts"
```

---

## Task 4: `resolve_field_policies` + `FieldPlan` / `FieldPolicyResolution`

The single validator entry point (panel lockdown #1). It owns required
reporting: it emits a `required` `ValidationError` for each
`Requiredness::Required && !value_present` field.

**Files:**
- Modify: `crates/validator/src/policy/mod.rs`
- Test: same file `mod tests`

- [ ] **Step 1: Write the failing test**

In `crates/validator/src/policy/mod.rs` `mod tests`, add:

```rust
    #[test]
    fn resolve_field_policies_plans_and_required_failures() {
        use crate::rule::{context::PredicateContext, predicate::Predicate};
        let ctx = PredicateContext::from_json(&serde_json::json!({"mode": "oauth"}));

        let visible_path = FieldPath::parse("client_id").unwrap();
        let hidden_path = FieldPath::parse("legacy").unwrap();
        let req_rule = Rule::Predicate(Predicate::Eq(
            FieldPath::parse("mode").unwrap(),
            serde_json::json!("oauth"),
        ));

        let decls = vec![
            FieldPolicyDecl {
                path: &visible_path,
                visibility: VisibilityPolicy::Always,
                required: RequiredPolicy::When(&req_rule),
                value_present: false, // required (mode==oauth) but absent → failure
            },
            FieldPolicyDecl {
                path: &hidden_path,
                visibility: VisibilityPolicy::Never,
                required: RequiredPolicy::Always,
                value_present: false, // hidden → no required failure
            },
        ];

        let res = resolve_field_policies(decls, &ctx);

        assert_eq!(res.plans.len(), 2);
        let visible_plan = res.plans.iter().find(|p| p.path == &visible_path).unwrap();
        assert_eq!(visible_plan.presence, Presence::Active);
        assert_eq!(visible_plan.requiredness, Requiredness::Required);
        let hidden_plan = res.plans.iter().find(|p| p.path == &hidden_path).unwrap();
        assert_eq!(hidden_plan.presence, Presence::Skipped);

        // Exactly one required failure: the visible, required, absent field.
        // The hidden field is skipped → its `Always` required does not fire.
        let failures: Vec<_> = res.required_failures.iter().collect();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].code, "required");
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p nebula-validator policy::tests::resolve_field_policies_plans_and_required_failures`
Expected: FAIL — `cannot find type `FieldPolicyDecl``.

- [ ] **Step 3: Implement the decl/plan structs + entry point**

First, replace the module doc-comment block (the `//!` lines, including
the scaffolding line "Imports are added by later tasks (Task 3 adds …;
Task 4 adds …)") with this permanent, plan-independent description — no
plan/task references may remain in committed code (project rule):

```rust
//! Field visibility / required policy evaluation (ADR-0052).
//!
//! Owns the *engine* for `When(Rule)` conditions. Callers get typed
//! `Presence`/`Requiredness` verdicts — never a raw `bool` they could
//! forget to branch on. `resolve_field_policies` is the single entry
//! point `nebula-schema`'s `validate` uses.
```

Then, extend the imports with the types this task adds (Task 3 already
added `use crate::rule::{PredicateContext, Rule};`). Add a second `use`
line right below it:

```rust
use crate::foundation::{FieldPath, ValidationError, ValidationErrors};
```

Then, in `crates/validator/src/policy/mod.rs`, before `#[cfg(test)]`, add:

```rust
/// Per-field policy declaration the schema hands to the validator.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FieldPolicyDecl<'a> {
    /// RFC 6901 path of the field.
    pub path: &'a FieldPath,
    /// Visibility policy borrowed from the schema enum.
    pub visibility: VisibilityPolicy<'a>,
    /// Required policy borrowed from the schema enum.
    pub required: RequiredPolicy<'a>,
    /// Whether a non-absent raw value is present for this field.
    pub value_present: bool,
}

impl<'a> FieldPolicyDecl<'a> {
    /// Construct a decl. Explicit ctor keeps the `#[non_exhaustive]` struct
    /// constructible across the crate boundary.
    #[must_use]
    pub fn new(
        path: &'a FieldPath,
        visibility: VisibilityPolicy<'a>,
        required: RequiredPolicy<'a>,
        value_present: bool,
    ) -> Self {
        Self {
            path,
            visibility,
            required,
            value_present,
        }
    }
}

/// Per-field decision the schema MUST honor.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FieldPlan<'a> {
    /// RFC 6901 path of the field.
    pub path: &'a FieldPath,
    /// Whether the field participates this round.
    pub presence: Presence,
    /// Resolved required-ness (informational; required failures are already
    /// emitted into `FieldPolicyResolution::required_failures`).
    pub requiredness: Requiredness,
}

/// Output of [`resolve_field_policies`].
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct FieldPolicyResolution<'a> {
    /// One plan per input decl, in input order.
    pub plans: Vec<FieldPlan<'a>>,
    /// `required` errors for visible, required, absent fields — validator
    /// owns this reporting (ADR-0052).
    pub required_failures: ValidationErrors,
}

/// Resolve visibility/required for a set of fields against one context.
///
/// A `Presence::Skipped` field never produces a required failure even if its
/// `RequiredPolicy` is `Always` — a hidden field cannot be required.
#[must_use]
pub fn resolve_field_policies<'a, I>(decls: I, ctx: &PredicateContext) -> FieldPolicyResolution<'a>
where
    I: IntoIterator<Item = FieldPolicyDecl<'a>>,
{
    let mut out = FieldPolicyResolution::default();
    for d in decls {
        let presence = d.visibility.resolve(ctx);
        let requiredness = d.required.resolve(ctx);
        if presence == Presence::Active
            && requiredness == Requiredness::Required
            && !d.value_present
        {
            out.required_failures.push(
                ValidationError::new("required", "field is required")
                    .with_field_path(d.path.clone()),
            );
        }
        out.plans.push(FieldPlan {
            path: d.path,
            presence,
            requiredness,
        });
    }
    out
}
```

> Note for the implementer: confirm the exact constructor names by reading
> `crates/validator/src/foundation/error/validation_error.rs` — the codebase
> uses `ValidationError::new(code, msg)` and `.with_field_path(FieldPath)`
> (see `predicate_error` at `crates/validator/src/rule/mod.rs:314-316`) and
> `ValidationErrors::push` (see `crates/validator/src/foundation/error/validation_errors.rs`).
> If `ValidationErrors` exposes `add` rather than `push`, use that name; do
> not invent a method.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo nextest run -p nebula-validator policy::tests`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git -C "$WT" add crates/validator/src/policy/mod.rs
git -C "$WT" -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(validator): resolve_field_policies single entry point + FieldPlan"
```

---

## Task 5: Re-export the policy surface; redacting `PredicateContext` Debug (#5)

**Files:**
- Modify: `crates/validator/src/lib.rs:84-86`
- Modify: `crates/validator/src/rule/context.rs:12`
- Test: `crates/validator/src/rule/context.rs` `mod tests`

- [ ] **Step 1: Write the failing Debug-redaction test**

In `crates/validator/src/rule/context.rs` `#[cfg(test)] mod tests` (after `empty_context_is_empty`), add:

```rust
    #[test]
    fn debug_does_not_leak_field_values() {
        let ctx = PredicateContext::from_json(&json!({"api_key": "s3cr3t-value", "n": 1}));
        let dbg = format!("{ctx:?}");
        assert!(!dbg.contains("s3cr3t-value"), "Debug must not print field values: {dbg}");
        assert!(dbg.contains("PredicateContext"));
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p nebula-validator rule::context::tests::debug_does_not_leak_field_values`
Expected: FAIL — derived `Debug` prints the map, output contains `s3cr3t-value`.

- [ ] **Step 3: Replace derived Debug with a redacting hand impl**

In `crates/validator/src/rule/context.rs`, change line 12 from:

```rust
#[derive(Debug, Clone, Default)]
```

to:

```rust
#[derive(Clone, Default)]
```

Then immediately after the `struct PredicateContext { … }` block (after line 15), add:

```rust
impl std::fmt::Debug for PredicateContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Field values may be secret-shaped pre-resolve; never print them.
        f.debug_struct("PredicateContext")
            .field("field_count", &self.fields.len())
            .finish_non_exhaustive()
    }
}
```

- [ ] **Step 4: Run the context tests to verify they pass**

Run: `cargo nextest run -p nebula-validator rule::context::tests`
Expected: PASS (all existing context tests + `debug_does_not_leak_field_values`).

- [ ] **Step 5: Re-export the policy surface**

In `crates/validator/src/lib.rs`, in the `// ── Re-exports ──` block, add after the `pub use proof::Validated;` line:

```rust
pub use policy::{
    FieldPlan, FieldPolicyDecl, FieldPolicyResolution, Presence, RequiredPolicy, Requiredness,
    VisibilityPolicy, resolve_field_policies,
};
```

Leave the `pub use rule::{ … RuleContext … }` line unchanged for now — Task 8 removes `RuleContext` from it once schema no longer needs the trait (deleting it here would break the schema build mid-plan).

- [ ] **Step 6: Verify the crate builds and commit**

Run: `cargo check -p nebula-validator`
Expected: builds (warnings about unused `RuleContext` re-export are fine until Task 8).

```bash
git -C "$WT" add crates/validator/src/lib.rs crates/validator/src/rule/context.rs
git -C "$WT" -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(validator): re-export policy surface; redact PredicateContext Debug"
```

---

## Task 6: schema — scrubbed `PredicateContext` builder keyed on `Field::Secret` (#1)

Replaces `RootContext`/`ObjectContext`. The builder walks the schema and the
values together and **omits any field whose `Field` is `Field::Secret`** —
this catches the pre-resolve `FieldValue::Literal(plaintext)` case the old
runtime-tag scrub (`context.rs:22-27`) missed.

**Files:**
- Rewrite: `crates/schema/src/context.rs`
- Test: `crates/schema/src/context.rs` `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing test (pre-resolve plaintext secret must be absent)**

Replace the entire `#[cfg(test)] mod tests { … }` block at the bottom of `crates/schema/src/context.rs` with:

```rust
#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{Field, key::FieldKey, value::{FieldValue, FieldValues}};

    #[test]
    fn literal_field_is_visible_to_predicates() {
        let fields = vec![Field::string(FieldKey::new("name").unwrap())];
        let mut values = FieldValues::new();
        values.set(FieldKey::new("name").unwrap(), FieldValue::Literal(json!("alice")));
        let ctx = predicate_context_for(&fields, &values);
        assert_eq!(
            ctx.get(&nebula_validator::foundation::FieldPath::parse("name").unwrap()),
            Some(&json!("alice"))
        );
    }

    #[test]
    fn pre_resolve_plaintext_secret_is_scrubbed_by_schema_type() {
        // A Field::Secret holding a pre-resolve plaintext Literal MUST NOT
        // enter the predicate context. The old runtime-tag scrub failed this.
        let fields = vec![Field::secret(FieldKey::new("api_key").unwrap())];
        let mut values = FieldValues::new();
        values.set(
            FieldKey::new("api_key").unwrap(),
            FieldValue::Literal(json!("s3cr3t-plaintext")),
        );
        let ctx = predicate_context_for(&fields, &values);
        assert!(
            ctx.get(&nebula_validator::foundation::FieldPath::parse("api_key").unwrap()).is_none(),
            "secret-typed field must be excluded from the predicate context"
        );
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p nebula-schema context::tests::pre_resolve_plaintext_secret_is_scrubbed_by_schema_type`
Expected: FAIL — `cannot find function `predicate_context_for``.

- [ ] **Step 3: Rewrite the module body**

Replace everything in `crates/schema/src/context.rs` ABOVE the `#[cfg(test)]` line (lines 1-45, the doc comment + imports + `RootContext`/`ObjectContext` + their `impl RuleContext`) with:

```rust
//! Builds the `nebula_validator::PredicateContext` for a schema's
//! visibility/required evaluation (ADR-0052).
//!
//! Walks the schema field tree and the value tree together, emitting
//! RFC-6901 pointers (`/a/b`) for non-secret `Literal` values at **any
//! depth**, and excluding any `Field::Secret` subtree **by schema type** —
//! pre-resolve a secret is `FieldValue::Literal(plaintext)`, so a
//! runtime-tag check would leak it. Nested resolution is required: the prior
//! flat `RootContext` made every nested predicate fail open (ADR-0052).

use indexmap::IndexMap;
use nebula_validator::{PredicateContext, foundation::FieldPath};

use crate::{
    field::Field,
    key::FieldKey,
    value::{FieldValue, FieldValues},
};

/// Build a `PredicateContext` from the value tree, recursively, excluding
/// every field declared as `Field::Secret` at any depth.
///
/// Only `FieldValue::Literal` leaves are addressable by predicates;
/// expression / list / mode / secret-sentinel subtrees are non-addressable
/// (matches the prior `RootContext` literal-only semantics, now nested).
#[must_use]
pub fn predicate_context_for(fields: &[Field], values: &FieldValues) -> PredicateContext {
    let mut pairs: Vec<(FieldPath, serde_json::Value)> = Vec::new();
    collect_non_secret(fields, values.as_map(), None, &mut pairs);
    PredicateContext::from_fields(pairs)
}

/// Recurse fields ↔ values in lockstep, scrubbing `Field::Secret` at every
/// level and descending `Field::Object` ↔ `FieldValue::Object`.
fn collect_non_secret(
    fields: &[Field],
    values: &IndexMap<FieldKey, FieldValue>,
    prefix: Option<&FieldPath>,
    out: &mut Vec<(FieldPath, serde_json::Value)>,
) {
    for field in fields {
        if matches!(field, Field::Secret(_)) {
            continue; // exclude secret-typed fields by schema type, any depth
        }
        let key = field.key();
        let Some(val) = values.get(key) else { continue };
        let path = match prefix {
            None => FieldPath::single(key.as_str()),
            Some(p) => p.push(key.as_str()),
        };
        match (field, val) {
            (Field::Object(obj), FieldValue::Object(sub)) => {
                collect_non_secret(obj.fields(), sub, Some(&path), out);
            },
            (_, FieldValue::Literal(v)) => {
                out.push((path, v.clone()));
            },
            // Expression / SecretLiteral / List / Mode subtrees are
            // non-addressable by predicates.
            _ => {},
        }
    }
}
```

> Implementer: confirm these exact names against source before coding (each
> is a name confirmation, not a design choice — the recursive scrub-by-type
> behavior is fixed):
> - `FieldValues::as_map() -> &IndexMap<FieldKey, FieldValue>`: `FieldValues`
>   is a newtype `FieldValues(IndexMap<FieldKey, FieldValue>)` (see
>   `crates/schema/src/value.rs:308,328`). If no `as_map()` accessor exists,
>   add a `pub(crate) fn as_map(&self) -> &IndexMap<FieldKey, FieldValue> { &self.0 }`
>   on `impl FieldValues` (value.rs:308) — `context.rs` is in-crate so `&self.0`
>   is also acceptable directly.
> - `Field::Object(ObjectField)` variant and `ObjectField::fields() -> &[Field]`:
>   `ObjectField { fields: Vec<Field> }` is generated by `define_field!`
>   (`crates/schema/src/field.rs:513-514`). Use whatever accessor that macro
>   emits for `fields` (getter `fields()` or the field itself); do not invent.
> - `field.key() -> &FieldKey` (`field.rs:113,1123`), `FieldKey::as_str()`
>   (old `context.rs:20-22`), `FieldPath::single(&str)` / `FieldPath::push(&str)`
>   (`crates/validator/src/rule/context.rs:64-68`).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p nebula-schema context::tests`
Expected: PASS (2 tests). The crate will NOT fully build yet — `validated.rs` still references `RootContext`; that is fixed in Task 7. Run instead:

Run: `cargo nextest run -p nebula-schema --no-run 2>&1 | head -5` only to confirm the failure is the expected `RootContext` reference, not a `context.rs` error. Proceed to Task 7 before committing (these two tasks form one compilable unit).

Nested correctness (the recursive flatten + depth-scrub) is proven
**end-to-end** by Task 7's `nested_required_when_is_enforced_not_fail_open`:
that test's predicate targets `/auth/mode` (a `Field::Object` child) and
cannot pass unless `collect_non_secret` descends `Field::Object ↔
FieldValue::Object` and emits the `/auth/mode` pointer. If Task 7's test
still fails open after Task 7 lands, the bug is in `collect_non_secret`
recursion, not in `validate`.

- [ ] **Step 5: (no commit yet — Task 7 completes the compilable unit)**

---

## Task 7: schema — `validate` builds scrubbed ctx, consumes `FieldPlan` (lockdown #2)

`validate_field` loses its visibility/required block and its `ctx: &dyn
RuleContext` parameter. The `match plan.presence` is the **sole** path into
field-value validation — the runner cannot observe a `Field` without its
`Presence` already matched in the same expression.

**Files:**
- Modify: `crates/schema/src/validated.rs:333-365` (`ValidSchema::validate`)
- Modify: `crates/schema/src/validated.rs:1038-1110` (`validate_field`)
- Modify: `crates/schema/src/validated.rs:1117-1122` (`validate_literal_value` signature)
- Test: `crates/schema/tests/validate_schema.rs` (existing integration test file)

- [ ] **Step 1: Write the failing behavioral test (nested-path required now enforced)**

Append to `crates/schema/tests/validate_schema.rs`:

```rust
#[test]
fn nested_required_when_is_enforced_not_fail_open() {
    use nebula_schema::{Schema, FieldKey, FieldValues};
    use nebula_validator::{Rule, Predicate};
    use nebula_validator::foundation::FieldPath;
    use serde_json::json;

    // `secret_token` is required WHEN /auth/mode == "oauth". The old
    // Rule::evaluate flat-key path silently returned false for the nested
    // path → field was NOT enforced (fail-open). It must now be enforced.
    let schema = Schema::builder()
        .object(FieldKey::new("auth").unwrap(), |o| {
            o.string(FieldKey::new("mode").unwrap())
        })
        .string(FieldKey::new("secret_token").unwrap())
        .required_when(
            FieldKey::new("secret_token").unwrap(),
            Rule::Predicate(Predicate::Eq(
                FieldPath::parse("/auth/mode").unwrap(),
                json!("oauth"),
            )),
        )
        .build()
        .expect("schema builds");

    let values = FieldValues::from_json(&json!({
        "auth": { "mode": "oauth" }
        // secret_token absent
    }))
    .unwrap();

    let err = schema.validate(&values).expect_err("must reject: required field absent");
    assert!(
        err.iter().any(|e| e.code == "required"),
        "nested required_when must be enforced, got: {err:?}"
    );
}
```

> Implementer: confirm the builder methods (`Schema::builder()`, `.object(key, closure)`,
> `.string(key)`, `.required_when(key, Rule)`, `.build()`) against
> `crates/schema/src/schema.rs` and `crates/schema/src/builder/`. If
> `required_when` is named differently (e.g. a field-builder method), adapt
> the construction to the real API — keep the asserted behavior identical
> (absent field + nested predicate true ⇒ `required` error). `FieldValues::from_json`
> is used at `crates/schema/src/validated.rs:320`.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p nebula-schema --test validate_schema nested_required_when_is_enforced_not_fail_open`
Expected: FAIL — currently compiles but the assertion fails (fail-open: no `required` error) OR fails to compile because `validate_field` still threads `&dyn RuleContext`. Either failure is acceptable as the red state.

- [ ] **Step 3: Rewrite `ValidSchema::validate`**

In `crates/schema/src/validated.rs`, replace the body of `validate` (lines 333-365, from `pub fn validate` through its closing `}`) with:

```rust
    pub fn validate(&self, values: &FieldValues) -> Result<ValidValues, ValidationReport> {
        use nebula_validator::policy::{
            FieldPolicyDecl, Presence, RequiredPolicy, VisibilityPolicy, resolve_field_policies,
        };

        use crate::context::predicate_context_for;

        let mut report = ValidationReport::new();
        let ctx = predicate_context_for(&self.0.fields, values);

        // Map schema serde enums → borrowed validator policy (no clones).
        fn vis_policy(m: &crate::mode::VisibilityMode) -> VisibilityPolicy<'_> {
            match m {
                crate::mode::VisibilityMode::Always => VisibilityPolicy::Always,
                crate::mode::VisibilityMode::Never => VisibilityPolicy::Never,
                crate::mode::VisibilityMode::When(r) => VisibilityPolicy::When(r),
            }
        }
        fn req_policy(m: &crate::mode::RequiredMode) -> RequiredPolicy<'_> {
            match m {
                crate::mode::RequiredMode::Never => RequiredPolicy::Optional,
                crate::mode::RequiredMode::Always => RequiredPolicy::Always,
                crate::mode::RequiredMode::When(r) => RequiredPolicy::When(r),
            }
        }

        let paths: Vec<FieldPath> = self
            .0
            .fields
            .iter()
            .map(|f| FieldPath::root().join(f.key().clone()))
            .collect();

        let decls = self.0.fields.iter().enumerate().map(|(i, f)| {
            FieldPolicyDecl::new(
                &paths[i],
                vis_policy(f.visible()),
                req_policy(f.required()),
                values.get(f.key()).is_some(),
            )
        });

        let resolution = resolve_field_policies(decls, &ctx);
        for e in resolution.required_failures.iter() {
            report.push(e.clone());
        }

        // `match plan.presence` is the SOLE path into field-value validation:
        // a `Field` cannot reach value rules without its `Presence` matched
        // here (panel lockdown #2 — data-flow-enforced, not call-order).
        for (i, plan) in resolution.plans.iter().enumerate() {
            match plan.presence {
                Presence::Skipped => continue,
                Presence::Active => {
                    let field = &self.0.fields[i];
                    validate_field(field, values.get(field.key()), &paths[i], &mut report);
                },
            }
        }

        run_root_rules(&self.0.root_rules, values, &mut report);

        if report.has_errors() {
            tracing::warn!(
                target: "nebula_schema::validate",
                error_count = report.errors().count(),
                "validate produced errors"
            );
            return Err(report);
        }

        let warnings: Arc<[ValidationError]> = report
            .iter()
            .filter(|e| e.severity == Severity::Warning)
            .cloned()
            .collect();
        Ok(ValidValues {
            schema: self.clone(),
            values: values.clone(),
            warnings,
        })
    }
```

> The `#[tracing::instrument(...)]` attribute (lines 323-332) and the doc
> comment above `validate` are unchanged — leave them in place; only the
> `pub fn validate … { … }` body is replaced. `resolution.plans` is in the
> same order as `self.0.fields` (Task 4 guarantees input order), so the
> index `i` aligns plan ↔ field ↔ path.

- [ ] **Step 4: Strip the visibility/required block from `validate_field`**

In `crates/schema/src/validated.rs`, change `validate_field`'s signature (lines 1038-1044) — remove the `ctx` parameter:

```rust
fn validate_field(
    field: &Field,
    raw: Option<&FieldValue>,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
```

Delete lines 1045-1060 entirely (the `// Visibility predicate …` comment, the `let visible = match …`, the `if !visible && raw.is_none() { return; }`, the `// Required predicate.` comment, and the `let required = match …`). Also delete the now-orphaned required-emit block at lines 1061-1069 (`if required && is_absent_for_required(field, raw) { report.push(ValidationError::builder("required") … ); return; }`) — required reporting is now owned by `resolve_field_policies`. The function body now begins at the old line 1071:

```rust
    let Some(value) = raw else {
        return;
    };
```

In the same function, find the recursive `validate_field(...)` / `validate_literal_value(...)` calls (the Object/List/Mode recursion further down) and remove the `ctx` argument from each call site so they match the new signature.

- [ ] **Step 5: Drop `ctx` from `validate_literal_value`**

At lines 1117-1122, change the signature to remove the `ctx: &dyn nebula_validator::RuleContext` parameter:

```rust
fn validate_literal_value(
    field: &Field,
    value: &FieldValue,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
```

Remove `ctx` from its body's recursive `validate_field`/`validate_literal_value` calls. `run_rules(field.rules(), &transformed, path, report)` is unchanged (it never took `ctx`). If `is_absent_for_required` is now unused, delete the function; if the compiler reports it still used elsewhere, leave it.

- [ ] **Step 6: Run the build + behavioral + context tests**

Run: `cargo nextest run -p nebula-schema --test validate_schema nested_required_when_is_enforced_not_fail_open`
Expected: PASS.

Run: `cargo nextest run -p nebula-schema context::tests`
Expected: PASS (Task 6's tests, now that the crate compiles).

Run: `cargo nextest run -p nebula-schema --test validate_schema`
Expected: PASS — the whole existing suite still green (visibility/required behavior preserved for the non-nested cases, fixed for nested).

- [ ] **Step 7: Commit (Task 6 + Task 7 are one compilable unit)**

```bash
git -C "$WT" add crates/schema/src/context.rs crates/schema/src/validated.rs crates/schema/tests/validate_schema.rs
git -C "$WT" -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(schema): delegate visibility/required to validator policy; scrub secrets by type"
```

---

## Task 8: Delete `RuleContext` + `Rule::evaluate` (no shim)

**Files:**
- Modify: `crates/validator/src/rule/mod.rs` (delete `:39-53`, `:164-277`)
- Modify: `crates/validator/src/lib.rs:84-86` (drop `RuleContext` from re-export)
- Test: `crates/schema/tests/compile_fail/rule_context_removed.rs` (Task 11 adds the trybuild case; here just ensure nothing references it)

- [ ] **Step 1: Verify no remaining references**

Run: `rg -n "RuleContext|\.evaluate\(|evaluate_predicate_via_rule_context" crates --glob '!**/target/**'`
Expected: matches ONLY in `crates/validator/src/rule/mod.rs` (the definitions to delete), `crates/validator/src/lib.rs` (the re-export), and possibly `crates/validator/src/rule/tests.rs` / docs. There must be **zero** references in `crates/schema/src/**`, `crates/*/src/**` outside validator. If any non-validator src reference exists, fix it to use `Rule::matches`/`policy` before deleting.

- [ ] **Step 2: Delete the trait**

In `crates/validator/src/rule/mod.rs`, delete lines 39-53 (the `/// Borrowed view …` doc comment, `pub trait RuleContext { … }`, and `impl RuleContext for std::collections::HashMap<String, serde_json::Value> { … }`).

- [ ] **Step 3: Delete `Rule::evaluate` and its helper**

In the same file, delete the `evaluate` method (the doc comment block starting `/// Evaluates this rule as a boolean predicate …` through the method's closing `}` — original lines 164-198) and the free function `evaluate_predicate_via_rule_context` with its doc comment (original lines 201-277, ending at the `}` before `/// Four kinds of rule`). Leave `RuleKind`, `predicate_code`, `predicate_error`, and the `impl Validate<serde_json::Value> for Rule` intact.

- [ ] **Step 4: Drop the re-export**

In `crates/validator/src/lib.rs`, change the `pub use rule::{ … };` block (lines 84-86) to remove `RuleContext`:

```rust
pub use rule::{
    DeferredRule, Logic, Predicate, PredicateContext, Rule, RuleKind, ValueRule,
};
```

- [ ] **Step 5: Build both crates**

Run: `cargo check -p nebula-validator -p nebula-schema`
Expected: builds clean. If `rule/tests.rs` had `evaluate` tests, delete those specific test fns (they tested removed behavior; `Rule::matches` tests from Task 2 replace them).

- [ ] **Step 6: Run validator + schema suites**

Run: `cargo nextest run -p nebula-validator -p nebula-schema`
Expected: PASS (full suites).

- [ ] **Step 7: Commit**

```bash
git -C "$WT" add crates/validator/src/rule/mod.rs crates/validator/src/lib.rs crates/validator/src/rule/tests.rs
git -C "$WT" -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "refactor(validator)!: delete RuleContext and Rule::evaluate (ADR-0052, no shim)"
```

---

## Task 9: schema lint `secret.predicate_on_value` (#1 lint half)

Reject a schema where any value-comparing predicate (`Eq/Ne/Gt/Gte/Lt/Lte/IsTrue/IsFalse/Contains/Matches/In`)
targets a `Field::Secret` path. `Set`/`Empty` (presence-only) stay legal.
Mirror the `secret.default_forbidden` pattern.

**Files:**
- Modify: `crates/schema/src/lint.rs` (add fn + call from `lint_root_rules`/the field-rule lint near `:51`/`:605`)
- Test: `crates/schema/tests/lint_and_loader.rs` (existing lint integration file)

- [ ] **Step 1: Write the failing test**

Append to `crates/schema/tests/lint_and_loader.rs`:

```rust
#[test]
fn value_predicate_targeting_secret_is_rejected() {
    use nebula_schema::{Schema, FieldKey};
    use nebula_validator::{Rule, Predicate};
    use nebula_validator::foundation::FieldPath;
    use serde_json::json;

    // A visibility rule that compares the *value* of a secret field.
    let res = Schema::builder()
        .secret(FieldKey::new("api_key").unwrap())
        .string(FieldKey::new("region").unwrap())
        .visible_when(
            FieldKey::new("region").unwrap(),
            Rule::Predicate(Predicate::Eq(
                FieldPath::parse("api_key").unwrap(),
                json!("prod-key"),
            )),
        )
        .build();

    let err = res.expect_err("schema with value-predicate on secret must not build");
    assert!(
        err.iter().any(|e| e.code == "secret.predicate_on_value"),
        "expected secret.predicate_on_value, got: {err:?}"
    );
}

#[test]
fn presence_predicate_on_secret_is_allowed() {
    use nebula_schema::{Schema, FieldKey};
    use nebula_validator::{Rule, Predicate};
    use nebula_validator::foundation::FieldPath;

    // Set/Empty (presence only) on a secret is fine.
    Schema::builder()
        .secret(FieldKey::new("api_key").unwrap())
        .string(FieldKey::new("region").unwrap())
        .visible_when(
            FieldKey::new("region").unwrap(),
            Rule::Predicate(Predicate::Set(FieldPath::parse("api_key").unwrap())),
        )
        .build()
        .expect("presence predicate on secret is allowed");
}
```

> Implementer: match the real builder API for `.secret(key)` / `.visible_when(key, Rule)`
> as in Task 7's note. Keep the asserted codes/behavior identical.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p nebula-schema --test lint_and_loader value_predicate_targeting_secret_is_rejected`
Expected: FAIL — schema builds (no such lint yet), `expect_err` panics.

- [ ] **Step 3: Implement the lint**

In `crates/schema/src/lint.rs`, add this function (place it next to `lint_rule_refs_new`, near line 605):

```rust
/// Reject value-comparing predicates whose `FieldPath` targets a
/// `Field::Secret`. Presence-only predicates (`Set`/`Empty`) are allowed —
/// a secret's *value* must never be a visibility/required discriminant.
fn lint_secret_predicate_on_value(
    fields: &[Field],
    rules_with_paths: &[(&Rule, &FieldPath)],
    report: &mut ValidationReport,
) {
    use nebula_validator::Predicate;

    // Collect the set of secret-field top-level keys.
    let secret_keys: std::collections::HashSet<&str> = fields
        .iter()
        .filter(|f| matches!(f, Field::Secret(_)))
        .map(|f| f.key().as_str())
        .collect();
    if secret_keys.is_empty() {
        return;
    }

    fn collect_value_predicates<'a>(rule: &'a Rule, out: &mut Vec<&'a Predicate>) {
        match rule {
            Rule::Predicate(p) => match p {
                Predicate::Set(_) | Predicate::Empty(_) => {},
                _ => out.push(p),
            },
            Rule::Logic(l) => {
                for c in l.children() {
                    collect_value_predicates(c, out);
                }
            },
            Rule::Described(inner, _) => collect_value_predicates(inner, out),
            Rule::Value(_) | Rule::Deferred(_) => {},
        }
    }

    for (rule, _owner) in rules_with_paths {
        let mut preds = Vec::new();
        collect_value_predicates(rule, &mut preds);
        for p in preds {
            let target = p.field().as_str().trim_start_matches('/');
            if secret_keys.contains(target) {
                report.push(
                    ValidationError::builder("secret.predicate_on_value")
                        .message(format!(
                            "predicate targets secret field `{target}` by value; \
                             only Set/Empty (presence) predicates may reference a secret"
                        ))
                        .build(),
                );
            }
        }
    }
}
```

> Implementer: confirm `Logic::children()` exists (used at
> `crates/validator/src/rule/mod.rs:155`), `Predicate::field()` (`predicate.rs:47`),
> `ValidationError::builder("code").message(..).build()` (pattern at
> `lint.rs:154-160`, the `secret.default_forbidden` site), and
> `report.push(..)`. `Rule`/`Field`/`FieldPath`/`ValidationError`/`ValidationReport`
> are already imported in `lint.rs` (see the file header `use` block).

- [ ] **Step 4: Wire the lint into the schema build**

Find where root rules and per-field `When(Rule)` rules are linted. `lint_root_rules(rules, fields, report)` is at `crates/schema/src/lint.rs:51`. Add a call to the new lint that passes every `When(Rule)` from each field's `visible()`/`required()` plus the root rules. In the function that drives field linting (the same place `lint_rule_refs_new` is invoked, near `:605`), assemble the rule list and call:

```rust
    // Gather all rules that carry predicates: root rules + each field's
    // visibility/required When(rule).
    let mut rules_with_paths: Vec<(&Rule, &FieldPath)> = Vec::new();
    for r in root_rules {
        rules_with_paths.push((r, root_path));
    }
    for field in fields {
        if let crate::mode::VisibilityMode::When(r) = field.visible() {
            rules_with_paths.push((r, prefix));
        }
        if let crate::mode::RequiredMode::When(r) = field.required() {
            rules_with_paths.push((r, prefix));
        }
    }
    lint_secret_predicate_on_value(fields, &rules_with_paths, report);
```

> Implementer: adapt variable names (`root_rules`, `root_path`, `prefix`,
> `fields`) to whatever the surrounding lint driver already has in scope —
> read 20 lines around `lint_rule_refs_new`'s call site and reuse its inputs.
> The lint must run for both root rules and nested scopes (it is already
> called per-scope if you hook it where `lint_rule_refs_new` is hooked).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo nextest run -p nebula-schema --test lint_and_loader value_predicate_targeting_secret_is_rejected presence_predicate_on_secret_is_allowed`
Expected: PASS (2 tests).

- [ ] **Step 6: Run the full schema suite (no regressions)**

Run: `cargo nextest run -p nebula-schema`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git -C "$WT" add crates/schema/src/lint.rs crates/schema/tests/lint_and_loader.rs
git -C "$WT" -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(schema): lint secret.predicate_on_value (no secret value as discriminant)"
```

---

## Task 10: ADR-0052 seam test — proof-token custody (P1 merge-blocker, lockdown #3)

**Files:**
- Create: `crates/schema/tests/seam_proof_token_custody.rs`

- [ ] **Step 1: Write the seam test**

Create `crates/schema/tests/seam_proof_token_custody.rs`:

```rust
//! ADR-0052 seam test: `ValidValues`/`ResolvedValues` are mintable ONLY via
//! the `nebula-schema` pipeline. Moving condition evaluation into
//! `nebula-validator` must not add a back-door constructor.

use nebula_schema::{FieldKey, FieldValues, Schema};
use serde_json::json;

#[test]
fn valid_values_only_minted_by_validate() {
    let schema = Schema::builder()
        .string(FieldKey::new("name").unwrap())
        .build()
        .expect("schema builds");

    let values = FieldValues::from_json(&json!({"name": "alice"})).unwrap();

    // The ONLY way to obtain ValidValues is ValidSchema::validate.
    let vv = schema.validate(&values).expect("validates");
    assert_eq!(vv.schema().0.fields.len(), 1, "token carries the schema it was minted from");
}

#[test]
fn condition_eval_did_not_leak_token_constructors() {
    // Compile-time guarantee proxy: there is no public associated fn on
    // ValidValues/ResolvedValues other than the pipeline. This test exists
    // so a future `ValidValues::new`/`from_*` addition forces a deliberate
    // edit to this file (the seam contract lives here, per ADR-0052).
    fn _assert_no_public_ctor() {
        // If someone adds `ValidValues::new(...)` this file must be revisited.
        // Intentionally empty — presence of the test is the contract anchor.
    }
    _assert_no_public_ctor();
}
```

> Implementer: if `vv.schema().0` is not accessible from an integration test
> (the `.0` tuple field may be `pub(crate)`), replace that assertion with a
> public accessor that exists (e.g. `vv.schema()` returning `&ValidSchema`
> and any public method on it), or assert `schema.validate(&bad).is_err()`
> for an invalid input. Keep the core contract: `ValidValues` is obtained
> only from `validate`.

- [ ] **Step 2: Run it**

Run: `cargo nextest run -p nebula-schema --test seam_proof_token_custody`
Expected: PASS (2 tests).

- [ ] **Step 3: Commit**

```bash
git -C "$WT" add crates/schema/tests/seam_proof_token_custody.rs
git -C "$WT" -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "test(schema): ADR-0052 proof-token custody seam test"
```

---

## Task 11: trybuild compile-fail — policy exhaustiveness + `RuleContext` gone

**Files:**
- Create: `crates/schema/tests/compile_fail/policy_presence_non_exhaustive.rs`
- Create: `crates/schema/tests/compile_fail/rule_context_removed.rs`
- Modify: `crates/schema/tests/compile_fail.rs` (register cases)

- [ ] **Step 1: Inspect the trybuild harness**

Read `crates/schema/tests/compile_fail.rs` (it registers `crates/schema/tests/compile_fail/*.rs` via `trybuild::TestCases::compile_fail(...)`). Note the exact registration idiom used (one `t.compile_fail("tests/compile_fail/NAME.rs");` line per case).

- [ ] **Step 2: Create the `Presence` exhaustiveness case**

Create `crates/schema/tests/compile_fail/policy_presence_non_exhaustive.rs`:

```rust
//! A `match` on `Presence` missing the `Skipped` arm must NOT compile —
//! this is what makes "forgot to skip hidden fields" structurally impossible.
fn main() {
    let p = nebula_validator::Presence::Active;
    // Non-exhaustive + missing arm → E0004.
    match p {
        nebula_validator::Presence::Active => {},
    }
}
```

- [ ] **Step 3: Create the `RuleContext`-removed case**

Create `crates/schema/tests/compile_fail/rule_context_removed.rs`:

```rust
//! `nebula_validator::RuleContext` and `Rule::evaluate` were deleted by
//! ADR-0052 (no shim). Referencing them must not compile.
fn main() {
    fn _needs_ctx<T: nebula_validator::RuleContext>(_: T) {}
}
```

- [ ] **Step 4: Register both cases**

In `crates/schema/tests/compile_fail.rs`, add (matching the file's existing idiom):

```rust
    t.compile_fail("tests/compile_fail/policy_presence_non_exhaustive.rs");
    t.compile_fail("tests/compile_fail/rule_context_removed.rs");
```

- [ ] **Step 5: Run trybuild**

Run: `cargo nextest run -p nebula-schema --test compile_fail`
Expected: PASS — both new cases fail to compile as required (trybuild reports them as expected failures). If trybuild prompts to create `.stderr` files, run with `TRYBUILD=overwrite cargo test -p nebula-schema --test compile_fail` once, inspect the generated `.stderr` for sanity (E0004 for the match; E0405/E0412 "cannot find trait `RuleContext`" for the other), then re-run without the env var.

- [ ] **Step 6: Commit**

```bash
git -C "$WT" add crates/schema/tests/compile_fail.rs crates/schema/tests/compile_fail/
git -C "$WT" -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "test(schema): compile-fail for Presence exhaustiveness + RuleContext removal"
```

---

## Task 12: P1 verification gate + PR

**Files:** none (verification + PR only)

- [ ] **Step 1: Format**

Run: `task fmt`
Expected: clean (no diff). If it reformats, `git add -A && git commit -m "style(schema): fmt"`.

- [ ] **Step 2: Clippy (workspace, -D warnings)**

Run: `task clippy`
Expected: zero warnings. Fix any clippy finding in the touched files before proceeding (no `#[allow]` without a one-line reason matching the codebase convention).

- [ ] **Step 3: Targeted test suites**

Run: `cargo nextest run -p nebula-validator -p nebula-schema --all-features`
Expected: PASS (all).

- [ ] **Step 4: Doctests for touched crates**

Run: `cargo test -p nebula-validator -p nebula-schema --doc`
Expected: PASS.

- [ ] **Step 5: Rustdoc (warnings as errors)**

Run: `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p nebula-validator -p nebula-schema`
Expected: clean.

- [ ] **Step 6: cargo-deny (layer wrappers unchanged)**

Run: `task deny`
Expected: PASS — confirms no new cross-layer edge was introduced (schema→validator was already legal).

- [ ] **Step 7: Full pre-PR gate**

Run: `task dev:check`
Expected: PASS (fmt + clippy + nextest + doctests + deny).

- [ ] **Step 8: Open the PR**

```bash
git -C "$WT" push -u origin HEAD
gh pr create --repo <origin> \
  --title "feat(schema): P1 — visibility/required engine → nebula-validator (ADR-0052)" \
  --body "Implements P1 of docs/superpowers/specs/2026-05-15-nebula-schema-finalization-design.md. ADR-0052 + proof-token-custody seam test included (canon §0.1/§17, P1 merge-blocker). Deletes RuleContext/Rule::evaluate (no shim); fixes nested-path required fail-open; scrubs Field::Secret from predicate context by schema type; redacts PredicateContext Debug. P2–P4 follow in separate PRs."
```

Expected: PR created, CI required jobs (fmt, clippy, nextest, doctests, MSRV, deny) green. ADR-0052 is in the diff (merge-blocker satisfied).

---

## Panel plan-objection round (4/4 — NO BLOCKING OBJECTION) — BINDING hardenings

All four panelists cleared the corrected plan. Each returned one substantive
**non-blocking** refinement; they converge (SRP + type-system independently
on plan↔field alignment; canon on seam-test theater; security on lint depth).
These are **merge-blocking for P1** — the implementer MUST apply them in the
named tasks. They do not change the design; they harden it.

**H1 — `resolve_field_policies` ordering invariant (Task 4; SRP + type-system).**
In Task 4 Step 3, the `resolve_field_policies` doc comment MUST state, and the
body MUST honor: *one `FieldPlan` pushed per input decl, in input order, with
no filtering, reordering, dedup, or early `continue` that skips a push*. A
`Presence::Skipped` decl still pushes its plan (with `presence: Skipped`). The
Task 4 loop already does unconditional push-per-decl — it must stay that way.
Add this line to the doc comment:

```rust
/// INVARIANT: exactly one `FieldPlan` is emitted per input decl, in input
/// order — never filtered, reordered, or deduped. Callers rely on positional
/// `plans[i] ↔ decls[i]` correspondence; breaking it silently misvalidates.
```

**H2 — caller alignment guard (Task 7 Step 3; type-system + SRP).** In the
`ValidSchema::validate` body, immediately before the `for (i, plan) in
resolution.plans.iter().enumerate()` loop, insert (no mutation of
`resolution.plans` may occur between `resolve_field_policies` and this loop;
the `required_failures` drain above is read-only and is fine):

```rust
        // INVARIANT: plan index ≡ field index ≡ path index. `resolve_field_policies`
        // emits one plan per decl in input order (H1). Any future filter/reorder
        // of `plans` silently breaks the presence gate — the correct P2 fix is to
        // carry `&'a Field` (or an opaque token minted by resolve_field_policies)
        // inside `FieldPlan` so the runner cannot obtain a `Field` except via the
        // matched plan. Do NOT `retain`/`sort`/filter `resolution.plans`.
        debug_assert_eq!(
            resolution.plans.len(),
            self.0.fields.len(),
            "policy resolution must be 1:1 with schema fields (H1)"
        );
```

**H3 — middle-skipped alignment regression test (new; SRP).** Append to
`crates/schema/tests/validate_schema.rs` (Task 7 Step 1's file) a test with
**≥3 top-level fields where a MIDDLE field is `VisibilityMode::Never`**, with
distinct value-rule violations on the first and last fields, asserting the
reported error paths point at the first and last fields (not shifted by the
skipped middle). This catches an off-by-one that the 1-2 field cases cannot:

```rust
#[test]
fn middle_skipped_field_does_not_shift_plan_to_field_mapping() {
    use nebula_schema::{Schema, FieldKey, FieldValues};
    use serde_json::json;

    // f_first (min_length 5), f_mid (Never-visible), f_last (min_length 5).
    // Both f_first and f_last get too-short values. If plan↔field shifts by
    // the skipped middle, the error paths land on the wrong fields.
    let schema = Schema::builder()
        .string(FieldKey::new("f_first").unwrap())
        .min_length(FieldKey::new("f_first").unwrap(), 5)
        .string(FieldKey::new("f_mid").unwrap())
        .visible(FieldKey::new("f_mid").unwrap(), nebula_schema::VisibilityMode::Never)
        .string(FieldKey::new("f_last").unwrap())
        .min_length(FieldKey::new("f_last").unwrap(), 5)
        .build()
        .expect("schema builds");

    let values = FieldValues::from_json(&json!({"f_first": "ab", "f_last": "cd"})).unwrap();
    let err = schema.validate(&values).expect_err("both short fields must fail");

    let codes_paths: Vec<(String, String)> = err
        .iter()
        .map(|e| (e.code.to_string(), e.field.as_deref().unwrap_or("").to_string()))
        .collect();
    assert!(
        codes_paths.iter().any(|(_, p)| p.contains("f_first")),
        "f_first must be the error path, not shifted: {codes_paths:?}"
    );
    assert!(
        codes_paths.iter().any(|(_, p)| p.contains("f_last")),
        "f_last must be the error path, not shifted: {codes_paths:?}"
    );
}
```

> Implementer: adapt `.min_length(key, n)` / `.visible(key, VisibilityMode)`
> to the real builder API (same verification discipline as Task 7 Step 1).
> Keep the asserted property identical: with a middle field skipped, the
> first/last fields' error paths are NOT shifted. `ValidationError::field`
> accessor — confirm name (`e.field` / `e.field_pointer()`); the assertion
> only needs the path string to contain the field key.

**H4 — replace the theater seam test with a real structural guarantee
(Task 10 + Task 11; canon).** In Task 10, **delete** the
`condition_eval_did_not_leak_token_constructors` test and its empty
`_assert_no_public_ctor()` body entirely (canon §14 "green tests, wrong
product" — it asserts nothing). Keep `valid_values_only_minted_by_validate`
(the real custody assertion that alone satisfies canon §0.1/§17). Replace the
deleted test's guarantee with a trybuild compile-fail case. Add to Task 11 a
new file `crates/schema/tests/compile_fail/valid_values_no_public_ctor.rs`:

```rust
//! ADR-0052 custody: ValidValues has no public constructor other than the
//! pipeline. Any `new`/`from_*` ctor must NOT exist.
fn main() {
    let _ = nebula_schema::ValidValues::new();
}
```

and register it in `crates/schema/tests/compile_fail.rs` next to the other two
Task 11 cases:

```rust
    t.compile_fail("tests/compile_fail/valid_values_no_public_ctor.rs");
```

Expected trybuild failure: `no function or associated item named `new` found
for struct `ValidValues``. If `ValidValues` is not publicly re-exported,
qualify via the real public path (`nebula_schema::validated::ValidValues` per
`crates/schema/src/lib.rs:239`); do not make it constructible to satisfy the
test.

**H5 — Task 11 trybuild is necessary-but-not-sufficient (note; type-system +
canon).** Add to Task 11 (after Step 5) and to the Self-Review a one-line
statement: the `policy_presence_non_exhaustive.rs` case proves `Presence` is
`#[non_exhaustive]` + the `Skipped` arm is mandatory; it does **not** prove
the runner cannot obtain a `Field` outside the matched arm. That stronger
guarantee is the named **P2 hardening**: `FieldPlan` carries `&'a Field` /
opaque token. P1 relies on H1+H2 (invariant + `debug_assert` + comment) for
that property; this is explicitly an interim, not the end state.

**H6 — `secret.predicate_on_value` lint must recurse objects (Task 9;
security).** In Task 9 Step 3, `lint_secret_predicate_on_value`'s
`secret_keys` collection MUST recurse `Field::Object` so a value-predicate
targeting a nested secret (`/auth/api_key` where `api_key` is `Field::Secret`
inside object `auth`) is also flagged — matching Task 6's depth-awareness.
Collect normalized RFC-6901 paths of every `Field::Secret` at any depth and
compare against the predicate's normalized `p.field()` pointer. This is
advisory completeness only — runtime #1 is already closed by Task 6's
recursive scrub (the predicate fails closed against an absent value), so this
is a quality refinement, not a leak fix; still required for P1.

**P2 backlog (record so it is not lost):** (a) `FieldPlan` carries `&'a Field`
/ opaque token so plan↔field desync is unrepresentable (type-system end
state); (b) single schema→validator behavioral crossing + a test asserting it
(delete `run_root_rules`/`validator_bridge.rs`, SRP lockdown #1 end state);
(c) these belong to the P2 plan authored against P1's landed signatures.

## Self-Review

**1. Spec coverage (P1 slice of `2026-05-15-nebula-schema-finalization-design.md`):**
- `policy` module + `Rule::matches` → Tasks 1-5. ✓
- Delete `RuleContext`/`Rule::evaluate` (no shim) → Task 8. ✓
- schema delegates; `RequiredMode::When` wired nested-correct (closes fail-open) → Task 7 (test `nested_required_when_is_enforced_not_fail_open`). ✓
- secret scrub #1 (by `Field::Secret` schema type, pre-resolve plaintext, **recursive — scrubbed at any depth**; the recursive flatten also delivers the nested-path resolution that fixes the §4.5 `required` fail-open) → Task 6 + lint Task 9, proven E2E by Task 7. ✓
- `PredicateContext` redacting Debug #5 → Task 5. ✓
- ADR-0052 + seam test inside P1 PR (lockdown #3) → Task 0 + Task 10 + Task 12 Step 8. ✓
- Data-flow-enforced `FieldPlan` consumption (lockdown #2) → Task 7 Step 3 (`match plan.presence` sole path) + Task 11 (compile-fail proof). ✓
- Single validator entry point pinned (lockdown #1) → Task 4 (`resolve_field_policies` signature fixed; `required_failures` validator-owned). ✓
- #2 slot_bindings — correctly absent (spec Non-goal; separate task). ✓
- P2-only items (`run_rules`/`validator_bridge`/`derive_schema.rs:95`/json-schema/regex) correctly absent. ✓

**2. Placeholder scan:** No "TBD"/"add error handling"/"similar to". Implementer notes reference exact file:line to confirm an API name, not to invent one — all types/methods used (`Rule`, `Predicate`, `PredicateContext`, `VisibilityMode`/`RequiredMode`, `Field::Secret`, `ValidationError::new`/`builder`, `report.push`) are read from real source. ✓

**3. Type consistency:** `resolve_field_policies(decls, ctx) -> FieldPolicyResolution { plans, required_failures }`; `FieldPolicyDecl::new(path, visibility, required, value_present)`; `Presence::{Active,Skipped}`; `Requiredness::{Required,Optional}`; `VisibilityPolicy::{Always,Never,When}` / `RequiredPolicy::{Optional,Always,When}`; `Rule::matches(&PredicateContext)`; `predicate_context_for(&[Field], &FieldValues)`. Names identical across Tasks 1-11. ✓

> Open verification items the implementer MUST confirm against source before
> coding the step (each flagged inline): exact `ValidationErrors` mutator
> (`push` vs `add`); builder API names (`.visible_when`/`.required_when`/`.secret`);
> `Logic::children`; `vv.schema().0` visibility from an integration test.
> These are name confirmations, not design gaps — the behavior asserted by
> each test is fixed.
