# nebula-validator — Rule Type Split Refactor (Refactor 1)

**Status:** Design
**Date:** 2026-04-17
**Scope:** `crates/validator`, `crates/validator/macros`, `crates/schema`, `crates/parameter`, planning docs referencing `Rule`

## Goal

Replace the flat 30-variant `Rule` enum with a typed sum-of-sums. Lock the shape before real persistence or external API commitments exist. Make illegal state combinations (e.g. calling `validate_value` on a predicate) unrepresentable at compile time without losing the ergonomic "mixed list of rules" pattern that the current design deliberately supports.

Simultaneously, while wire format is still changeable, adopt a compact externally-tagged JSON encoding and a `Described` decorator for custom error messages — eliminating the `"rule":` key redundancy and the per-variant `message: Option<String>` bloat.

## Philosophy

**One method per semantic class, one outer Rule for mixed dispatch.**

- Value validation, context-predicate evaluation, logical combination, and deferred runtime checks are four distinct semantic kinds.
- Each kind deserves its own type with only the method that makes sense for it.
- But consumers commonly receive a heterogeneous list of rules and dispatch through a single engine call — that ergonomic stays.
- The outer `Rule` becomes a classifier that holds the kind; all actual work happens on typed inner enums.

Silent-pass on type mismatch (e.g. `MinLength` on a number → `Ok`) remains as a documented ergonomic *within* `ValueRule`. What goes away is silent-pass *across* kinds (predicate returning `Ok` from a value-validation method).

## Current State Summary

The existing `Rule` (`crates/validator/src/rule/mod.rs`) is a flat enum with `#[serde(tag = "rule")]` and 30 variants across four categories:

| Category | Count | Method that applies | Current behavior on non-applicable method |
|---|---|---|---|
| Value validation | 12 | `validate_value(&Value)` | Predicates return `Ok` silently |
| Context predicate | 13 | `evaluate(&HashMap)` | Value rules return `true` silently |
| Logical combinator | 3 | both | recursive |
| Deferred | 2 | runtime only | skipped at schema time |

Cross-crate consumers: `nebula-schema` (4 files), `nebula-parameter` (re-exports `Rule`), `nebula-validator-macros` (generates `Rule::*` constructors). Planning documents in `docs/superpowers/plans/*-schema-*.md` reference the current API.

No stored data. No external API commitments yet. Alpha stage.

---

## 1. Rule Structure — Sum of Sums

### 1.1 Top-level enum

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Rule {
    Value(ValueRule),
    Predicate(Predicate),
    Logic(Box<Logic>),
    Deferred(DeferredRule),
    Described(Box<Rule>, String),
}
```

The outer `Rule` has minimal behavior: `validate(input, ctx, mode)` dispatch, `kind()` classifier, serde. All real work is on the inner enums.

`Described` is a tuple variant `(Box<Rule>, String)` rather than a separate struct — matches the tuple wire form directly and keeps dispatch code terse.

### 1.2 ValueRule

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ValueRule {
    MinLength(usize),
    MaxLength(usize),
    Pattern(String),
    Min(serde_json::Number),
    Max(serde_json::Number),
    GreaterThan(serde_json::Number),
    LessThan(serde_json::Number),
    OneOf(Vec<serde_json::Value>),
    MinItems(usize),
    MaxItems(usize),
    Email,
    Url,
}

impl ValueRule {
    pub fn validate_value(&self, input: &serde_json::Value) -> Result<(), ValidationError> { ... }
}
```

`validate_value` is the only method on `ValueRule`. Silent-pass on JSON type mismatch (e.g. `MinLength` on a number) is preserved as documented ergonomic.

Single-scalar variants become tuple variants (`MinLength(usize)` instead of `MinLength { min: usize }`). This aligns with compact wire form and removes the redundant field name noise in Rust code for no cost.

### 1.3 Predicate

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Predicate {
    Eq(FieldPath, serde_json::Value),
    Ne(FieldPath, serde_json::Value),
    Gt(FieldPath, serde_json::Number),
    Gte(FieldPath, serde_json::Number),
    Lt(FieldPath, serde_json::Number),
    Lte(FieldPath, serde_json::Number),
    IsTrue(FieldPath),
    IsFalse(FieldPath),
    Set(FieldPath),
    Empty(FieldPath),
    Contains(FieldPath, serde_json::Value),
    Matches(FieldPath, String),
    In(FieldPath, Vec<serde_json::Value>),
}

impl Predicate {
    pub fn evaluate(&self, ctx: &PredicateContext) -> bool { ... }
    pub fn field(&self) -> &FieldPath { ... }
}
```

`evaluate` is the only method that applies. `field()` helper returns the referenced field path without match boilerplate.

All predicates use `FieldPath` (not raw `String`) — RFC 6901 validated at construction.

### 1.4 Logic

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Logic {
    All(Vec<Rule>),
    Any(Vec<Rule>),
    Not(Rule),
}

impl Logic {
    pub fn validate(
        &self,
        input: &Value,
        ctx: Option<&PredicateContext>,
        mode: ExecutionMode,
    ) -> Result<(), ValidationError> { ... }

    pub fn walk(&self) -> impl Iterator<Item = &Rule> { ... }
}
```

Children are `Rule` (mixed-kind allowed — matches current semantics).

### 1.5 DeferredRule

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeferredRule {
    Custom(String),         // expression string; typing deferred to Refactor 2
    UniqueBy(FieldPath),
}

impl DeferredRule {
    pub fn validate(&self, input: &Value, ctx: Option<&PredicateContext>) -> Result<(), ValidationError> { ... }
}
```

`Custom` stays as `String` in Refactor 1. Typing via `nebula-expression` integration is **Refactor 2** (separate PR). The `#[non_exhaustive]` attribute leaves room for future `AsyncCustom(...)` or similar.

### 1.6 Described decorator

The `Described` variant is a tuple on `Rule` itself — `Described(Box<Rule>, String)`. No separate struct.

```rust
// Construction
Rule::Described(Box::new(inner_rule), message)

// Or via sugar
Rule::described(inner_rule, "custom message")
inner_rule.with_message("custom message")
```

Replaces the per-variant `message: Option<String>` fields. Wraps any `Rule` (including `Logic` and nested `Described`) to override the resulting error's message. Nesting behavior: outer message wins (inner message is overridden by outer `map_err`).

Wire form: `{"described": [{"min_length": 3}, "too short"]}` — tuple of (rule, message).

This is more powerful than the current inline `message` field: it works for combinators too, where previously only leaf rules could carry a custom message.

### 1.7 Message templates (named placeholders)

The `String` in `Described` may be a plain message **or** a template with named placeholders. Templates render at error display time using parameters attached to the `ValidationError`.

Example:

```rust
Rule::min_length(3).with_message("got {value}, expected at least {min} chars")
```

When this rule fails on input `"hi"`, the resulting error renders as:
```
got "hi", expected at least 3 chars
```

#### 1.7.1 Placeholder catalog

Common (available on any rule):
- `{value}` — the input that failed
- `{field}` — field path (empty if no field context)

Rule-specific (injected automatically by the rule's `validate_value` / `evaluate`):

| Rule | Placeholders |
|---|---|
| `MinLength`, `MinItems`, `Min`, `GreaterThan` | `{min}` |
| `MaxLength`, `MaxItems`, `Max`, `LessThan` | `{max}` |
| `Pattern`, `Matches` | `{pattern}` |
| `OneOf`, `In` | `{allowed}` (comma-separated) |
| `Eq`, `Ne`, `Gt`, `Gte`, `Lt`, `Lte`, `Contains` | `{expected}` |
| `Email`, `Url`, `Set`, `Empty`, `IsTrue`, `IsFalse` | — (common only) |

New rules added in the future declare their own placeholders via their `validate_*` implementation — adding a placeholder never breaks existing templates.

#### 1.7.2 Rendering

`ValidationError` already carries a `SmallVec<[(&'static str, Value); 2]>` params slot in `ErrorExtras`. Rendering is a single-pass walk:

```rust
pub fn render_message(template: &str, params: &[(&str, &Value)]) -> Cow<'_, str> {
    if !template.contains('{') { return Cow::Borrowed(template); }  // hot path
    // walk, substitute {name} with matching param
}
```

`Display` impl for `ValidationError` checks `template.contains('{')` — **no overhead** for plain messages. Templates cost one allocation per failure.

#### 1.7.3 Unknown placeholder handling

`{unknown}` in a template where no such param exists — rendered as literal `{unknown}` (no error). This avoids noisy failures when templates reference placeholders not produced by a given rule, and makes templates cheap to write.

For stricter checking, macros at emit time validate that placeholders in user-written messages match the rule's declared set — compile-time warning (not error) on unknown placeholder.

#### 1.7.4 Escape

Literal `{` / `}` in a message — doubled: `{{` / `}}`. Matches Rust `format!()` convention.

---

## 2. Wire Format — Externally Tagged, Tuple-Compact

### 2.1 Before/after comparison

| Shape | Old | New |
|---|---|---|
| Value with scalar | `{"rule":"min_length","min":3}` | `{"min_length":3}` |
| Value unit | `{"rule":"email"}` | `"email"` |
| Predicate | `{"rule":"eq","field":"status","value":"active"}` | `{"eq":["status","active"]}` |
| Logic | `{"rule":"all","rules":[...]}` | `{"all":[...]}` |
| Not | `{"rule":"not","inner":{...}}` | `{"not":{...}}` |
| With message | `{"rule":"min_length","min":3,"message":"too short"}` | `{"described":[{"min_length":3},"too short"]}` |

### 2.2 Sample compound rule

Old (115 chars):
```json
[
  {"rule":"min_length","min":3},
  {"rule":"max_length","max":100},
  {"rule":"email"}
]
```

New (45 chars — 61% reduction):
```json
[
  {"min_length":3},
  {"max_length":100},
  "email"
]
```

### 2.3 Serde approach

Default serde externally-tagged representation for each inner enum via `#[derive(Serialize, Deserialize)]` + `#[serde(rename_all = "snake_case")]`. Tuple variants for scalar cases.

For `Rule` itself — outer enum must dispatch to inner on deserialize. Two approaches:

- **Approach A:** Manual `Deserialize` that reads the first key (or bare-string for unit variants), looks up which inner enum owns that tag, and deserializes into it. Clean error messages. ~100 lines of glue code.
- **Approach B:** `#[serde(untagged)]` on outer `Rule`, let serde try each variant. Cleaner code (~5 lines) but buffering overhead and generic error messages.

**Decision: Approach A.** The error quality matters — future users will pass malformed JSON. Manual impl gives us "unknown rule `min_lenght` (did you mean `min_length`?)" instead of "data did not match any variant".

### 2.4 Error messages for malformed input

Example failure: `{"min_lenght": 3}` (typo).

Expected error: `unknown rule "min_lenght" at $.path. Known rules: min_length, max_length, pattern, min, max, greater_than, less_than, one_of, min_items, max_items, email, url, eq, ne, gt, gte, lt, lte, is_true, is_false, set, empty, contains, matches, in, all, any, not, custom, unique_by, described`

Implementation via manual `Deserialize` that holds a static `KNOWN_RULES` list and emits a proper `serde::de::Error::unknown_variant`.

---

## 3. FieldPath Integration

`FieldPath` already exists in `crates/validator/src/foundation/field_path.rs` — 400 lines, RFC 6901, escape-safe, well-tested. No changes to the type itself.

**Integration work:**
- Add `impl Serialize for FieldPath` — serialize as inner `&str`.
- Add `impl<'de> Deserialize<'de> for FieldPath` — deserialize `String`, then `FieldPath::parse(...)` on it.
- Expose `FieldPath` publicly via prelude and re-export from `rule::*` for consumers.
- Replace all `field: String` in `Predicate` variants with `field: FieldPath`.
- Replace `UniqueBy { key: String }` with `UniqueBy(FieldPath)`.

Wire format for `FieldPath` is just a string (`"/status"`, `"/user/email"`) — transparent.

---

## 4. Engine Dispatch

### 4.1 Single entry point

```rust
impl Rule {
    pub fn validate(
        &self,
        input: &serde_json::Value,
        ctx: Option<&PredicateContext>,
        mode: ExecutionMode,
    ) -> Result<(), ValidationError> {
        match self {
            Rule::Value(v) => v.validate_value(input),
            Rule::Predicate(p) => match ctx {
                Some(c) => {
                    if p.evaluate(c) { Ok(()) }
                    else { Err(ValidationError::predicate_failed(p)) }
                }
                None => Ok(()),
            },
            Rule::Logic(l) => l.validate(input, ctx, mode),
            Rule::Deferred(_) if mode == ExecutionMode::StaticOnly => Ok(()),
            Rule::Deferred(d) => d.validate(input, ctx),
            Rule::Described(inner, message) => inner.validate(input, ctx, mode)
                .map_err(|e| e.with_message(message)),
        }
    }
}
```

### 4.2 Semantics for `Predicate` without context

`ctx = None` → return `Ok(())`. Matches current `ExecutionMode::StaticOnly` semantics and aligns with the two-tier validation model:
- **Client / edge:** sync validation with no context → skips predicates (they'd need field lookups the client doesn't do).
- **Server:** full validation with context → evaluates predicates.

This is intentional silent-pass at the dispatch level (not at the method level). Documented in the `validate` method's doc-comment.

### 4.3 PredicateContext

Current code uses `HashMap<String, serde_json::Value>`. We introduce a typed newtype:

```rust
pub struct PredicateContext {
    fields: HashMap<FieldPath, serde_json::Value>,
}

impl PredicateContext {
    pub fn get(&self, path: &FieldPath) -> Option<&serde_json::Value> { ... }
    pub fn from_json(obj: &serde_json::Value) -> Self { ... }
}
```

Shields consumers from the raw `HashMap` and provides `FieldPath`-aware lookup.

### 4.4 Backward-compat trait impl

```rust
impl Validate<serde_json::Value> for Rule {
    fn validate(&self, input: &serde_json::Value) -> Result<(), ValidationError> {
        Rule::validate(self, input, None, ExecutionMode::StaticOnly)
    }
}
```

Existing `Validate<T>` trait-based callers continue to work without changes.

---

## 5. Constructor API

Ergonomics rely heavily on short constructor functions at both the typed sub-enum level and the wrapped `Rule` level.

```rust
impl ValueRule {
    pub fn min_length(n: usize) -> Self { ValueRule::MinLength(n) }
    pub fn max_length(n: usize) -> Self { ValueRule::MaxLength(n) }
    pub fn pattern(p: impl Into<String>) -> Self { ValueRule::Pattern(p.into()) }
    // ... for each variant
}

impl Predicate {
    pub fn eq(field: impl TryInto<FieldPath>, v: impl Into<serde_json::Value>) -> Result<Self, FieldPathError> { ... }
    // ... for each variant
}

impl Rule {
    pub fn min_length(n: usize) -> Self { Rule::Value(ValueRule::min_length(n)) }
    pub fn pattern(p: impl Into<String>) -> Self { Rule::Value(ValueRule::pattern(p)) }
    pub fn email() -> Self { Rule::Value(ValueRule::Email) }
    pub fn all(rules: impl IntoIterator<Item = Rule>) -> Self {
        Rule::Logic(Box::new(Logic::All(rules.into_iter().collect())))
    }
    pub fn described(rule: Rule, message: impl Into<String>) -> Self {
        Rule::Described(Box::new(rule), message.into())
    }
    // ... for each common variant, plus helpers

    pub fn with_message(self, message: impl Into<String>) -> Self {
        Rule::described(self, message)
    }
}
```

Typical usage:

```rust
let rules = vec![
    Rule::min_length(3).with_message("too short"),
    Rule::max_length(100),
    Rule::email(),
    Rule::all([
        Rule::pattern("^[a-z]+$"),
        Rule::any([Rule::set("email"), Rule::set("phone")]),
    ]),
];
```

---

## 6. Macro Emit Strategy (derive)

User-facing `#[validate(...)]` attribute syntax is **unchanged**. The emit side uses typed sub-enums directly instead of wrapping everything through `Rule`.

### 6.1 Field-level value rules — typed bypass

```rust
#[validate(min_length = 3, max_length = 20, email)]
email: String
```

Emits direct `ValueRule` calls (no `Rule::Value(...)` wrapping):

```rust
let v = serde_json::json!(self.email);
if let Err(e) = ValueRule::MinLength(3).validate_value(&v) {
    errors.push(e.with_field("email"));
}
if let Err(e) = ValueRule::MaxLength(20).validate_value(&v) {
    errors.push(e.with_field("email"));
}
if let Err(e) = ValueRule::Email.validate_value(&v) {
    errors.push(e.with_field("email"));
}
```

### 6.2 Field-level custom message

`#[validate(min_length = 3, message = "too short")]` does **not** produce a `Rule::Described`. It applies `.with_message(...)` directly on the error:

```rust
if let Err(e) = ValueRule::MinLength(3).validate_value(&v) {
    errors.push(e.with_field("email").with_message("too short"));
}
```

`Described` is a runtime composition tool; macros apply the message at emit time without boxing.

For templated messages (`message = "got {value}, min {min}"`), macros emit placeholder parameters along with the message. The macro knows which placeholders are valid for each rule and can compile-time warn on unknown ones:

```rust
#[validate(min_length = 3, message = "got {value}, min {min}")]
email: String
```

Emits:

```rust
if let Err(e) = ValueRule::MinLength(3).validate_value(&v) {
    errors.push(
        e.with_field("email")
         .with_message("got {value}, min {min}")
         .with_params([
             ("min", serde_json::json!(3)),
             ("value", v.clone()),
         ])
    );
}
```

The macro emits `with_params` only for placeholders actually referenced in the template — no dead-weight params for unused placeholders.

### 6.3 Field-level cross-field predicates — inline comparison

A simple "this field relates to one other field" reference bypasses `Predicate` too:

```rust
#[validate(eq(field = "password"))]
password_confirm: String
```

Emits direct comparison (no `PredicateContext` construction, no allocation):

```rust
if self.password_confirm != self.password {
    errors.push(
        ValidationError::new(codes::EQ_FAILED)
            .with_field("password_confirm")
    );
}
```

### 6.4 Struct-level cross-field — full Rule tree

Complex conditions at struct level (combinators, multi-field) go through the real `Rule`/`Predicate`/`Logic` machinery with a one-time-built `PredicateContext`:

```rust
#[derive(Validator)]
#[validate(any(
    set(field = "email"),
    set(field = "phone"),
))]
struct Contact { email: Option<String>, phone: Option<String> }
```

Emits:

```rust
let ctx = PredicateContext::from_struct(self);
let rule = Rule::any([
    Rule::predicate(Predicate::Set(FieldPath::parse("email").unwrap())),
    Rule::predicate(Predicate::Set(FieldPath::parse("phone").unwrap())),
]);
if let Err(e) = rule.validate(&serde_json::json!(self), Some(&ctx), ExecutionMode::Full) {
    errors.push(e);
}
```

### 6.5 Decision table

| Attribute location | Emit form | Rationale |
|---|---|---|
| Field-level value rule | Direct `ValueRule::*` call | Fastest path; no dispatch |
| Field-level `message = "..."` | `.with_message(...)` on error | No need for `Described` allocation |
| Field-level simple cross-field (`eq(field = "X")`) | Inline `self.a == self.b` | Compile-time known; skip `Predicate` |
| Struct-level cross-field with combinators | Full `Rule` tree + `PredicateContext` | Combinators require the tree |

No user-visible behavior change — only cleaner expanded output and fewer runtime allocations.

---

## 7. Breaking Change Impact

### 7.1 In-workspace

| Crate | Files affected | Nature |
|---|---|---|
| `nebula-validator` src | ~10 | Core type redefinition |
| `nebula-validator` tests | ~15 | Pattern matches + fixtures |
| `nebula-validator/macros` | 4 | Emit code updated |
| `nebula-validator` docs/benches | ~5 | Doc examples + bench wire-compat |
| `nebula-schema` | 4 | Pattern matches + classification |
| `nebula-parameter` | ~5 | Re-exports + tests |
| `docs/superpowers/plans/*-schema-*` | 2 | Code examples refreshed |

Estimated ~200-400 pattern-match sites across workspace. Mechanical replacement with IDE assistance. No algorithmic rewrites.

### 7.2 Wire format

Breaking. New encoding adopted immediately, no dual-format `Deserialize`. Justification: no stored data exists, no external API commitments. Alpha stage allows clean cut.

### 7.3 Not broken

- `Validate<T>` trait and blanket `Validatable`/`ValidateExt` — untouched.
- Derive macro public attributes (`#[validate(min_length = 3)]`) — unchanged. Only emit-side changes.
- Error model (`ValidationError`, `ValidationErrors`, redaction, 80-byte bound) — untouched.
- Combinator types in `src/combinators/` — untouched in this refactor (candidate for Refactor 3's module consolidation, or left alone).

---

## 8. Out of Scope

Explicitly deferred to follow-up refactors, not forgotten:

### Refactor 2: Typed `Custom` via nebula-expression
- Replace `DeferredRule::Custom(String)` with `Custom(Expression)` where `Expression` is a typed AST from `nebula-expression`.
- Depends on `nebula-expression` API readiness.
- Separate PR.

### Refactor 3: Validated<T> first-class + observability + architecture docs
- Promote `Validated<T>` proof-carrying type to primary API.
- Add `#[tracing::instrument]` on engine hot paths.
- Write `crates/validator/ARCHITECTURE.md` + 3-5 ADRs.
- Separate PR.

### Semantic variant consolidation
- Merging `Min`/`Max`/`GreaterThan`/`LessThan` into `Numeric { bound: Bound<f64> }`.
- Merging `MinLength`/`MaxLength` into `StringLength(RangeInclusive)`, similar for `MinItems`/`MaxItems`.
- Wire-breaking. Deferred until async/server validation integration forces a wire review anyway.

### Combinator module consolidation
- Flattening 13 one-type files in `src/combinators/` into 3 grouped files.
- Cosmetic. Deferred.

---

## 9. Testing Strategy

### 9.1 Updates to existing tests
- `tests/integration/rule_roundtrip.rs` — golden files regenerated for new wire format.
- `tests/integration/combinator_interop.rs` — pattern matches updated.
- `tests/integration/deep_nesting.rs` — constructor API.
- `tests/contract/error_envelope_schema_test.rs` — verify `Described` message override.
- `tests/contract/typed_dynamic_equivalence_test.rs` — equivalence now holds via `Rule::validate` dispatch.
- UI tests for derive macro — unchanged (public attributes unchanged).

### 9.2 New tests
- `typed_narrowing_test.rs` — compile-fail: calling `validate_value` on `Predicate` must not compile.
- `described_decorator_test.rs` — `Described` wraps any `Rule` (including nested `Logic`, including nested `Described`) and overrides error message.
- `wire_format_compact_test.rs` — golden files for all variants in new externally-tagged compact form.
- `unknown_variant_error_test.rs` — malformed JSON produces `unknown rule "X"` error (verifies custom `Deserialize`).
- `message_template_test.rs` — named placeholders (`{value}`, `{min}`, `{max}`, `{pattern}`, `{expected}`, `{allowed}`) render correctly; unknown placeholder left as literal; `{{` / `}}` escape works; plain message (no `{`) takes zero-alloc hot path.

### 9.3 Benchmarks
- Existing `benches/rule_engine.rs` — wire-compat updated. Expected: comparable or faster (less tag parsing).
- New micro-benchmark: deserialize 1000-rule tree, compare old wire vs new wire.

---

## 10. Open Questions (non-blocking)

- **`Logic` tuple vs named fields.** Current proposal: `All(Vec<Rule>)` tuple. Trade-off: tuple more compact in wire (`{"all":[...]}`) but loses field name in Rust code. Alternative: `All { rules: Vec<Rule> }` named → wire becomes `{"all":{"rules":[...]}}`. **Leaning tuple** for compactness.
- **`Rule::Logic(Box<Logic>)` vs flattening Logic into Rule.** Could spread `All`/`Any`/`Not` back into `Rule` directly (Rule::All, Rule::Any, Rule::Not) — saves one wrapping layer. Trade-off: makes `Rule` bigger again, blurs the 4-class boundary. **Leaning keep as `Logic` sub-enum** for consistency.
- **`PredicateContext::from_json` shape.** Flat HashMap vs tree-walking Value. Depends on whether predicates can reference `/user/email` or only top-level fields. **Defer decision to implementation** — start flat, extend if needed.

---

## 11. Deliverables

- Updated `crates/validator` with new Rule structure, wire format, FieldPath integration.
- Updated `crates/validator/macros` emit code.
- Updated `crates/schema` and `crates/parameter` consumers.
- Updated `docs/superpowers/plans/*-schema-*.md` code examples.
- New/updated tests per §8.
- No `ARCHITECTURE.md` or ADRs in this PR (Refactor 3).

---

## 12. Non-Goals

This refactor does not:
- Add async validation.
- Integrate with `nebula-expression` for typed Custom rules.
- Introduce `Validated<T>` as primary API.
- Add tracing/observability.
- Consolidate numeric/length variants semantically.
- Write ADRs or `ARCHITECTURE.md`.

Each is deliberately deferred.
