# nebula-validator — Rule Type Split Refactor — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the flat 30-variant `Rule` enum in `crates/validator` with a typed sum-of-sums (`Rule::{Value, Predicate, Logic, Deferred, Described}`), adopt a compact externally-tagged wire format, integrate `FieldPath` into predicates, and add named-placeholder message templates.

**Architecture:** The outer `Rule` becomes a thin classifier with one `validate(input, ctx, mode)` dispatch method. Each semantic kind (value validation, context predicate, logical combinator, deferred runtime check) lives in its own inner enum with a single method that makes sense for it. A `Described(Box<Rule>, String)` decorator replaces per-variant `message: Option<String>` fields and works across combinators too. Wire format drops the redundant `"rule":` tag and uses tuple encoding for scalar variants, cutting compound-rule JSON by ~60%.

**Tech Stack:** Rust 2024, `serde`, `serde_json`, `smallvec`, `regex`, `trybuild`, `criterion`.

**Spec:** [`docs/superpowers/specs/2026-04-17-nebula-validator-rule-refactor-design.md`](../specs/2026-04-17-nebula-validator-rule-refactor-design.md)

**Stale spec notes (to ignore during execution):**
- The spec mentions `nebula-parameter` as a consumer — that crate was deleted in Phase 1 (commit `582ebc1e`). Skip any parameter-related work.
- The spec §6 describes macro emit strategy that is **mostly aspirational**. Current `crates/validator/macros` emits direct validator calls (`min_length(n).validate(x)`), not runtime `Rule::*` constructors. Macro impact for this refactor is near-zero; Task 16 covers the small surface that does need updating.

**Canonical pre-merge command (run after every phase):**
```bash
cargo +nightly fmt --all && \
cargo clippy --workspace -- -D warnings && \
cargo nextest run -p nebula-validator -p nebula-schema && \
cargo test -p nebula-validator --doc
```

---

## File Structure

### New files

| Path | Responsibility |
|---|---|
| `crates/validator/src/rule/value.rs` | `ValueRule` enum + `validate_value` + constructors |
| `crates/validator/src/rule/predicate.rs` | `Predicate` enum + `evaluate` + `field()` accessor |
| `crates/validator/src/rule/logic.rs` | `Logic` enum + `validate` + `walk` iterator |
| `crates/validator/src/rule/deferred.rs` | `DeferredRule` enum + `validate` |
| `crates/validator/src/rule/context.rs` | `PredicateContext` newtype over `HashMap<FieldPath, Value>` |
| `crates/validator/src/rule/deserialize.rs` | Manual `Deserialize` for outer `Rule` with friendly unknown-variant errors |
| `crates/validator/tests/integration/wire_format_compact.rs` | Golden tests for new wire shape |
| `crates/validator/tests/integration/described_decorator.rs` | `Described` semantics (any rule including nested) |
| `crates/validator/tests/integration/message_template.rs` | `{placeholder}` rendering on `ValidationError` |
| `crates/validator/tests/integration/unknown_variant_error.rs` | Friendly error for unknown rule key |
| `crates/validator/tests/ui/typed_narrowing.rs` | compile-fail: `validate_value` on `Predicate` |
| `crates/validator/tests/ui/typed_narrowing.stderr` | Expected compile error |

### Modified files

| Path | Nature |
|---|---|
| `crates/validator/src/foundation/field_path.rs` | Add `Serialize`/`Deserialize` impls |
| `crates/validator/src/foundation/error/validation_error.rs` | Add template rendering + update `Display` |
| `crates/validator/src/rule/mod.rs` | Replace flat enum with sum-of-sums outer `Rule` + dispatch |
| `crates/validator/src/rule/constructors.rs` | Rewrite constructors for new `Rule` + inner enums |
| `crates/validator/src/rule/helpers.rs` | Keep; minor adjustments to drop `override_message` |
| `crates/validator/src/rule/tests.rs` | Rewrite test bodies for new API |
| `crates/validator/src/engine.rs` | `validate_rules` dispatches via `Rule::validate` |
| `crates/validator/src/lib.rs` | Re-export new types (`ValueRule`, `Predicate`, `Logic`, `DeferredRule`, `PredicateContext`) |
| `crates/validator/src/prelude.rs` | Add new type re-exports |
| `crates/validator/tests/integration/rule_roundtrip.rs` | Port assertions to new wire format |
| `crates/validator/tests/integration/main.rs` | Register new integration modules |
| `crates/validator/benches/rule_engine.rs` | Update constructor calls |
| `crates/schema/src/lint.rs` | Update ~40 pattern matches to new shape |
| `crates/schema/src/field.rs` | Update ~10 builder `Rule::*` construction sites |
| `CHANGELOG.md` | Document breaking change + wire format cut |

### Deleted files

| Path | Why |
|---|---|
| `crates/validator/src/rule/validate.rs` | Logic moves into `rule/value.rs` + `rule/logic.rs` |
| `crates/validator/src/rule/evaluate.rs` | Logic moves into `rule/predicate.rs` + `rule/logic.rs` |
| `crates/validator/src/rule/classify.rs` | Replaced by `Rule::kind()` + `matches!` on outer variants |

---

## Task 1: `FieldPath` Serde integration

**Files:**
- Modify: `crates/validator/src/foundation/field_path.rs`

**Rationale:** `Predicate` variants store `FieldPath` instead of `String`. Need serde support so the new `Predicate` variants can roundtrip through JSON without custom per-variant `Deserialize` logic.

- [ ] **Step 1: Read current FieldPath impl structure**

Run: `grep -n "impl FieldPath\|pub fn\|pub(crate)" crates/validator/src/foundation/field_path.rs | head -20`

Confirm `as_str()` and `into_inner()` exist for write access, and `FieldPath::parse(&str) -> Option<Self>` for read.

- [ ] **Step 2: Write failing serde roundtrip test**

Append to `crates/validator/src/foundation/field_path.rs` (inside the existing `#[cfg(test)] mod tests`):

```rust
#[test]
fn serialize_is_plain_string() {
    let p = FieldPath::parse("user.email").unwrap();
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json, serde_json::json!("/user/email"));
}

#[test]
fn deserialize_from_string() {
    let p: FieldPath = serde_json::from_value(serde_json::json!("/user/email")).unwrap();
    assert_eq!(p.as_str(), "/user/email");
}

#[test]
fn deserialize_rejects_empty() {
    let result: Result<FieldPath, _> = serde_json::from_value(serde_json::json!(""));
    assert!(result.is_err());
}

#[test]
fn roundtrip_stable_across_formats() {
    let p = FieldPath::parse("items[0].city").unwrap();
    let encoded = serde_json::to_value(&p).unwrap();
    let decoded: FieldPath = serde_json::from_value(encoded).unwrap();
    assert_eq!(p, decoded);
}
```

- [ ] **Step 3: Run to verify tests fail (no Serialize impl yet)**

Run: `cargo test -p nebula-validator --lib foundation::field_path::tests::serialize -- --nocapture`
Expected: compile error "`FieldPath` doesn't implement `Serialize`".

- [ ] **Step 4: Implement Serialize and Deserialize**

Add these impls to `crates/validator/src/foundation/field_path.rs` (just after the `impl fmt::Display for FieldPath` block — find it via grep or scroll):

```rust
impl serde::Serialize for FieldPath {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for FieldPath {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        FieldPath::parse(raw.as_ref())
            .ok_or_else(|| serde::de::Error::custom(format!("invalid field path: {raw:?}")))
    }
}
```

- [ ] **Step 5: Run tests to verify pass**

Run: `cargo test -p nebula-validator --lib foundation::field_path`
Expected: all tests pass including new ones.

- [ ] **Step 6: Commit**

```bash
git add crates/validator/src/foundation/field_path.rs
git commit -m "$(cat <<'EOF'
feat(validator): add Serialize/Deserialize for FieldPath

Wire form is the inner JSON Pointer string. Prereq for Predicate
variants to carry FieldPath instead of raw String.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Message template rendering on `ValidationError`

**Files:**
- Modify: `crates/validator/src/foundation/error/validation_error.rs`

**Rationale:** `Described` messages can contain `{name}` placeholders that substitute from the error's `params`. Rendering happens at `Display` time with zero allocation for plain messages.

- [ ] **Step 1: Read current Display impl**

Open `crates/validator/src/foundation/error/validation_error.rs` and locate the `impl fmt::Display for ValidationError` block (around line 357). Note that it already writes `self.message` directly.

- [ ] **Step 2: Write failing template tests**

Append to the existing `#[cfg(test)] mod tests` in `crates/validator/src/foundation/error/mod.rs`:

```rust
#[test]
fn template_substitutes_named_placeholder() {
    let err = ValidationError::new("min_length", "got {value}, expected at least {min} chars")
        .with_param("min", "3")
        .with_param("value", "\"hi\"");
    let rendered = format!("{err}");
    assert!(rendered.contains("got \"hi\", expected at least 3 chars"));
}

#[test]
fn template_leaves_unknown_placeholder_literal() {
    let err = ValidationError::new("test", "value is {unknown}");
    let rendered = format!("{err}");
    assert!(rendered.contains("value is {unknown}"));
}

#[test]
fn template_escape_double_brace() {
    let err = ValidationError::new("test", "literal {{ and {{value}}");
    let rendered = format!("{err}");
    assert!(rendered.contains("literal { and {value}"));
}

#[test]
fn plain_message_bypasses_template_path() {
    let err = ValidationError::new("test", "no placeholders here");
    let rendered = format!("{err}");
    assert!(rendered.contains("no placeholders here"));
}
```

- [ ] **Step 3: Run to verify first test fails**

Run: `cargo test -p nebula-validator --lib foundation::error::tests::template_substitutes -- --nocapture`
Expected: FAIL — rendered output contains `{value}` literal, not substituted.

- [ ] **Step 4: Add render_message helper and update Display**

In `crates/validator/src/foundation/error/validation_error.rs`, add this helper below the `ValidationError` struct (above `impl fmt::Display`):

```rust
/// Renders a message template by substituting `{name}` placeholders with
/// the matching entry from `params`. `{{` and `}}` are literal braces.
/// Unknown `{name}` is left as-is. Zero allocation when the template has
/// no `{` at all.
fn render_template<'a>(template: &'a str, params: &[(Cow<'static, str>, Cow<'static, str>)]) -> Cow<'a, str> {
    if !template.contains('{') {
        return Cow::Borrowed(template);
    }

    let mut out = String::with_capacity(template.len());
    let mut chars = template.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        if c == '{' {
            if matches!(chars.peek(), Some((_, '{'))) {
                out.push('{');
                chars.next();
                continue;
            }
            let mut name = String::new();
            let mut closed = false;
            while let Some((_, nc)) = chars.next() {
                if nc == '}' {
                    closed = true;
                    break;
                }
                name.push(nc);
            }
            if !closed {
                out.push('{');
                out.push_str(&name);
                continue;
            }
            match params.iter().find(|(k, _)| k.as_ref() == name) {
                Some((_, v)) => out.push_str(v.as_ref()),
                None => {
                    out.push('{');
                    out.push_str(&name);
                    out.push('}');
                },
            }
        } else if c == '}' {
            if matches!(chars.peek(), Some((_, '}'))) {
                out.push('}');
                chars.next();
            } else {
                out.push('}');
            }
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}
```

Then modify the `impl fmt::Display for ValidationError` block — replace the message write line. Current:

```rust
if let Some(field) = &self.field {
    write!(f, "[{}] {}: {}", field, self.code, self.message)?;
} else {
    write!(f, "{}: {}", self.code, self.message)?;
}
```

New:

```rust
let params = self.params();
let rendered = render_template(self.message.as_ref(), params);
if let Some(field) = &self.field {
    write!(f, "[{}] {}: {}", field, self.code, rendered)?;
} else {
    write!(f, "{}: {}", self.code, rendered)?;
}
```

- [ ] **Step 5: Run tests to verify pass**

Run: `cargo test -p nebula-validator --lib foundation::error`
Expected: all tests (old + new) pass.

- [ ] **Step 6: Commit**

```bash
git add crates/validator/src/foundation/error/validation_error.rs crates/validator/src/foundation/error/mod.rs
git commit -m "$(cat <<'EOF'
feat(validator): named-placeholder templates in ValidationError::Display

Adds render_template helper that substitutes {name} placeholders with
matching entries from ValidationError params, with {{/}} escape and
literal passthrough for unknown placeholders. Zero-alloc hot path for
messages without {.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `ValueRule` sub-enum

**Files:**
- Create: `crates/validator/src/rule/value.rs`

**Rationale:** Extracts the 12 value-validation variants into their own enum with a single `validate_value` method. Silent-pass on JSON type mismatch is preserved as documented ergonomic.

- [ ] **Step 1: Write the new module with failing tests inline**

Create `crates/validator/src/rule/value.rs`:

```rust
//! Value-validation rules — operate on a single JSON value, no context.
//!
//! Silent-pass on JSON type mismatch (e.g. `MinLength` on a number
//! returns `Ok`) is preserved as documented ergonomic. Cross-kind
//! silent-pass (predicate returning `Ok` from `validate_value`) is
//! eliminated by the type split.

use serde::{Deserialize, Serialize};

use super::helpers::{compile_regex, format_json_number, json_number_cmp};
use crate::{
    foundation::{Validate, ValidationError},
    validators::{
        content::{EMAIL_PATTERN, URL_PATTERN},
        max_length, max_size, min_length, min_size,
    },
};

/// Value-validation rule. Takes a JSON value, returns `Ok` or a
/// `ValidationError` whose `params` include rule-specific placeholders
/// (`{min}`, `{max}`, `{pattern}`, `{allowed}`) for template rendering.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ValueRule {
    /// String must be at least `n` characters.
    MinLength(usize),
    /// String must be at most `n` characters.
    MaxLength(usize),
    /// String must match the regex.
    Pattern(String),
    /// Number must be >= bound.
    Min(serde_json::Number),
    /// Number must be <= bound.
    Max(serde_json::Number),
    /// Number must be strictly > bound.
    GreaterThan(serde_json::Number),
    /// Number must be strictly < bound.
    LessThan(serde_json::Number),
    /// Value must be one of the given alternatives (type-matched).
    OneOf(Vec<serde_json::Value>),
    /// Collection must contain at least `n` items.
    MinItems(usize),
    /// Collection must contain at most `n` items.
    MaxItems(usize),
    /// Value must be a valid email address.
    Email,
    /// Value must be a valid URL.
    Url,
}

impl ValueRule {
    /// Validates a JSON value against this rule. Returns `Ok(())` silently
    /// when the JSON type doesn't match the rule's expected type.
    ///
    /// Errors carry rule-specific `params` for message-template rendering:
    /// `{min}`, `{max}`, `{pattern}`, `{allowed}`, plus always `{value}`.
    pub fn validate_value(&self, value: &serde_json::Value) -> Result<(), ValidationError> {
        match self {
            Self::MinLength(n) => {
                if let Some(s) = value.as_str() {
                    min_length(*n)
                        .validate(s)
                        .map_err(|e| e.with_param("min", n.to_string()).with_param("value", format!("{value}")))?;
                }
                Ok(())
            },
            Self::MaxLength(n) => {
                if let Some(s) = value.as_str() {
                    max_length(*n)
                        .validate(s)
                        .map_err(|e| e.with_param("max", n.to_string()).with_param("value", format!("{value}")))?;
                }
                Ok(())
            },
            Self::Pattern(p) => {
                if let Some(s) = value.as_str() {
                    let re = compile_regex(p)?;
                    if !re.is_match(s) {
                        return Err(ValidationError::invalid_format("", "regex")
                            .with_param("pattern", p.clone())
                            .with_param("value", format!("{value}")));
                    }
                }
                Ok(())
            },
            Self::Min(bound) => {
                if let Some(ord) = json_number_cmp(value, bound)
                    && ord.is_lt()
                {
                    return Err(ValidationError::new(
                        "min",
                        "Value must be at least {min}",
                    )
                    .with_param("min", format_json_number(bound))
                    .with_param("value", format!("{value}")));
                }
                Ok(())
            },
            Self::Max(bound) => {
                if let Some(ord) = json_number_cmp(value, bound)
                    && ord.is_gt()
                {
                    return Err(ValidationError::new(
                        "max",
                        "Value must be at most {max}",
                    )
                    .with_param("max", format_json_number(bound))
                    .with_param("value", format!("{value}")));
                }
                Ok(())
            },
            Self::GreaterThan(bound) => {
                if let Some(ord) = json_number_cmp(value, bound)
                    && !ord.is_gt()
                {
                    return Err(ValidationError::new(
                        "greater_than",
                        "Value must be greater than {min}",
                    )
                    .with_param("min", format_json_number(bound))
                    .with_param("value", format!("{value}")));
                }
                Ok(())
            },
            Self::LessThan(bound) => {
                if let Some(ord) = json_number_cmp(value, bound)
                    && !ord.is_lt()
                {
                    return Err(ValidationError::new(
                        "less_than",
                        "Value must be less than {max}",
                    )
                    .with_param("max", format_json_number(bound))
                    .with_param("value", format!("{value}")));
                }
                Ok(())
            },
            Self::OneOf(values) => {
                if values.is_empty() {
                    return Ok(());
                }
                let has_same_type = values
                    .iter()
                    .any(|v| std::mem::discriminant(v) == std::mem::discriminant(value));
                if !has_same_type {
                    return Ok(());
                }
                if !values.contains(value) {
                    let allowed = values
                        .iter()
                        .map(|v| format!("{v}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(ValidationError::new(
                        "one_of",
                        "must be one of {allowed}",
                    )
                    .with_param("allowed", allowed)
                    .with_param("value", format!("{value}")));
                }
                Ok(())
            },
            Self::MinItems(n) => {
                if let Some(items) = value.as_array() {
                    min_size::<serde_json::Value>(*n)
                        .validate(items.as_slice())
                        .map_err(|e| e.with_param("min", n.to_string()))?;
                }
                Ok(())
            },
            Self::MaxItems(n) => {
                if let Some(items) = value.as_array() {
                    max_size::<serde_json::Value>(*n)
                        .validate(items.as_slice())
                        .map_err(|e| e.with_param("max", n.to_string()))?;
                }
                Ok(())
            },
            Self::Email => {
                if let Some(s) = value.as_str() {
                    static EMAIL_RE: std::sync::LazyLock<regex::Regex> =
                        std::sync::LazyLock::new(|| regex::Regex::new(EMAIL_PATTERN).expect("email regex"));
                    if !EMAIL_RE.is_match(s) {
                        return Err(ValidationError::invalid_format("", "email")
                            .with_param("value", format!("{value}")));
                    }
                }
                Ok(())
            },
            Self::Url => {
                if let Some(s) = value.as_str() {
                    static URL_RE: std::sync::LazyLock<regex::Regex> =
                        std::sync::LazyLock::new(|| regex::Regex::new(URL_PATTERN).expect("url regex"));
                    if !URL_RE.is_match(s) {
                        return Err(ValidationError::invalid_format("", "url")
                            .with_param("value", format!("{value}")));
                    }
                }
                Ok(())
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn min_length_ok_and_err() {
        assert!(ValueRule::MinLength(3).validate_value(&json!("alice")).is_ok());
        assert!(ValueRule::MinLength(3).validate_value(&json!("ab")).is_err());
    }

    #[test]
    fn min_length_silent_pass_on_non_string() {
        assert!(ValueRule::MinLength(3).validate_value(&json!(42)).is_ok());
    }

    #[test]
    fn min_rejects_below_bound() {
        let rule = ValueRule::Min(serde_json::Number::from(10));
        assert!(rule.validate_value(&json!(5)).is_err());
        assert!(rule.validate_value(&json!(15)).is_ok());
    }

    #[test]
    fn one_of_empty_passes() {
        assert!(ValueRule::OneOf(vec![]).validate_value(&json!("x")).is_ok());
    }

    #[test]
    fn wire_form_scalar_is_tuple() {
        let r = ValueRule::MinLength(3);
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j, json!({"min_length": 3}));
    }

    #[test]
    fn wire_form_unit_is_bare_string() {
        let r = ValueRule::Email;
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j, json!("email"));
    }

    #[test]
    fn error_injects_params_for_template_rendering() {
        let err = ValueRule::MinLength(3).validate_value(&json!("hi")).unwrap_err();
        assert_eq!(err.param("min"), Some("3"));
    }
}
```

- [ ] **Step 2: Register the module and run tests**

Not yet registered — `rule/mod.rs` still has the flat `Rule`. Leave it floating for now; Task 9 wires it up. To test in isolation, temporarily add `pub mod value;` to `crates/validator/src/rule/mod.rs` (above `use serde::...`) for this step only.

Run: `cargo test -p nebula-validator --lib rule::value`
Expected: all 7 tests pass.

- [ ] **Step 3: Revert the temporary `pub mod value;` line**

The outer `Rule` still occupies the namespace; we'll wire `value` in properly during Task 9. For now remove that line so the module lives on disk but isn't reachable.

- [ ] **Step 4: Commit**

```bash
git add crates/validator/src/rule/value.rs
git commit -m "$(cat <<'EOF'
feat(validator): add ValueRule sub-enum (not yet wired)

Defines the value-validation kind as a standalone enum with a single
validate_value method. Errors inject rule-specific params (min, max,
pattern, allowed, value) for template rendering.

Wire form is externally-tagged, tuple-compact. Not yet re-exported
from the crate — wiring happens in the outer Rule rewrite (Task 9).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `Predicate` sub-enum

**Files:**
- Create: `crates/validator/src/rule/predicate.rs`

**Rationale:** 13 context-predicate variants in one typed enum. `FieldPath` instead of raw `String` means pointer-path validation is enforced at construction.

- [ ] **Step 1: Create the module**

Create `crates/validator/src/rule/predicate.rs`:

```rust
//! Context predicates — boolean tests against sibling fields.
//!
//! Each variant holds a `FieldPath` to the sibling. `evaluate` takes a
//! `PredicateContext` and returns `bool`. Missing-field semantics are
//! documented per-variant.

use serde::{Deserialize, Serialize};

use super::{context::PredicateContext, helpers::compile_regex};
use crate::foundation::FieldPath;

/// Context predicate. Always evaluates to `bool` — never errors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Predicate {
    /// `field == value`
    Eq(FieldPath, serde_json::Value),
    /// `field != value`
    Ne(FieldPath, serde_json::Value),
    /// `field > value` (numeric).
    Gt(FieldPath, serde_json::Number),
    /// `field >= value` (numeric).
    Gte(FieldPath, serde_json::Number),
    /// `field < value` (numeric).
    Lt(FieldPath, serde_json::Number),
    /// `field <= value` (numeric).
    Lte(FieldPath, serde_json::Number),
    /// `field == true`
    IsTrue(FieldPath),
    /// `field == false`
    IsFalse(FieldPath),
    /// Field has a non-null, non-empty value.
    Set(FieldPath),
    /// Field is null, absent, or empty string/array.
    Empty(FieldPath),
    /// String or array field contains the given value.
    Contains(FieldPath, serde_json::Value),
    /// String field matches the regular expression.
    Matches(FieldPath, String),
    /// Field value is a member of the given set.
    In(FieldPath, Vec<serde_json::Value>),
}

impl Predicate {
    /// Returns the `FieldPath` this predicate references.
    pub fn field(&self) -> &FieldPath {
        match self {
            Self::Eq(f, _)
            | Self::Ne(f, _)
            | Self::Gt(f, _)
            | Self::Gte(f, _)
            | Self::Lt(f, _)
            | Self::Lte(f, _)
            | Self::IsTrue(f)
            | Self::IsFalse(f)
            | Self::Set(f)
            | Self::Empty(f)
            | Self::Contains(f, _)
            | Self::Matches(f, _)
            | Self::In(f, _) => f,
        }
    }

    /// Evaluates the predicate against context. Missing field → per-variant
    /// defaults: `Eq/Gt/Gte/Lt/Lte/IsTrue/IsFalse/Set/Contains/Matches/In` → false;
    /// `Ne` → true; `Empty` → true.
    #[must_use]
    pub fn evaluate(&self, ctx: &PredicateContext) -> bool {
        use super::helpers::{cmp_number_predicate, json_number_cmp};

        match self {
            Self::Eq(f, v) => ctx.get(f).is_some_and(|x| x == v),
            Self::Ne(f, v) => ctx.get(f).is_none_or(|x| x != v),
            Self::Gt(f, v) => cmp_number_predicate(ctx.get(f), v, |o| o.is_gt()),
            Self::Gte(f, v) => cmp_number_predicate(ctx.get(f), v, |o| o.is_ge()),
            Self::Lt(f, v) => cmp_number_predicate(ctx.get(f), v, |o| o.is_lt()),
            Self::Lte(f, v) => cmp_number_predicate(ctx.get(f), v, |o| o.is_le()),
            Self::IsTrue(f) => ctx.get(f).and_then(serde_json::Value::as_bool) == Some(true),
            Self::IsFalse(f) => ctx.get(f).and_then(serde_json::Value::as_bool) == Some(false),
            Self::Set(f) => ctx.get(f).is_some_and(|v| {
                !v.is_null()
                    && match v {
                        serde_json::Value::String(s) => !s.is_empty(),
                        serde_json::Value::Array(a) => !a.is_empty(),
                        _ => true,
                    }
            }),
            Self::Empty(f) => ctx.get(f).is_none_or(|v| {
                v.is_null()
                    || match v {
                        serde_json::Value::String(s) => s.is_empty(),
                        serde_json::Value::Array(a) => a.is_empty(),
                        _ => false,
                    }
            }),
            Self::Contains(f, v) => ctx.get(f).is_some_and(|x| match x {
                serde_json::Value::String(s) => v.as_str().is_some_and(|needle| s.contains(needle)),
                serde_json::Value::Array(items) => items.contains(v),
                _ => false,
            }),
            Self::Matches(f, pat) => {
                debug_assert!(
                    regex::Regex::new(pat).is_ok(),
                    "Predicate::Matches: invalid regex {pat:?}"
                );
                ctx.get(f)
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|s| compile_regex(pat).is_ok_and(|re| re.is_match(s)))
            },
            Self::In(f, allowed) => ctx.get(f).is_some_and(|x| {
                let _ = json_number_cmp; // keep import alive if module shape changes
                allowed.contains(x)
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ctx_from(obj: serde_json::Value) -> PredicateContext {
        PredicateContext::from_json(&obj)
    }

    #[test]
    fn eq_matches_existing_field() {
        let p = Predicate::Eq(FieldPath::parse("status").unwrap(), json!("active"));
        assert!(p.evaluate(&ctx_from(json!({"status": "active"}))));
    }

    #[test]
    fn eq_on_missing_field_is_false() {
        let p = Predicate::Eq(FieldPath::parse("status").unwrap(), json!("active"));
        assert!(!p.evaluate(&ctx_from(json!({}))));
    }

    #[test]
    fn ne_on_missing_field_is_true() {
        let p = Predicate::Ne(FieldPath::parse("status").unwrap(), json!("active"));
        assert!(p.evaluate(&ctx_from(json!({}))));
    }

    #[test]
    fn wire_form_tuple_is_compact() {
        let p = Predicate::Eq(FieldPath::parse("status").unwrap(), json!("active"));
        let j = serde_json::to_value(&p).unwrap();
        assert_eq!(j, json!({"eq": ["/status", "active"]}));
    }

    #[test]
    fn wire_form_roundtrip() {
        let p = Predicate::In(FieldPath::parse("method").unwrap(), vec![json!("POST"), json!("PUT")]);
        let back: Predicate = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }
}
```

- [ ] **Step 2: Temporarily wire and test**

Temporarily add `pub mod predicate;` and `pub mod context;` to `crates/validator/src/rule/mod.rs` (but see next Task for `context`). For this Task, also write a **placeholder** `context.rs` that we'll replace in Task 7:

Create `crates/validator/src/rule/context.rs` (placeholder):

```rust
//! Placeholder — replaced by full impl in Task 7.

use std::collections::HashMap;

use crate::foundation::FieldPath;

pub struct PredicateContext {
    fields: HashMap<FieldPath, serde_json::Value>,
}

impl PredicateContext {
    pub fn get(&self, p: &FieldPath) -> Option<&serde_json::Value> {
        self.fields.get(p)
    }

    pub fn from_json(obj: &serde_json::Value) -> Self {
        let mut fields = HashMap::new();
        if let Some(m) = obj.as_object() {
            for (k, v) in m {
                if let Some(path) = FieldPath::parse(k) {
                    fields.insert(path, v.clone());
                }
            }
        }
        Self { fields }
    }
}
```

Add `pub mod context;` temporarily in `rule/mod.rs`, then:

Run: `cargo test -p nebula-validator --lib rule::predicate`
Expected: all 5 Predicate tests pass.

- [ ] **Step 3: Remove temporary `pub mod` lines from `rule/mod.rs`**

Keep the files on disk; they'll be wired permanently in Task 9.

- [ ] **Step 4: Commit**

```bash
git add crates/validator/src/rule/predicate.rs crates/validator/src/rule/context.rs
git commit -m "$(cat <<'EOF'
feat(validator): add Predicate sub-enum + PredicateContext placeholder

Predicate holds FieldPath (not raw String), exposes field() accessor,
evaluate takes a typed PredicateContext. Missing-field semantics
match current Rule::evaluate. Wire form is tuple-compact
({"eq": ["/path", value]}).

Context module is a placeholder — full impl comes in Task 7.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `Logic` sub-enum

**Files:**
- Create: `crates/validator/src/rule/logic.rs`

**Rationale:** Combinators recurse into the outer `Rule`, which doesn't exist yet in new form. Define the enum structurally; the `validate` method is a stub that calls into `Rule::validate` (resolved by module order once Rule is rewritten in Task 9).

- [ ] **Step 1: Create the module**

Create `crates/validator/src/rule/logic.rs`:

```rust
//! Logical combinators — `All`, `Any`, `Not` over nested `Rule`.

use serde::{Deserialize, Serialize};

use super::Rule;
use crate::{
    engine::ExecutionMode,
    foundation::ValidationError,
    rule::context::PredicateContext,
};

/// Logical combinator. Children are `Rule`, so combinators can mix kinds
/// (e.g. an `All` containing both `ValueRule::Email` and a `Predicate`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Logic {
    /// All children must pass.
    All(Vec<Rule>),
    /// At least one child must pass.
    Any(Vec<Rule>),
    /// Negates the child.
    Not(Rule),
}

impl Logic {
    /// Dispatches the combinator. Errors from children are collected into
    /// the parent's `nested` chain.
    pub fn validate(
        &self,
        input: &serde_json::Value,
        ctx: Option<&PredicateContext>,
        mode: ExecutionMode,
    ) -> Result<(), ValidationError> {
        match self {
            Self::All(rules) => {
                let mut errs = Vec::new();
                for r in rules {
                    if let Err(e) = r.validate(input, ctx, mode) {
                        errs.push(e);
                    }
                }
                if errs.is_empty() {
                    Ok(())
                } else if errs.len() == 1 {
                    Err(errs.into_iter().next().unwrap())
                } else {
                    let n = errs.len();
                    Err(ValidationError::new("all_failed", format!("{n} of the rules failed"))
                        .with_nested(errs))
                }
            },
            Self::Any(rules) => {
                if rules.is_empty() {
                    return Ok(());
                }
                let mut errs = Vec::new();
                for r in rules {
                    match r.validate(input, ctx, mode) {
                        Ok(()) => return Ok(()),
                        Err(e) => errs.push(e),
                    }
                }
                let n = errs.len();
                Err(ValidationError::new("any_failed", format!("All {n} alternatives failed"))
                    .with_nested(errs))
            },
            Self::Not(inner) => match inner.validate(input, ctx, mode) {
                Ok(()) => Err(ValidationError::new("not_failed", "negated rule passed")),
                Err(_) => Ok(()),
            },
        }
    }

    /// Iterates all direct child rules (shallow — does not recurse).
    pub fn children(&self) -> &[Rule] {
        match self {
            Self::All(v) | Self::Any(v) => v.as_slice(),
            Self::Not(inner) => std::slice::from_ref(inner),
        }
    }
}
```

- [ ] **Step 2: Commit (no tests yet — they depend on `Rule::validate` from Task 9)**

The module will fail to compile on its own because `super::Rule` still has the old shape. Don't wire it into `mod.rs` yet. It's file-on-disk only.

Run (sanity check that the rest of the crate still compiles without this file being reachable):
```bash
cargo check -p nebula-validator
```
Expected: PASS (logic.rs is not pulled in via `mod` declaration anywhere).

```bash
git add crates/validator/src/rule/logic.rs
git commit -m "$(cat <<'EOF'
feat(validator): add Logic sub-enum (unwired pending Rule rewrite)

All/Any/Not combinators recursing into Rule. Children mix kinds,
matching current semantics. validate() dispatches via Rule::validate
which is introduced in Task 9 — module stays off the module tree
until then.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: `DeferredRule` sub-enum

**Files:**
- Create: `crates/validator/src/rule/deferred.rs`

**Rationale:** Two variants: `Custom(String)` (expression string) and `UniqueBy(FieldPath)` (array-item uniqueness). Both skipped at `ExecutionMode::StaticOnly`.

- [ ] **Step 1: Create the module**

Create `crates/validator/src/rule/deferred.rs`:

```rust
//! Deferred rules — require runtime context beyond the value + predicate
//! map. Skipped at schema-validation time.

use serde::{Deserialize, Serialize};

use crate::{
    foundation::{FieldPath, ValidationError},
    rule::context::PredicateContext,
};

/// Rule requiring runtime evaluation beyond static context.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeferredRule {
    /// Custom expression string. Typing via `nebula-expression` is Refactor 2.
    Custom(String),
    /// Each array item must have a unique value at the given sub-path.
    UniqueBy(FieldPath),
}

impl DeferredRule {
    /// Validates deferred. Without a ctx bridge to the runtime evaluator
    /// these rules return `Ok(())` — they'll be picked up by the workflow
    /// engine when it has a real context.
    pub fn validate(
        &self,
        _input: &serde_json::Value,
        _ctx: Option<&PredicateContext>,
    ) -> Result<(), ValidationError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn custom_wire_form() {
        let r = DeferredRule::Custom("check()".into());
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j, json!({"custom": "check()"}));
    }

    #[test]
    fn unique_by_roundtrip() {
        let r = DeferredRule::UniqueBy(FieldPath::parse("name").unwrap());
        let back: DeferredRule = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }
}
```

- [ ] **Step 2: Temporarily wire and test**

Add `pub mod deferred;` (and `pub mod context;` if not already from Task 4 commit) temporarily to `crates/validator/src/rule/mod.rs`.

Run: `cargo test -p nebula-validator --lib rule::deferred`
Expected: both tests pass.

- [ ] **Step 3: Remove temporary `pub mod deferred;`**

- [ ] **Step 4: Commit**

```bash
git add crates/validator/src/rule/deferred.rs
git commit -m "$(cat <<'EOF'
feat(validator): add DeferredRule sub-enum (unwired)

Custom(String) holds expression string (typing deferred to Refactor 2);
UniqueBy(FieldPath) carries the sub-path key typed. validate() is a no-op
placeholder — workflow engine owns real deferred evaluation.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Full `PredicateContext` impl

**Files:**
- Modify: `crates/validator/src/rule/context.rs` (replacing placeholder from Task 4)

**Rationale:** Replace the placeholder with a proper newtype that hides the internal storage and supports top-level + nested-path lookup.

- [ ] **Step 1: Write failing test file**

Replace `crates/validator/src/rule/context.rs` entirely:

```rust
//! `PredicateContext` — typed context for [`Predicate::evaluate`].
//!
//! Wraps a `FieldPath`-keyed map of values. Callers build it from JSON
//! once per evaluation round.

use std::collections::HashMap;

use crate::foundation::FieldPath;

/// Typed field context for predicate evaluation. Construct via
/// `PredicateContext::from_json` or `PredicateContext::from_fields`.
#[derive(Debug, Clone, Default)]
pub struct PredicateContext {
    fields: HashMap<FieldPath, serde_json::Value>,
}

impl PredicateContext {
    /// Empty context — predicates see no fields.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from an iterator of `(FieldPath, Value)` pairs.
    pub fn from_fields<I: IntoIterator<Item = (FieldPath, serde_json::Value)>>(iter: I) -> Self {
        Self {
            fields: iter.into_iter().collect(),
        }
    }

    /// Flatten a JSON object into a FieldPath-keyed map.
    ///
    /// Top-level keys map to `/key` pointers. Nested objects get recursive
    /// `/a/b` keys. Arrays are stored as-is under their parent path
    /// (callers can extend to array-index paths if needed).
    pub fn from_json(obj: &serde_json::Value) -> Self {
        let mut fields = HashMap::new();
        if let Some(m) = obj.as_object() {
            collect_paths(&mut fields, "", m);
        }
        Self { fields }
    }

    /// Fetch a value by path. Returns `None` if the field is absent.
    pub fn get(&self, path: &FieldPath) -> Option<&serde_json::Value> {
        self.fields.get(path)
    }

    /// Number of stored field bindings.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// True if no fields are bound.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

fn collect_paths(
    out: &mut HashMap<FieldPath, serde_json::Value>,
    prefix: &str,
    obj: &serde_json::Map<String, serde_json::Value>,
) {
    for (k, v) in obj {
        let full = if prefix.is_empty() {
            format!("/{k}")
        } else {
            format!("{prefix}/{k}")
        };
        if let Some(path) = FieldPath::parse(&full) {
            out.insert(path, v.clone());
        }
        if let Some(nested) = v.as_object() {
            collect_paths(out, &full, nested);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn top_level_keys_indexed_by_pointer() {
        let ctx = PredicateContext::from_json(&json!({"name": "alice", "age": 30}));
        let name = ctx.get(&FieldPath::parse("name").unwrap());
        assert_eq!(name, Some(&json!("alice")));
    }

    #[test]
    fn nested_keys_indexed_recursively() {
        let ctx = PredicateContext::from_json(&json!({"user": {"email": "x@y.z"}}));
        let email = ctx.get(&FieldPath::parse("/user/email").unwrap());
        assert_eq!(email, Some(&json!("x@y.z")));
    }

    #[test]
    fn missing_field_returns_none() {
        let ctx = PredicateContext::from_json(&json!({}));
        assert!(ctx.get(&FieldPath::parse("absent").unwrap()).is_none());
    }

    #[test]
    fn empty_context_is_empty() {
        let ctx = PredicateContext::new();
        assert!(ctx.is_empty());
    }
}
```

- [ ] **Step 2: Temporarily wire and run tests**

Add `pub mod context;` to `crates/validator/src/rule/mod.rs` temporarily.

Run: `cargo test -p nebula-validator --lib rule::context`
Expected: all 4 tests pass.

- [ ] **Step 3: Remove temporary wiring**

- [ ] **Step 4: Commit**

```bash
git add crates/validator/src/rule/context.rs
git commit -m "$(cat <<'EOF'
feat(validator): full PredicateContext impl with nested-path flattening

Typed newtype over HashMap<FieldPath, Value>. from_json flattens a
JSON object into /path-keyed entries, handling nested objects.
Arrays stored under their parent path for now — array-index paths
are a future extension.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Manual `Deserialize` module for outer `Rule`

**Files:**
- Create: `crates/validator/src/rule/deserialize.rs`

**Rationale:** Default `#[serde(untagged)]` gives unhelpful "data did not match any variant" errors. A manual `Visitor` reads the first map key (or bare string for unit variants) and dispatches to the matching inner enum, with a known-rules list for suggestions.

- [ ] **Step 1: Create the file with full impl (test comes in Task 9 integration)**

Create `crates/validator/src/rule/deserialize.rs`:

```rust
//! Manual `Deserialize` for [`Rule`] — dispatches the first JSON key to
//! the matching sub-enum variant, producing friendly errors for unknown
//! keys.

use serde::de::{self, Deserializer, MapAccess, Visitor};

use super::{DeferredRule, Logic, Predicate, Rule, ValueRule};
use crate::foundation::FieldPath;

const VALUE_RULES: &[&str] = &[
    "min_length",
    "max_length",
    "pattern",
    "min",
    "max",
    "greater_than",
    "less_than",
    "one_of",
    "min_items",
    "max_items",
    "email",
    "url",
];

const PREDICATES: &[&str] = &[
    "eq", "ne", "gt", "gte", "lt", "lte", "is_true", "is_false", "set", "empty", "contains",
    "matches", "in",
];

const LOGIC: &[&str] = &["all", "any", "not"];
const DEFERRED: &[&str] = &["custom", "unique_by"];
const DESCRIBED: &str = "described";

fn all_known() -> String {
    let mut out = String::new();
    for (i, k) in VALUE_RULES
        .iter()
        .chain(PREDICATES)
        .chain(LOGIC)
        .chain(DEFERRED)
        .chain(std::iter::once(&DESCRIBED))
        .enumerate()
    {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(k);
    }
    out
}

impl<'de> serde::Deserialize<'de> for Rule {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(RuleVisitor)
    }
}

struct RuleVisitor;

impl<'de> Visitor<'de> for RuleVisitor {
    type Value = Rule;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "a rule as a bare string (unit variant) or map with a single rule key")
    }

    // Unit variants: "email", "url".
    fn visit_str<E: de::Error>(self, s: &str) -> Result<Self::Value, E> {
        match s {
            "email" => Ok(Rule::Value(ValueRule::Email)),
            "url" => Ok(Rule::Value(ValueRule::Url)),
            other => Err(E::custom(format!(
                "unknown unit rule {other:?}; known unit rules: email, url"
            ))),
        }
    }

    fn visit_string<E: de::Error>(self, s: String) -> Result<Self::Value, E> {
        self.visit_str(&s)
    }

    fn visit_map<M: MapAccess<'de>>(self, mut m: M) -> Result<Self::Value, M::Error> {
        let Some(key) = m.next_key::<String>()? else {
            return Err(de::Error::custom("empty rule object"));
        };

        let rule = match key.as_str() {
            // ── Value rules ────────────────────────────────────────────
            "min_length" => Rule::Value(ValueRule::MinLength(m.next_value()?)),
            "max_length" => Rule::Value(ValueRule::MaxLength(m.next_value()?)),
            "pattern" => Rule::Value(ValueRule::Pattern(m.next_value()?)),
            "min" => Rule::Value(ValueRule::Min(m.next_value()?)),
            "max" => Rule::Value(ValueRule::Max(m.next_value()?)),
            "greater_than" => Rule::Value(ValueRule::GreaterThan(m.next_value()?)),
            "less_than" => Rule::Value(ValueRule::LessThan(m.next_value()?)),
            "one_of" => Rule::Value(ValueRule::OneOf(m.next_value()?)),
            "min_items" => Rule::Value(ValueRule::MinItems(m.next_value()?)),
            "max_items" => Rule::Value(ValueRule::MaxItems(m.next_value()?)),
            "email" => Rule::Value(ValueRule::Email),
            "url" => Rule::Value(ValueRule::Url),

            // ── Predicates ─────────────────────────────────────────────
            "eq" => {
                let (p, v): (FieldPath, serde_json::Value) = m.next_value()?;
                Rule::Predicate(Predicate::Eq(p, v))
            },
            "ne" => {
                let (p, v): (FieldPath, serde_json::Value) = m.next_value()?;
                Rule::Predicate(Predicate::Ne(p, v))
            },
            "gt" => {
                let (p, v): (FieldPath, serde_json::Number) = m.next_value()?;
                Rule::Predicate(Predicate::Gt(p, v))
            },
            "gte" => {
                let (p, v): (FieldPath, serde_json::Number) = m.next_value()?;
                Rule::Predicate(Predicate::Gte(p, v))
            },
            "lt" => {
                let (p, v): (FieldPath, serde_json::Number) = m.next_value()?;
                Rule::Predicate(Predicate::Lt(p, v))
            },
            "lte" => {
                let (p, v): (FieldPath, serde_json::Number) = m.next_value()?;
                Rule::Predicate(Predicate::Lte(p, v))
            },
            "is_true" => Rule::Predicate(Predicate::IsTrue(m.next_value()?)),
            "is_false" => Rule::Predicate(Predicate::IsFalse(m.next_value()?)),
            "set" => Rule::Predicate(Predicate::Set(m.next_value()?)),
            "empty" => Rule::Predicate(Predicate::Empty(m.next_value()?)),
            "contains" => {
                let (p, v): (FieldPath, serde_json::Value) = m.next_value()?;
                Rule::Predicate(Predicate::Contains(p, v))
            },
            "matches" => {
                let (p, pat): (FieldPath, String) = m.next_value()?;
                Rule::Predicate(Predicate::Matches(p, pat))
            },
            "in" => {
                let (p, vs): (FieldPath, Vec<serde_json::Value>) = m.next_value()?;
                Rule::Predicate(Predicate::In(p, vs))
            },

            // ── Logic ──────────────────────────────────────────────────
            "all" => Rule::Logic(Box::new(Logic::All(m.next_value()?))),
            "any" => Rule::Logic(Box::new(Logic::Any(m.next_value()?))),
            "not" => Rule::Logic(Box::new(Logic::Not(m.next_value()?))),

            // ── Deferred ───────────────────────────────────────────────
            "custom" => Rule::Deferred(DeferredRule::Custom(m.next_value()?)),
            "unique_by" => Rule::Deferred(DeferredRule::UniqueBy(m.next_value()?)),

            // ── Described ──────────────────────────────────────────────
            "described" => {
                let (inner, msg): (Rule, String) = m.next_value()?;
                Rule::Described(Box::new(inner), msg)
            },

            other => {
                return Err(de::Error::custom(format!(
                    "unknown rule {other:?}. Known rules: {}",
                    all_known()
                )));
            },
        };

        // Defensive: reject trailing keys — rules are single-key objects.
        if let Some(extra) = m.next_key::<String>()? {
            return Err(de::Error::custom(format!(
                "rule object must have exactly one key; found extra key {extra:?}"
            )));
        }

        Ok(rule)
    }
}
```

- [ ] **Step 2: Do not commit yet — Task 9 introduces the `Rule` outer enum that this refers to**

Check the file is valid Rust by running (it won't compile until Task 9 lands):

```bash
grep -c 'fn ' crates/validator/src/rule/deserialize.rs
```
Expected: prints a number > 5 (sanity check that the file saved).

---

## Task 9: Replace outer `Rule` — the atomic structural commit

**Files:**
- Modify: `crates/validator/src/rule/mod.rs`
- Delete: `crates/validator/src/rule/validate.rs`
- Delete: `crates/validator/src/rule/evaluate.rs`
- Delete: `crates/validator/src/rule/classify.rs`
- Modify: `crates/validator/src/rule/constructors.rs`
- Modify: `crates/validator/src/rule/tests.rs` (replace with minimal — detailed tests arrive in Task 14)

**Rationale:** This is the atomic swap. Before this task the new sub-enums exist on disk but are unreachable. After this task, the flat 30-variant `Rule` is gone and replaced with the sum-of-sums. `Validate<serde_json::Value>` impl stays working via `Rule::validate`.

- [ ] **Step 1: Replace `rule/mod.rs` wholesale**

Write new content to `crates/validator/src/rule/mod.rs`:

```rust
//! Unified declarative rule system — sum-of-sums over typed kinds.
//!
//! [`Rule`] is a classifier over four semantic kinds:
//!
//! | Kind | Inner type | Method | Wire tag |
//! |---|---|---|---|
//! | Value validation | [`ValueRule`] | `validate_value(&Value)` | `{"min_length": 3}` etc. |
//! | Context predicate | [`Predicate`] | `evaluate(&PredicateContext)` | `{"eq": ["/path", value]}` |
//! | Logical combinator | [`Logic`] | recursive `validate` | `{"all": [...]}` |
//! | Deferred | [`DeferredRule`] | runtime-evaluated | `{"custom": "expr"}` |
//!
//! The `Described(Box<Rule>, String)` decorator wraps any rule with a
//! custom message (may contain `{placeholder}` templates).
//!
//! Unit variants (`Email`, `Url`) serialize as bare strings.

pub mod context;
pub mod deferred;
mod deserialize;
pub mod logic;
pub mod predicate;
pub mod value;

mod constructors;
mod helpers;

#[cfg(test)]
mod tests;

pub use context::PredicateContext;
pub use deferred::DeferredRule;
pub use logic::Logic;
pub use predicate::Predicate;
use serde::Serialize;
pub use value::ValueRule;

use serde::ser::SerializeMap;

use crate::{engine::ExecutionMode, foundation::ValidationError};

/// Borrowed view over a value bag used by predicate rules.
///
/// Kept for backward compatibility with callers that built their own
/// `HashMap<String, Value>`-based contexts. New code should prefer
/// [`PredicateContext`].
pub trait RuleContext {
    /// Fetch a value by key.
    fn get(&self, key: &str) -> Option<&serde_json::Value>;
}

impl RuleContext for std::collections::HashMap<String, serde_json::Value> {
    fn get(&self, key: &str) -> Option<&serde_json::Value> {
        std::collections::HashMap::get(self, key)
    }
}

/// Unified declarative rule. See module docs.
///
/// `Serialize` is manual because `Described` must emit as
/// `{"described": [inner, msg]}`, which `#[serde(untagged)]` on a tuple
/// variant cannot produce. `Deserialize` is manual for friendly
/// unknown-variant errors (see `rule/deserialize.rs`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Rule {
    /// Value-validation rule.
    Value(ValueRule),
    /// Context predicate.
    Predicate(Predicate),
    /// Logical combinator.
    Logic(Box<Logic>),
    /// Deferred runtime rule.
    Deferred(DeferredRule),
    /// Wrapper with custom error message.
    Described(Box<Rule>, String),
}

impl Serialize for Rule {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Value(v) => v.serialize(s),
            Self::Predicate(p) => p.serialize(s),
            Self::Logic(l) => l.as_ref().serialize(s),
            Self::Deferred(d) => d.serialize(s),
            Self::Described(inner, msg) => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("described", &(inner.as_ref(), msg))?;
                m.end()
            },
        }
    }
}

impl Rule {
    /// Validates an input against this rule using the given execution mode.
    ///
    /// `ctx = None` short-circuits `Predicate` dispatch to `Ok(())` —
    /// matching the two-tier semantics (sync client skips, full server
    /// evaluates). Deferred rules are skipped when
    /// `mode == ExecutionMode::StaticOnly`.
    pub fn validate(
        &self,
        input: &serde_json::Value,
        ctx: Option<&PredicateContext>,
        mode: ExecutionMode,
    ) -> Result<(), ValidationError> {
        match self {
            Self::Value(v) => v.validate_value(input),
            Self::Predicate(p) => match ctx {
                Some(c) if p.evaluate(c) => Ok(()),
                Some(_) => Err(ValidationError::new(
                    predicate_code(p),
                    "predicate failed",
                )
                .with_field_path(p.field().clone())),
                None => Ok(()),
            },
            Self::Logic(l) => l.validate(input, ctx, mode),
            Self::Deferred(_) if mode == ExecutionMode::StaticOnly => Ok(()),
            Self::Deferred(d) => d.validate(input, ctx),
            Self::Described(inner, msg) => inner
                .validate(input, ctx, mode)
                .map_err(|mut e| {
                    e.message = std::borrow::Cow::Owned(msg.clone());
                    e
                }),
        }
    }

    /// Classifies this rule by kind — cheap non-recursive check.
    #[must_use]
    pub fn kind(&self) -> RuleKind {
        match self {
            Self::Value(_) => RuleKind::Value,
            Self::Predicate(_) => RuleKind::Predicate,
            Self::Logic(_) => RuleKind::Logic,
            Self::Deferred(_) => RuleKind::Deferred,
            Self::Described(inner, _) => inner.kind(),
        }
    }

    /// True if this rule needs runtime context (Deferred).
    pub fn is_deferred(&self) -> bool {
        matches!(self.kind(), RuleKind::Deferred)
    }
}

/// Four kinds of rule for classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RuleKind {
    /// Value-validation rule.
    Value,
    /// Context predicate.
    Predicate,
    /// Logical combinator.
    Logic,
    /// Deferred runtime rule.
    Deferred,
}

fn predicate_code(p: &Predicate) -> &'static str {
    match p {
        Predicate::Eq(_, _) => "eq_failed",
        Predicate::Ne(_, _) => "ne_failed",
        Predicate::Gt(_, _) => "gt_failed",
        Predicate::Gte(_, _) => "gte_failed",
        Predicate::Lt(_, _) => "lt_failed",
        Predicate::Lte(_, _) => "lte_failed",
        Predicate::IsTrue(_) => "is_true_failed",
        Predicate::IsFalse(_) => "is_false_failed",
        Predicate::Set(_) => "set_failed",
        Predicate::Empty(_) => "empty_failed",
        Predicate::Contains(_, _) => "contains_failed",
        Predicate::Matches(_, _) => "matches_failed",
        Predicate::In(_, _) => "in_failed",
    }
}

impl crate::foundation::Validate<serde_json::Value> for Rule {
    fn validate(&self, input: &serde_json::Value) -> Result<(), ValidationError> {
        Rule::validate(self, input, None, ExecutionMode::StaticOnly)
    }
}
```

Note: the `#[serde(untagged)]` on the outer `Rule` is for **serialization only** — the manual `Deserialize` from Task 8 overrides the deserialize path. This gives correct wire output (`{"min_length": 3}` not `{"Value": {"min_length": 3}}`).

Check the Serialize side doesn't double-wrap by verifying in Task 14.

- [ ] **Step 2: Delete old dispatch files**

```bash
git rm crates/validator/src/rule/validate.rs crates/validator/src/rule/evaluate.rs crates/validator/src/rule/classify.rs
```

- [ ] **Step 3: Replace constructors.rs**

Write new content to `crates/validator/src/rule/constructors.rs`:

```rust
//! Ergonomic constructors for [`Rule`] and inner sub-enums.

use super::{DeferredRule, Logic, Predicate, Rule, ValueRule};
use crate::foundation::FieldPath;

// ── ValueRule ────────────────────────────────────────────────────────────
impl ValueRule {
    /// Creates a [`ValueRule::MinLength`].
    #[must_use]
    pub fn min_length(n: usize) -> Self {
        Self::MinLength(n)
    }
    /// Creates a [`ValueRule::MaxLength`].
    #[must_use]
    pub fn max_length(n: usize) -> Self {
        Self::MaxLength(n)
    }
    /// Creates a [`ValueRule::Pattern`].
    #[must_use]
    pub fn pattern(p: impl Into<String>) -> Self {
        Self::Pattern(p.into())
    }
}

// ── Predicate ────────────────────────────────────────────────────────────
impl Predicate {
    /// Creates a [`Predicate::Eq`]. Returns `None` if the path is invalid.
    #[must_use]
    pub fn eq(field: impl AsRef<str>, value: impl Into<serde_json::Value>) -> Option<Self> {
        Some(Self::Eq(FieldPath::parse(field)?, value.into()))
    }
}

// ── Rule: shorthand wrappers ─────────────────────────────────────────────
impl Rule {
    /// Wraps a [`ValueRule`] into [`Rule::Value`].
    #[must_use]
    pub fn value(v: ValueRule) -> Self {
        Self::Value(v)
    }

    /// Wraps a [`Predicate`] into [`Rule::Predicate`].
    #[must_use]
    pub fn predicate(p: Predicate) -> Self {
        Self::Predicate(p)
    }

    /// Creates a [`Rule::Value(ValueRule::MinLength)`].
    #[must_use]
    pub fn min_length(n: usize) -> Self {
        Self::Value(ValueRule::MinLength(n))
    }

    /// Creates a [`Rule::Value(ValueRule::MaxLength)`].
    #[must_use]
    pub fn max_length(n: usize) -> Self {
        Self::Value(ValueRule::MaxLength(n))
    }

    /// Creates a [`Rule::Value(ValueRule::Pattern)`]. Regex is not
    /// validated at construction.
    #[must_use]
    pub fn pattern(p: impl Into<String>) -> Self {
        Self::Value(ValueRule::Pattern(p.into()))
    }

    /// Creates a [`Rule::Value(ValueRule::Pattern)`], validating the regex.
    /// Returns `None` if the pattern is invalid.
    #[must_use]
    pub fn try_pattern(p: impl Into<String>) -> Option<Self> {
        let p = p.into();
        regex::Regex::new(&p).ok()?;
        Some(Self::Value(ValueRule::Pattern(p)))
    }

    /// Creates a [`Rule::Value(ValueRule::Min)`] from an `i64`.
    #[must_use]
    pub fn min_value(n: i64) -> Self {
        Self::Value(ValueRule::Min(serde_json::Number::from(n)))
    }

    /// Creates a [`Rule::Value(ValueRule::Max)`] from an `i64`.
    #[must_use]
    pub fn max_value(n: i64) -> Self {
        Self::Value(ValueRule::Max(serde_json::Number::from(n)))
    }

    /// Creates a [`Rule::Value(ValueRule::Min)`] from an `f64`. Returns
    /// `None` if the value is NaN or infinite.
    #[must_use]
    pub fn min_value_f64(n: f64) -> Option<Self> {
        Some(Self::Value(ValueRule::Min(serde_json::Number::from_f64(n)?)))
    }

    /// Creates a [`Rule::Value(ValueRule::Max)`] from an `f64`. Returns
    /// `None` if the value is NaN or infinite.
    #[must_use]
    pub fn max_value_f64(n: f64) -> Option<Self> {
        Some(Self::Value(ValueRule::Max(serde_json::Number::from_f64(n)?)))
    }

    /// Creates a [`Rule::Value(ValueRule::GreaterThan)`] from an `i64`.
    #[must_use]
    pub fn greater_than(n: i64) -> Self {
        Self::Value(ValueRule::GreaterThan(serde_json::Number::from(n)))
    }

    /// Creates a [`Rule::Value(ValueRule::LessThan)`] from an `i64`.
    #[must_use]
    pub fn less_than(n: i64) -> Self {
        Self::Value(ValueRule::LessThan(serde_json::Number::from(n)))
    }

    /// Creates a [`Rule::Value(ValueRule::OneOf)`].
    #[must_use]
    pub fn one_of<V: Into<serde_json::Value>>(values: impl IntoIterator<Item = V>) -> Self {
        Self::Value(ValueRule::OneOf(values.into_iter().map(Into::into).collect()))
    }

    /// Creates a [`Rule::Value(ValueRule::MinItems)`].
    #[must_use]
    pub fn min_items(n: usize) -> Self {
        Self::Value(ValueRule::MinItems(n))
    }

    /// Creates a [`Rule::Value(ValueRule::MaxItems)`].
    #[must_use]
    pub fn max_items(n: usize) -> Self {
        Self::Value(ValueRule::MaxItems(n))
    }

    /// Creates a [`Rule::Value(ValueRule::Email)`].
    #[must_use]
    pub fn email() -> Self {
        Self::Value(ValueRule::Email)
    }

    /// Creates a [`Rule::Value(ValueRule::Url)`].
    #[must_use]
    pub fn url() -> Self {
        Self::Value(ValueRule::Url)
    }

    /// Creates a [`Rule::Deferred(DeferredRule::Custom)`].
    #[must_use]
    pub fn custom(expression: impl Into<String>) -> Self {
        Self::Deferred(DeferredRule::Custom(expression.into()))
    }

    /// Creates a [`Rule::Deferred(DeferredRule::UniqueBy)`]. Returns
    /// `None` if the path is invalid.
    #[must_use]
    pub fn unique_by(path: impl AsRef<str>) -> Option<Self> {
        Some(Self::Deferred(DeferredRule::UniqueBy(FieldPath::parse(path)?)))
    }

    /// Creates a [`Rule::Logic(Logic::All)`].
    #[must_use]
    pub fn all(rules: impl IntoIterator<Item = Rule>) -> Self {
        Self::Logic(Box::new(Logic::All(rules.into_iter().collect())))
    }

    /// Creates a [`Rule::Logic(Logic::Any)`].
    #[must_use]
    pub fn any(rules: impl IntoIterator<Item = Rule>) -> Self {
        Self::Logic(Box::new(Logic::Any(rules.into_iter().collect())))
    }

    /// Creates a [`Rule::Logic(Logic::Not)`].
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "this is a rule constructor, not boolean negation"
    )]
    pub fn not(inner: Rule) -> Self {
        Self::Logic(Box::new(Logic::Not(inner)))
    }

    /// Wraps the rule with a custom error message.
    #[must_use]
    pub fn described(rule: Rule, message: impl Into<String>) -> Self {
        Self::Described(Box::new(rule), message.into())
    }

    /// Consumes `self` and wraps it in [`Rule::Described`] with a message.
    /// Sugar for building rules in method-chain style.
    #[must_use]
    pub fn with_message(self, message: impl Into<String>) -> Self {
        Self::described(self, message)
    }
}
```

- [ ] **Step 4: Collapse `tests.rs` to a placeholder**

Replace `crates/validator/src/rule/tests.rs` entirely with:

```rust
//! Integration coverage is in `tests/integration/rule_*`. Keep this
//! file for unit-level smoke tests of the new API only.

use serde_json::json;

use super::{Predicate, Rule, RuleKind, ValueRule};
use crate::foundation::FieldPath;

#[test]
fn constructors_build_value_kind() {
    assert_eq!(Rule::min_length(3).kind(), RuleKind::Value);
}

#[test]
fn constructors_build_logic_kind() {
    assert_eq!(
        Rule::all([Rule::min_length(3)]).kind(),
        RuleKind::Logic
    );
}

#[test]
fn described_inherits_inner_kind() {
    let r = Rule::email().with_message("bad mail");
    assert_eq!(r.kind(), RuleKind::Value);
}

#[test]
fn is_deferred_tags_custom() {
    assert!(Rule::custom("check()").is_deferred());
    assert!(!Rule::email().is_deferred());
}

#[test]
fn predicate_eq_constructor_parses_path() {
    let p = Predicate::eq("status", json!("active")).unwrap();
    assert_eq!(p.field().as_str(), "/status");
}

#[test]
fn value_rule_direct_construction_still_works() {
    let v = ValueRule::MinLength(3);
    assert!(v.validate_value(&json!("abc")).is_ok());
    assert!(v.validate_value(&json!("ab")).is_err());
    let _ = FieldPath::parse("x").unwrap(); // ensures FieldPath is still in tree
}
```

- [ ] **Step 5: Add `with_message` sugar for `ValidationError` if not present**

We reused `e.message = Cow::Owned(...)` directly inside `Rule::validate`. Confirm `ValidationError::message` is a public `pub` field — check `crates/validator/src/foundation/error/validation_error.rs` line ~112:

```bash
grep -n "pub message:" crates/validator/src/foundation/error/validation_error.rs
```
Expected output: a line with `pub message: Cow<'static, str>,`. If it's private, add a pub method `with_message` similar to `with_field`. (It is public per our earlier read.)

- [ ] **Step 6: Update `lib.rs` re-exports**

In `crates/validator/src/lib.rs`, locate the `pub use rule::{Rule, RuleContext};` line and replace with:

```rust
pub use rule::{DeferredRule, Logic, Predicate, PredicateContext, Rule, RuleContext, RuleKind, ValueRule};
```

- [ ] **Step 7: Compile — expect failures in schema consumers**

Run: `cargo check -p nebula-validator`
Expected: PASS.

Run: `cargo check -p nebula-schema`
Expected: FAIL with many errors about `Rule::MinLength { min: .., message: .. }` not matching new shape. Record the count:

```bash
cargo check -p nebula-schema 2>&1 | grep -c 'error\[E'
```

- [ ] **Step 8: Do not commit yet — Tasks 11 and 12 fix schema consumers**

Leave the worktree with the validator rebuilt. Tasks 11–12 will fix the schema compile breakage before we commit. This task's work is staged (additions + deletions) ready to include in the next commit.

---

## Task 10: Update `engine::validate_rules` for new dispatch

**Files:**
- Modify: `crates/validator/src/engine.rs`

**Rationale:** The engine currently calls `rule.validate_value(value)`. After the refactor the correct call is `rule.validate(value, None, mode)`. Predicate-bearing slices can also carry `ctx` — add an optional parameter.

- [ ] **Step 1: Update validate_rules signature and body**

In `crates/validator/src/engine.rs`, replace the existing `validate_rules` function body. Current (lines ~76–109):

```rust
pub fn validate_rules(
    value: &serde_json::Value,
    rules: &[Rule],
    mode: ExecutionMode,
) -> Result<(), ValidationErrors> {
    if rules.is_empty() {
        return Ok(());
    }
    let mut errors = Vec::new();
    for rule in rules {
        let should_run = match mode {
            ExecutionMode::StaticOnly => !rule.is_deferred(),
            ExecutionMode::Deferred => rule.is_deferred(),
            ExecutionMode::Full => true,
        };
        if !should_run {
            continue;
        }
        if let Err(e) = rule.validate_value(value) {
            errors.push(e);
        }
    }
    ...
}
```

New:

```rust
pub fn validate_rules(
    value: &serde_json::Value,
    rules: &[Rule],
    mode: ExecutionMode,
) -> Result<(), ValidationErrors> {
    validate_rules_with_ctx(value, rules, None, mode)
}

/// Validates with an optional predicate context. Rules whose kind doesn't
/// match `mode` are skipped.
pub fn validate_rules_with_ctx(
    value: &serde_json::Value,
    rules: &[Rule],
    ctx: Option<&crate::rule::PredicateContext>,
    mode: ExecutionMode,
) -> Result<(), ValidationErrors> {
    if rules.is_empty() {
        return Ok(());
    }
    let mut errors = Vec::new();
    for rule in rules {
        let should_run = match mode {
            ExecutionMode::StaticOnly => !rule.is_deferred(),
            ExecutionMode::Deferred => rule.is_deferred(),
            ExecutionMode::Full => true,
        };
        if !should_run {
            continue;
        }
        if let Err(e) = rule.validate(value, ctx, mode) {
            errors.push(e);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into_iter().collect())
    }
}
```

- [ ] **Step 2: Update the engine's inline tests**

In `crates/validator/src/engine.rs`, the `#[cfg(test)] mod tests` block has fixtures like `Rule::MinLength { min: 3, message: None }`. Replace each with `Rule::min_length(3)`, `Rule::eq("x", json!(1)).unwrap()`, etc.

Concrete substitutions (use `sed`-style edits or rewrite the block wholesale):

| Old | New |
|---|---|
| `Rule::MinLength { min: 3, message: None }` | `Rule::min_length(3)` |
| `Rule::MaxLength { max: 10, message: None }` | `Rule::max_length(10)` |
| `Rule::Pattern { pattern: "^[0-9]+$".into(), message: None }` | `Rule::pattern("^[0-9]+$")` |
| `Rule::Custom { expression: "...".into(), message: None }` | `Rule::custom("...")` |
| `Rule::UniqueBy { key: "id".into(), message: None }` | `Rule::unique_by("id").unwrap()` |
| `Rule::Eq { field: "x".into(), value: json!(1) }` | `Rule::predicate(Predicate::eq("x", json!(1)).unwrap())` |
| `Rule::All { rules: vec![...] }` | `Rule::all([...])` |

- [ ] **Step 3: Verify the engine compiles + tests pass**

Run: `cargo nextest run -p nebula-validator --lib engine`
Expected: PASS (assumes Tasks 11–12 aren't needed for this path; engine doesn't touch schema).

- [ ] **Step 4: Do not commit yet — continues in Task 11**

---

## Task 11: Update `crates/schema/src/lint.rs` — pattern migration

**Files:**
- Modify: `crates/schema/src/lint.rs`

**Rationale:** lint.rs has ~40 pattern-match sites on flat `Rule::*` variants. After the split they become `Rule::Value(ValueRule::X)` etc.

- [ ] **Step 1: Read current structure**

Open `crates/schema/src/lint.rs` and locate the two big `match rule { ... }` blocks: the `supports_rules` check (around line 341) and the `rule_code` name mapping (around line 455).

- [ ] **Step 2: Update the `supports_rules` block**

Replace the existing match arms in the function that returns `bool` for a given field + rule combination. Old (lines ~341–357):

```rust
Rule::Pattern { .. }
| Rule::MinLength { .. }
| Rule::MaxLength { .. }
| Rule::Email { .. }
| Rule::Url { .. } => supports_string_rules(field),
Rule::Min { .. } | Rule::Max { .. } => supports_number_rules(field),
Rule::MinItems { .. } | Rule::MaxItems { .. } => supports_collection_rules(field),
Rule::All { rules } | Rule::Any { rules } => {
    rules.iter().all(|r| rule_supported_for_field(r, field))
},
Rule::Not { inner } => rule_supported_for_field(inner, field),
_ => true,
```

New:

```rust
Rule::Value(v) => match v {
    ValueRule::Pattern(_)
    | ValueRule::MinLength(_)
    | ValueRule::MaxLength(_)
    | ValueRule::Email
    | ValueRule::Url => supports_string_rules(field),
    ValueRule::Min(_) | ValueRule::Max(_) | ValueRule::GreaterThan(_) | ValueRule::LessThan(_) => {
        supports_number_rules(field)
    },
    ValueRule::MinItems(_) | ValueRule::MaxItems(_) => supports_collection_rules(field),
    _ => true,
},
Rule::Logic(l) => match l.as_ref() {
    Logic::All(rs) | Logic::Any(rs) => rs.iter().all(|r| rule_supported_for_field(r, field)),
    Logic::Not(inner) => rule_supported_for_field(inner, field),
},
Rule::Described(inner, _) => rule_supported_for_field(inner, field),
_ => true,
```

- [ ] **Step 3: Update the `rule_code` name mapping block**

Old (lines ~455–482):

```rust
Rule::Pattern { .. } => "pattern",
Rule::MinLength { .. } => "min_length",
Rule::MaxLength { .. } => "max_length",
Rule::Min { .. } => "min",
Rule::Max { .. } => "max",
Rule::OneOf { .. } => "one_of",
Rule::MinItems { .. } => "min_items",
Rule::MaxItems { .. } => "max_items",
Rule::Email { .. } => "email",
Rule::Url { .. } => "url",
Rule::UniqueBy { .. } => "unique_by",
Rule::Custom { .. } => "custom",
Rule::Eq { .. } => "eq",
Rule::Ne { .. } => "ne",
// ...
Rule::All { .. } => "all",
Rule::Any { .. } => "any",
Rule::Not { .. } => "not",
```

New:

```rust
Rule::Value(v) => match v {
    ValueRule::Pattern(_) => "pattern",
    ValueRule::MinLength(_) => "min_length",
    ValueRule::MaxLength(_) => "max_length",
    ValueRule::Min(_) => "min",
    ValueRule::Max(_) => "max",
    ValueRule::GreaterThan(_) => "greater_than",
    ValueRule::LessThan(_) => "less_than",
    ValueRule::OneOf(_) => "one_of",
    ValueRule::MinItems(_) => "min_items",
    ValueRule::MaxItems(_) => "max_items",
    ValueRule::Email => "email",
    ValueRule::Url => "url",
},
Rule::Deferred(d) => match d {
    DeferredRule::UniqueBy(_) => "unique_by",
    DeferredRule::Custom(_) => "custom",
},
Rule::Predicate(p) => match p {
    Predicate::Eq(_, _) => "eq",
    Predicate::Ne(_, _) => "ne",
    Predicate::Gt(_, _) => "gt",
    Predicate::Gte(_, _) => "gte",
    Predicate::Lt(_, _) => "lt",
    Predicate::Lte(_, _) => "lte",
    Predicate::IsTrue(_) => "is_true",
    Predicate::IsFalse(_) => "is_false",
    Predicate::Set(_) => "set",
    Predicate::Empty(_) => "empty",
    Predicate::Contains(_, _) => "contains",
    Predicate::Matches(_, _) => "matches",
    Predicate::In(_, _) => "in",
},
Rule::Logic(l) => match l.as_ref() {
    Logic::All(_) => "all",
    Logic::Any(_) => "any",
    Logic::Not(_) => "not",
},
Rule::Described(inner, _) => rule_code(inner),
```

Add the necessary `use` imports at the top of `lint.rs`:

```rust
use nebula_validator::{DeferredRule, Logic, Predicate, Rule, ValueRule};
```

- [ ] **Step 4: Update the nested `match rule` around line ~496 (error code resolution)**

Old:

```rust
Rule::MinLength { min, .. } => { ... }
Rule::MaxLength { max, .. } => { ... }
Rule::MinItems { min, .. } => { ... }
Rule::MaxItems { max, .. } => { ... }
Rule::All { rules } | Rule::Any { rules } => { ... }
Rule::Not { inner } => { ... }
```

New:

```rust
Rule::Value(ValueRule::MinLength(min)) => { /* use *min */ }
Rule::Value(ValueRule::MaxLength(max)) => { /* use *max */ }
Rule::Value(ValueRule::MinItems(min)) => { /* use *min */ }
Rule::Value(ValueRule::MaxItems(max)) => { /* use *max */ }
Rule::Logic(l) => match l.as_ref() {
    Logic::All(rules) | Logic::Any(rules) => { /* unchanged body */ }
    Logic::Not(inner) => { /* unchanged body */ }
},
Rule::Described(inner, _) => { /* recurse on inner */ }
_ => {}
```

(Read the actual surrounding bodies — they compute something with `min` / `max` / recurse — preserve that logic.)

- [ ] **Step 5: Verify schema compiles**

Run: `cargo check -p nebula-schema`
Expected: PASS (or only errors about `field.rs` which is Task 12).

---

## Task 12: Update `crates/schema/src/field.rs` — builder migration

**Files:**
- Modify: `crates/schema/src/field.rs`

**Rationale:** field.rs has ~10 sites where builders push `Rule::MinLength { min, message: None }` etc. Replace with new constructors.

- [ ] **Step 1: Migrate each construction site**

Find all `Rule::` sites with:

```bash
grep -n "Rule::" crates/schema/src/field.rs
```

Substitutions:

| Old | New |
|---|---|
| `Rule::MinLength { min, message: None }` | `Rule::min_length(min)` |
| `Rule::MaxLength { max, message: None }` | `Rule::max_length(max)` |
| `Rule::Pattern { pattern: ..., message: None }` | `Rule::pattern(pattern)` |
| `Rule::Url { message: None }` | `Rule::url()` |
| `Rule::Email { message: None }` | `Rule::email()` |
| `Rule::Min { min: ..., message: None }` | `Rule::min_value_f64(n).unwrap_or_else(\|\| Rule::min_value(n as i64))` or keep as `Rule::Value(ValueRule::Min(serde_json::Number::from_f64(n).unwrap()))` depending on the type in scope. |
| `Rule::Max { max: ..., message: None }` | analogous |

Read each site carefully — some pass `min: serde_json::Number::from_f64(v)?`, in which case the replacement is:

```rust
self.rules.push(Rule::Value(ValueRule::Min(serde_json::Number::from_f64(v)?)));
```

- [ ] **Step 2: Verify workspace compiles**

Run: `cargo check --workspace`
Expected: PASS.

- [ ] **Step 3: Run schema tests**

Run: `cargo nextest run -p nebula-schema`
Expected: PASS. Some pre-existing tests may fail if wire format is recorded in fixtures — note any failures, they'll be addressed in Task 17 (fixture regeneration).

- [ ] **Step 4: Commit the big cross-crate refactor**

This commit bundles Tasks 9, 10, 11, 12 since they can only compile together. Stage everything:

```bash
# Deletions were already staged in Task 9 Step 2 via `git rm`; just add modifications.
git add crates/validator/src/rule/mod.rs crates/validator/src/rule/constructors.rs \
       crates/validator/src/rule/deserialize.rs crates/validator/src/rule/tests.rs \
       crates/validator/src/rule/value.rs crates/validator/src/rule/predicate.rs \
       crates/validator/src/rule/logic.rs crates/validator/src/rule/deferred.rs \
       crates/validator/src/rule/context.rs \
       crates/validator/src/engine.rs crates/validator/src/lib.rs \
       crates/schema/src/lint.rs crates/schema/src/field.rs
git commit -m "$(cat <<'EOF'
refactor(validator): split flat Rule enum into typed sum-of-sums

Replaces the 30-variant flat Rule with Rule::{Value, Predicate, Logic,
Deferred, Described} delegating to typed sub-enums. Each inner enum has
exactly one method that makes sense for its kind — value rules get
validate_value, predicates get evaluate, combinators get recursive
validate. Cross-kind silent-pass is gone; intra-kind silent-pass on
JSON type mismatch is preserved as documented ergonomic.

Wire format switches to externally-tagged tuple-compact form:
{"min_length": 3}, {"eq": ["/path", value]}, "email" for unit variants.
~60% reduction on compound rule arrays. Custom Deserialize gives
friendly "unknown rule X. Known rules: ..." errors instead of serde's
generic "data did not match any variant".

FieldPath now serializes as its inner JSON Pointer. Predicates carry
FieldPath (not String) so paths are validated at construction.
PredicateContext newtype replaces raw HashMap<String, Value>.

Described(Box<Rule>, String) decorator replaces per-variant message
fields and works across combinators (not just leaf rules). Message
strings can contain {placeholder} templates that render from the
error's params at Display time.

Breaking: wire format, pattern-match sites across workspace. Updates
lint/field in crates/schema. No stored data exists yet; alpha allows
clean cut.

Spec: docs/superpowers/specs/2026-04-17-nebula-validator-rule-refactor-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: Update `tests/integration/rule_roundtrip.rs` + register new modules

**Files:**
- Modify: `crates/validator/tests/integration/rule_roundtrip.rs`
- Modify: `crates/validator/tests/integration/main.rs`

**Rationale:** Existing tests assert old wire form (`encoded["rule"] == "min_length"`). Update to the new shape and verify behavior parity. Register the four new integration modules (to be written in Tasks 14–17).

- [ ] **Step 1: Replace rule_roundtrip.rs body**

Overwrite `crates/validator/tests/integration/rule_roundtrip.rs` with:

```rust
//! Scenario: `Rule` serialization contract — new externally-tagged
//! tuple-compact wire format. Rules pulled from JSON config must
//! roundtrip losslessly and validate the same values post-roundtrip.

use nebula_validator::{Predicate, Rule, ValueRule};
use serde_json::json;

#[test]
fn value_rule_compact_wire_form() {
    let rule = Rule::min_length(3);
    let encoded = serde_json::to_value(&rule).unwrap();
    assert_eq!(encoded, json!({"min_length": 3}));
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule);
}

#[test]
fn unit_rule_bare_string_wire_form() {
    let rule = Rule::email();
    let encoded = serde_json::to_value(&rule).unwrap();
    assert_eq!(encoded, json!("email"));
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule);
}

#[test]
fn predicate_rule_tuple_wire_form() {
    let rule = Rule::predicate(Predicate::eq("status", json!("active")).unwrap());
    let encoded = serde_json::to_value(&rule).unwrap();
    assert_eq!(encoded, json!({"eq": ["/status", "active"]}));
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule);
}

#[test]
fn combinator_wire_form() {
    let rule = Rule::all([Rule::min_length(3), Rule::max_length(20)]);
    let encoded = serde_json::to_value(&rule).unwrap();
    assert_eq!(
        encoded,
        json!({"all": [{"min_length": 3}, {"max_length": 20}]})
    );
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule);
}

#[test]
fn described_wire_form() {
    let rule = Rule::min_length(3).with_message("too short");
    let encoded = serde_json::to_value(&rule).unwrap();
    assert_eq!(encoded, json!({"described": [{"min_length": 3}, "too short"]}));
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule);
}

#[test]
fn roundtrip_preserves_validation_behavior() {
    let original = Rule::pattern(r"^[a-z]+$");
    let decoded: Rule = serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();

    for probe in [json!("hello"), json!("Bad1"), json!(42), json!(null)] {
        let a = nebula_validator::foundation::Validate::validate(&original, &probe).is_ok();
        let b = nebula_validator::foundation::Validate::validate(&decoded, &probe).is_ok();
        assert_eq!(a, b, "rules disagree on {probe:?}");
    }
}

#[test]
fn described_roundtrip_with_template() {
    let rule = Rule::min_length(5).with_message("got {value}, need {min}");
    let decoded: Rule = serde_json::from_value(serde_json::to_value(&rule).unwrap()).unwrap();
    let err = nebula_validator::foundation::Validate::validate(&decoded, &json!("hi")).unwrap_err();
    let rendered = format!("{err}");
    assert!(rendered.contains("got \"hi\", need 5"), "got: {rendered}");
}
```

- [ ] **Step 2: Register four new integration modules**

Edit `crates/validator/tests/integration/main.rs`. Add these `mod` declarations to the list:

```rust
mod described_decorator;
mod message_template;
mod unknown_variant_error;
mod wire_format_compact;
```

(The files themselves are created in Tasks 14–17.)

- [ ] **Step 3: Defer commit to Task 17 (all integration tests together)**

---

## Task 14: Integration test — `wire_format_compact.rs`

**Files:**
- Create: `crates/validator/tests/integration/wire_format_compact.rs`

**Rationale:** Golden test for every variant's new wire shape. Acts as the permanent encoding contract.

- [ ] **Step 1: Write the file**

Create `crates/validator/tests/integration/wire_format_compact.rs`:

```rust
//! Scenario: wire format is externally-tagged tuple-compact — one entry
//! per outer variant proves the encoding contract.

use nebula_validator::{DeferredRule, Logic, Predicate, Rule, ValueRule};
use serde_json::json;

fn golden(rule: Rule, expected: serde_json::Value) {
    let encoded = serde_json::to_value(&rule).unwrap();
    assert_eq!(encoded, expected, "encode mismatch for {rule:?}");
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule, "roundtrip mismatch for {rule:?}");
}

#[test]
fn golden_min_length() {
    golden(Rule::min_length(3), json!({"min_length": 3}));
}

#[test]
fn golden_max_length() {
    golden(Rule::max_length(20), json!({"max_length": 20}));
}

#[test]
fn golden_pattern() {
    golden(Rule::pattern("^[a-z]+$"), json!({"pattern": "^[a-z]+$"}));
}

#[test]
fn golden_min_int() {
    golden(Rule::min_value(10), json!({"min": 10}));
}

#[test]
fn golden_one_of() {
    golden(
        Rule::one_of(["a", "b"]),
        json!({"one_of": ["a", "b"]}),
    );
}

#[test]
fn golden_email_unit() {
    golden(Rule::email(), json!("email"));
}

#[test]
fn golden_url_unit() {
    golden(Rule::url(), json!("url"));
}

#[test]
fn golden_predicate_eq() {
    golden(
        Rule::predicate(Predicate::eq("status", json!("active")).unwrap()),
        json!({"eq": ["/status", "active"]}),
    );
}

#[test]
fn golden_predicate_is_true() {
    use nebula_validator::foundation::FieldPath;
    golden(
        Rule::predicate(Predicate::IsTrue(FieldPath::parse("enabled").unwrap())),
        json!({"is_true": "/enabled"}),
    );
}

#[test]
fn golden_logic_all() {
    golden(
        Rule::all([Rule::min_length(3), Rule::email()]),
        json!({"all": [{"min_length": 3}, "email"]}),
    );
}

#[test]
fn golden_logic_not() {
    golden(
        Rule::not(Rule::email()),
        json!({"not": "email"}),
    );
}

#[test]
fn golden_deferred_custom() {
    golden(
        Rule::custom("check_password()"),
        json!({"custom": "check_password()"}),
    );
}

#[test]
fn golden_deferred_unique_by() {
    golden(
        Rule::unique_by("name").unwrap(),
        json!({"unique_by": "/name"}),
    );
}

#[test]
fn golden_described() {
    golden(
        Rule::min_length(3).with_message("too short"),
        json!({"described": [{"min_length": 3}, "too short"]}),
    );
}

#[test]
fn golden_nested_described() {
    golden(
        Rule::email().with_message("bad").with_message("worse"),
        json!({"described": [{"described": ["email", "bad"]}, "worse"]}),
    );
}

#[test]
fn sample_compound_rule_is_compact() {
    let rules = vec![Rule::min_length(3), Rule::max_length(100), Rule::email()];
    let encoded = serde_json::to_string(&rules).unwrap();
    // Before: ~115 chars; after: ≤55 (actual: 45 per spec §2.2)
    assert!(
        encoded.len() <= 60,
        "compound rule grew: {} chars — {}",
        encoded.len(),
        encoded
    );
    // Sanity: ensure it's the expected compact form
    assert_eq!(
        encoded,
        r#"[{"min_length":3},{"max_length":100},"email"]"#
    );
}
```

- [ ] **Step 2: Defer commit to Task 17**

---

## Task 15: Integration test — `described_decorator.rs`

**Files:**
- Create: `crates/validator/tests/integration/described_decorator.rs`

- [ ] **Step 1: Write the file**

Create `crates/validator/tests/integration/described_decorator.rs`:

```rust
//! Scenario: `Described(Box<Rule>, String)` wraps any Rule (including
//! nested Described and Logic), overrides the resulting message, and
//! preserves the error's code and field context.

use nebula_validator::{foundation::Validate, Rule};
use serde_json::json;

#[test]
fn described_overrides_leaf_message() {
    let rule = Rule::min_length(3).with_message("too short");
    let err = rule.validate(&json!("ab")).unwrap_err();
    assert_eq!(err.message.as_ref(), "too short");
    assert_eq!(err.code.as_ref(), "min_length");
}

#[test]
fn described_wraps_combinator() {
    let rule =
        Rule::all([Rule::min_length(3), Rule::pattern("^[a-z]+$")]).with_message("combined fail");
    let err = rule.validate(&json!("A")).unwrap_err();
    assert_eq!(err.message.as_ref(), "combined fail");
}

#[test]
fn outer_described_wins_over_inner() {
    let inner = Rule::min_length(3).with_message("inner");
    let outer = inner.with_message("outer");
    let err = outer.validate(&json!("a")).unwrap_err();
    assert_eq!(err.message.as_ref(), "outer");
}

#[test]
fn described_does_not_change_passing_rule() {
    let rule = Rule::min_length(3).with_message("err text");
    assert!(rule.validate(&json!("hello")).is_ok());
}

#[test]
fn described_kind_follows_inner() {
    use nebula_validator::RuleKind;
    let r = Rule::email().with_message("x");
    assert_eq!(r.kind(), RuleKind::Value);
}
```

- [ ] **Step 2: Defer commit to Task 17**

---

## Task 16: Integration test — `message_template.rs` + `unknown_variant_error.rs`

**Files:**
- Create: `crates/validator/tests/integration/message_template.rs`
- Create: `crates/validator/tests/integration/unknown_variant_error.rs`

- [ ] **Step 1: Write message_template.rs**

Create `crates/validator/tests/integration/message_template.rs`:

```rust
//! Scenario: `{name}` placeholders in `Described` messages render from
//! ValidationError params at Display time.

use nebula_validator::{foundation::Validate, Rule};
use serde_json::json;

#[test]
fn min_placeholder_renders() {
    let rule = Rule::min_length(3).with_message("need at least {min} chars");
    let err = rule.validate(&json!("a")).unwrap_err();
    let rendered = format!("{err}");
    assert!(rendered.contains("need at least 3 chars"), "got: {rendered}");
}

#[test]
fn multiple_placeholders() {
    let rule = Rule::min_length(3).with_message("got {value}, need {min}");
    let err = rule.validate(&json!("hi")).unwrap_err();
    let rendered = format!("{err}");
    assert!(rendered.contains("got \"hi\""), "got: {rendered}");
    assert!(rendered.contains("need 3"), "got: {rendered}");
}

#[test]
fn pattern_placeholder() {
    let rule = Rule::pattern("^[0-9]+$").with_message("does not match {pattern}");
    let err = rule.validate(&json!("abc")).unwrap_err();
    let rendered = format!("{err}");
    assert!(rendered.contains("^[0-9]+$"), "got: {rendered}");
}

#[test]
fn unknown_placeholder_left_literal() {
    let rule = Rule::min_length(3).with_message("value is {mystery_field}");
    let err = rule.validate(&json!("a")).unwrap_err();
    let rendered = format!("{err}");
    assert!(rendered.contains("{mystery_field}"), "got: {rendered}");
}

#[test]
fn escape_double_brace() {
    let rule = Rule::min_length(3).with_message("needs {{}} brackets");
    let err = rule.validate(&json!("a")).unwrap_err();
    let rendered = format!("{err}");
    assert!(rendered.contains("needs {} brackets"), "got: {rendered}");
}
```

- [ ] **Step 2: Write unknown_variant_error.rs**

Create `crates/validator/tests/integration/unknown_variant_error.rs`:

```rust
//! Scenario: malformed JSON produces a descriptive error via our manual
//! Deserialize impl — not serde's generic "data did not match any variant".

use nebula_validator::Rule;
use serde_json::json;

#[test]
fn unknown_rule_name_lists_alternatives() {
    let result: Result<Rule, _> = serde_json::from_value(json!({"min_lenght": 3}));
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("unknown rule"), "got: {msg}");
    assert!(msg.contains("min_length"), "got: {msg}");
    assert!(msg.contains("Known rules:"), "got: {msg}");
}

#[test]
fn empty_object_rejected() {
    let result: Result<Rule, _> = serde_json::from_value(json!({}));
    assert!(result.is_err());
}

#[test]
fn unknown_unit_string_rejected() {
    let result: Result<Rule, _> = serde_json::from_value(json!("not_a_rule"));
    let err = result.unwrap_err();
    assert!(err.to_string().contains("unknown"));
}

#[test]
fn multi_key_object_rejected() {
    let result: Result<Rule, _> =
        serde_json::from_value(json!({"min_length": 3, "max_length": 10}));
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("exactly one key") || err.to_string().contains("extra key"),
        "got: {err}"
    );
}
```

- [ ] **Step 3: Defer commit to Task 17**

---

## Task 17: Compile-fail test + bench update + commit integration tests

**Files:**
- Create: `crates/validator/tests/ui/typed_narrowing.rs`
- Create: `crates/validator/tests/ui/typed_narrowing.stderr` (captured via trybuild)
- Modify: `crates/validator/benches/rule_engine.rs`
- Modify: `crates/validator/tests/ui.rs` (to register the new case if needed)

**Rationale:** Prove at compile time that calling `validate_value` on a `Predicate` fails. Update the `rule_engine` benchmark to use new constructors.

- [ ] **Step 1: Check existing trybuild harness**

Run: `cat crates/validator/tests/ui.rs`

The harness should discover `tests/ui/*.rs` files. Confirm that pattern.

- [ ] **Step 2: Write the compile-fail case**

Create `crates/validator/tests/ui/typed_narrowing.rs`:

```rust
//! Typed narrowing: validate_value belongs to ValueRule, not Predicate.

use nebula_validator::{foundation::FieldPath, Predicate};
use serde_json::json;

fn main() {
    let p = Predicate::Eq(FieldPath::parse("x").unwrap(), json!(1));
    // Should fail: Predicate does not implement validate_value.
    let _ = p.validate_value(&json!(1));
}
```

- [ ] **Step 3: Generate the expected stderr**

```bash
TRYBUILD=overwrite cargo test -p nebula-validator --test ui -- typed_narrowing
```

This creates `crates/validator/tests/ui/typed_narrowing.stderr`. Inspect it to confirm the error message points to the missing `validate_value` method on `Predicate`:

```bash
cat crates/validator/tests/ui/typed_narrowing.stderr
```

- [ ] **Step 4: Update rule_engine benchmark**

Open `crates/validator/benches/rule_engine.rs`. Replace each old constructor call with the new API. Example substitutions:

| Old | New |
|---|---|
| `Rule::MinLength { min: 3, message: None }` | `Rule::min_length(3)` |
| `Rule::MaxLength { max: 10, message: None }` | `Rule::max_length(10)` |
| `Rule::Pattern { pattern: "...".into(), message: None }` | `Rule::pattern("...")` |
| `Rule::Custom { expression: "...".into(), message: None }` | `Rule::custom("...")` |
| `Rule::UniqueBy { key: "id".into(), message: None }` | `Rule::unique_by("id").unwrap()` |
| `Rule::All { rules: vec![...] }` | `Rule::all([...])` |
| `Rule::Any { rules: vec![...] }` | `Rule::any([...])` |
| `Rule::Not { inner: Box::new(...) }` | `Rule::not(...)` |

- [ ] **Step 5: Verify everything compiles and tests pass**

Run: `cargo nextest run -p nebula-validator`
Expected: all tests pass. Count new tests:

```bash
cargo nextest run -p nebula-validator --list 2>&1 | grep -c 'described_decorator\|message_template\|wire_format_compact\|unknown_variant_error'
```
Expected: ≥25.

Run the compile-fail UI test:
```bash
cargo test -p nebula-validator --test ui
```
Expected: PASS (trybuild confirms the expected stderr matches).

- [ ] **Step 6: Commit the integration test suite**

```bash
git add crates/validator/tests/integration/ crates/validator/tests/ui/ crates/validator/benches/rule_engine.rs
git commit -m "$(cat <<'EOF'
test(validator): wire-format + described + template + compile-fail suite

- wire_format_compact: golden roundtrip for every variant, byte-size
  regression guard on sample compound rule.
- described_decorator: Described overrides leaf + combinator messages;
  outer wraps inner.
- message_template: {min}/{value}/{pattern} substitution, unknown
  placeholder left literal, {{/}} escape.
- unknown_variant_error: friendly "unknown rule X. Known rules: ..."
  from manual Deserialize; empty/multi-key/unknown-unit rejected.
- ui/typed_narrowing: compile-fail proves validate_value is not
  reachable on Predicate (the point of the refactor).
- benches/rule_engine: constructor call migration.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 18: Update `prelude.rs` + doctest sweep

**Files:**
- Modify: `crates/validator/src/prelude.rs`
- Modify: `crates/validator/src/lib.rs` (module docs + doctests)
- Modify: `crates/validator/src/rule/mod.rs` (module docs)

**Rationale:** Doctests and prelude still reference the old Rule shape.

- [ ] **Step 1: Update prelude**

In `crates/validator/src/prelude.rs`, replace the line:

```rust
rule::Rule,
```

with:

```rust
rule::{DeferredRule, Logic, Predicate, PredicateContext, Rule, RuleKind, ValueRule},
```

- [ ] **Step 2: Update lib.rs doctests**

In `crates/validator/src/lib.rs`, replace the existing `## Declarative Rules` doctest block. Old doctest (lines ~40–73):

```rust
//! let rule = Rule::MinLength { min: 3, message: None };
//! ...
//! let rule = Rule::Eq { field: "status".into(), value: json!("active") };
//! let ctx: std::collections::HashMap<String, serde_json::Value> = ...;
```

New:

```rust
//! use nebula_validator::{ExecutionMode, Predicate, Rule, PredicateContext, validate_rules};
//! use serde_json::json;
//!
//! // Value validation — checks a single JSON value
//! let rule = Rule::min_length(3);
//! assert!(validate_rules(&json!("alice"), &[rule.clone()], ExecutionMode::StaticOnly).is_ok());
//!
//! // Context predicate — checks a sibling field
//! let rule = Rule::predicate(Predicate::eq("status", json!("active")).unwrap());
//! let ctx = PredicateContext::from_json(&json!({"status": "active"}));
//! // Predicates pass silently without ctx; with ctx they evaluate:
//! assert!(rule.validate(&json!("any"), Some(&ctx), ExecutionMode::StaticOnly).is_ok());
//!
//! // Logical combinator
//! let rule = Rule::all([Rule::min_length(3), Rule::max_length(20)]);
//! assert!(validate_rules(&json!("hello"), &[rule], ExecutionMode::StaticOnly).is_ok());
```

Also find and update the `| Type | Purpose |` table row for `Rule` — the description stays but any inline examples need new syntax.

- [ ] **Step 3: Update rule/mod.rs module-level docs**

Replace the `//! # Rule Categories` table and examples. Use this as the new module docs (already written in Task 9 Step 1 — verify it's what shipped):

```rust
//! Unified declarative rule system — sum-of-sums over typed kinds.
//!
//! [`Rule`] is a classifier over four semantic kinds. See the type's
//! docs for the full table.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::{foundation::Validate, Rule};
//! use serde_json::json;
//!
//! let rule = Rule::min_length(3);
//! assert!(rule.validate(&json!("alice")).is_ok());
//! assert!(rule.validate(&json!("ab")).is_err());
//! ```
```

- [ ] **Step 4: Run doctests**

Run: `cargo test -p nebula-validator --doc`
Expected: all doctests pass. If any fail, inspect the failure and update the doctest body.

- [ ] **Step 5: Commit**

```bash
git add crates/validator/src/prelude.rs crates/validator/src/lib.rs crates/validator/src/rule/mod.rs
git commit -m "$(cat <<'EOF'
docs(validator): update prelude and doctests for new Rule shape

Prelude re-exports the new sub-enums; lib.rs Quick-Start shows
the new constructor API; rule/mod.rs docs drop the old-shape
Rule::MinLength{} examples.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 19: Final sweep — full workspace validation + CHANGELOG

**Files:**
- Modify: `CHANGELOG.md`
- All touched files (validation only)

- [ ] **Step 1: Run the full pre-PR gate**

```bash
cargo +nightly fmt --all && \
cargo clippy --workspace -- -D warnings && \
cargo nextest run --workspace && \
cargo test --workspace --doc && \
cargo deny check
```

All five commands must succeed. Record any warning and fix in-place (`cargo +nightly fmt --all` is the first line; if clippy warns, fix before moving on).

- [ ] **Step 2: Add CHANGELOG entry**

Edit `CHANGELOG.md`. Under `## [Unreleased]` → `### Changed` (add the section if missing), insert:

```markdown
- **nebula-validator**: **Breaking** — replaced flat 30-variant `Rule` enum
  with a typed sum-of-sums: `Rule::{Value(ValueRule), Predicate(Predicate),
  Logic(Box<Logic>), Deferred(DeferredRule), Described(Box<Rule>, String)}`.
  Each kind has a single method that makes sense for it (`validate_value`
  on `ValueRule`, `evaluate` on `Predicate`, etc.). Cross-kind silent-pass
  is gone (calling `validate_value` on a `Predicate` no longer compiles).
  Predicates now carry `FieldPath` instead of raw `String` — paths
  validated at construction. See
  `docs/superpowers/specs/2026-04-17-nebula-validator-rule-refactor-design.md`.
- **nebula-validator**: **Breaking wire format** — externally-tagged
  tuple-compact encoding for `Rule`: `{"min_length":3}`, `{"eq":["/path",value]}`,
  `"email"` for unit variants. ~60% shorter than the old `{"rule":"min_length","min":3}`.
  Manual `Deserialize` produces friendly `unknown rule "X". Known rules: ...`
  errors instead of serde's generic "data did not match any variant".
- **nebula-validator**: `Described(Box<Rule>, String)` decorator replaces
  per-variant `message: Option<String>` fields and now works across
  combinators (not just leaf rules). Messages can contain `{placeholder}`
  templates that render from the error's params at `Display` time; zero
  allocation for plain messages.
- **nebula-validator**: `FieldPath` now implements `Serialize`/`Deserialize`
  (wire form is the inner JSON Pointer string).
- **nebula-validator**: `PredicateContext` typed newtype replaces raw
  `HashMap<String, Value>` for predicate evaluation; auto-flattens nested
  JSON objects into `/path` keys.
- **nebula-schema**: consumer updated for new `Rule` shape — `lint.rs`
  classification and `field.rs` builder calls migrated.
```

- [ ] **Step 3: Commit CHANGELOG**

```bash
git add CHANGELOG.md
git commit -m "$(cat <<'EOF'
docs(changelog): record Rule type split refactor

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 4: Final status check**

```bash
git log --oneline origin/main..HEAD
```

Expected: 8–10 commits on the branch. Review the series to confirm coherent story:

1. `feat(validator): add Serialize/Deserialize for FieldPath`
2. `feat(validator): named-placeholder templates in ValidationError::Display`
3. `feat(validator): add ValueRule sub-enum (not yet wired)`
4. `feat(validator): add Predicate sub-enum + PredicateContext placeholder`
5. `feat(validator): add Logic sub-enum (unwired pending Rule rewrite)`
6. `feat(validator): add DeferredRule sub-enum (unwired)`
7. `feat(validator): full PredicateContext impl with nested-path flattening`
8. `refactor(validator): split flat Rule enum into typed sum-of-sums`
9. `test(validator): wire-format + described + template + compile-fail suite`
10. `docs(validator): update prelude and doctests for new Rule shape`
11. `docs(changelog): record Rule type split refactor`

- [ ] **Step 5: Ready for PR**

Follow the commit-push-pr skill (`commit-commands:commit-push-pr`) or the canonical flow:

```bash
git push -u origin claude/hardcore-meitner-f85782
gh pr create --title "refactor(validator): split flat Rule into typed sum-of-sums" \
   --body "$(cat <<'EOF'
## Summary
- Replaces 30-variant flat `Rule` enum with `Rule::{Value, Predicate, Logic, Deferred, Described}` delegating to typed sub-enums
- Adopts externally-tagged tuple-compact wire format (~60% smaller on compound rules)
- `FieldPath` on predicates (was `String`), typed `PredicateContext`
- Message templates via `{placeholder}` in `Described` messages
- Unlocks `nebula-schema` Phase 2 DX work (blocked on Rule shape)

Spec: `docs/superpowers/specs/2026-04-17-nebula-validator-rule-refactor-design.md`

## Test plan
- [ ] `cargo +nightly fmt --all` green
- [ ] `cargo clippy --workspace -- -D warnings` green
- [ ] `cargo nextest run --workspace` green
- [ ] `cargo test --workspace --doc` green
- [ ] `cargo deny check` green
- [ ] trybuild UI test `typed_narrowing` confirms Predicate::validate_value doesn't compile
- [ ] Sample compound rule encoding ≤ 60 chars (was ~115)

Breaking: wire format, pattern-match sites. No stored data; alpha stage.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Acceptance checklist

- [ ] All 19 tasks complete, each with its own commit
- [ ] `cargo +nightly fmt --all` green
- [ ] `cargo clippy --workspace -- -D warnings` green
- [ ] `cargo nextest run --workspace` green
- [ ] `cargo test --workspace --doc` green
- [ ] `cargo deny check` green
- [ ] `trybuild` UI test proves `Predicate::validate_value` fails to compile
- [ ] Compound rule byte size regression test asserts ≤60 chars
- [ ] Golden wire-format tests cover every variant kind
- [ ] `Described` wraps value rules, predicates, combinators, and nested Described
- [ ] Message templates render `{min}` / `{max}` / `{pattern}` / `{value}` / `{allowed}` correctly
- [ ] Malformed JSON produces `unknown rule "X". Known rules: ...` error
- [ ] `crates/schema` compiles and tests pass unchanged
- [ ] CHANGELOG updated with breaking-change note
