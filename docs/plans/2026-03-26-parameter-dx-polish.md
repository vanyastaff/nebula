# Parameter DX Polish — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Поднять DX nebula-parameter с 4/5 до 5/5, устранив все выявленные проблемы: silent no-ops, недостающие конструкторы, недостающие batch-операции, тонкий ValidatedValues, doctests.

**Architecture:** Все изменения внутри `crates/parameter/`. Никаких breaking changes для downstream (action, auth, credential, engine, sdk). ParameterType не используется ни в одном внешнем крейте через match — `#[non_exhaustive]` безопасен.

**Tech Stack:** Rust 1.93, serde, regex, nebula-validator (re-export Rule)

---

### Task 1: debug_assert! макрос для type-specific builder no-ops

**Files:**
- Modify: `crates/parameter/src/parameter.rs:531-923`

**Step 1: Write the failing test**

В `crates/parameter/src/parameter.rs`, в mod tests, добавить тест:

```rust
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "multiline()")]
fn multiline_on_wrong_type_panics_in_debug() {
    Parameter::number("x").multiline();
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "searchable()")]
fn searchable_on_wrong_type_panics_in_debug() {
    Parameter::string("x").searchable();
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "option()")]
fn option_on_wrong_type_panics_in_debug() {
    Parameter::boolean("x").option("v", "label");
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "min()")]
fn min_on_wrong_type_panics_in_debug() {
    Parameter::string("x").min(0.0);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "collapsed()")]
fn collapsed_on_wrong_type_panics_in_debug() {
    Parameter::string("x").collapsed();
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "variant()")]
fn variant_on_wrong_type_panics_in_debug() {
    Parameter::string("x").variant(Parameter::string("y"));
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "min_items()")]
fn min_items_on_wrong_type_panics_in_debug() {
    Parameter::string("x").min_items(1);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "accept()")]
fn accept_on_wrong_type_panics_in_debug() {
    Parameter::string("x").accept("image/*");
}
```

**Step 2: Run test to verify it fails**

Run: `rtk cargo nextest run -p nebula-parameter multiline_on_wrong_type`
Expected: FAIL — currently no panic

**Step 3: Add debug_assert! macro and apply to all ~30 type-specific builders**

At the top of the type-specific builders section (line ~531), add a private macro:

```rust
/// Asserts in debug builds that the parameter type matches the expected variant.
/// Zero cost in release builds. Catches misuse during development.
macro_rules! debug_assert_type {
    ($self:expr, $variant:pat, $method:literal) => {
        debug_assert!(
            matches!($self.param_type, $variant),
            concat!(
                $method,
                "() called on {:?} — only valid for ",
                stringify!($variant)
            ),
            std::mem::discriminant(&$self.param_type)
        );
    };
}
```

Then add `debug_assert_type!` to every type-specific method. Pattern for each:

```rust
pub fn multiline(mut self) -> Self {
    if let ParameterType::String { multiline, .. } = &mut self.param_type {
        *multiline = true;
    } else {
        debug_assert_type!(self, ParameterType::String { .. }, "multiline");
    }
    self
}
```

Apply to ALL type-specific builders (complete list):
- String: `multiline`
- Number: `min`, `max`, `step`, `min_i64`, `max_i64`, `step_i64`
- Select: `option`, `option_with`, `allow_custom`, `searchable`
- Select+File: `multiple` (no assert — two types valid)
- Select+Filter+Dynamic: `depends_on` (no assert — three types valid)
- Object: `add`, `collapsed`, `pick_fields`, `sections`
- List: `min_items`, `max_items`, `unique`, `sortable`
- Mode: `variant`, `default_variant`
- File: `accept`, `max_size`
- Filter: `operators`, `allow_groups`, `max_depth`, `filter_field`, `dynamic_fields`
- Computed: `returns_string`, `returns_number`, `returns_boolean`
- Loader setters: `with_option_loader`, `with_record_loader`, `with_filter_field_loader`

For methods that handle multiple types (`multiple`, `depends_on`), use a combined pattern:

```rust
pub fn multiple(mut self) -> Self {
    match &mut self.param_type {
        ParameterType::Select { multiple, .. } | ParameterType::File { multiple, .. } => {
            *multiple = true;
        }
        _ => {
            debug_assert!(
                false,
                "multiple() called on {:?} — only valid for Select or File",
                std::mem::discriminant(&self.param_type)
            );
        }
    }
    self
}
```

**Step 4: Fix the existing test `multiline_only_affects_string`**

The existing test at line 1081 verifies the no-op behavior. It needs `#[cfg(not(debug_assertions))]`:

```rust
#[test]
#[cfg(not(debug_assertions))]
fn multiline_only_affects_string_in_release() {
    let n = Parameter::number("num").multiline();
    assert!(matches!(n.param_type, ParameterType::Number { .. }));
}
```

**Step 5: Run tests to verify all pass**

Run: `rtk cargo nextest run -p nebula-parameter`
Expected: PASS

**Step 6: Commit**

```bash
rtk git add crates/parameter/src/parameter.rs
rtk git commit -m "feat(parameter): add debug_assert! to type-specific builder methods

Catches silent no-ops (e.g. .searchable() on String) during development.
Zero cost in release builds."
```

---

### Task 2: Condition shorthand constructors (gt, lt, is_true)

**Files:**
- Modify: `crates/parameter/src/conditions.rs:121-186`

**Step 1: Write the failing tests**

Add to `mod tests` in conditions.rs:

```rust
#[test]
fn gt_shorthand_constructor() {
    let cond = Condition::gt("count", 5);
    let vals = values(&[("count", serde_json::json!(10))]);
    assert!(cond.evaluate(&vals));
    let below = values(&[("count", serde_json::json!(3))]);
    assert!(!cond.evaluate(&below));
}

#[test]
fn lt_shorthand_constructor() {
    let cond = Condition::lt("count", 5);
    let vals = values(&[("count", serde_json::json!(3))]);
    assert!(cond.evaluate(&vals));
    let above = values(&[("count", serde_json::json!(10))]);
    assert!(!cond.evaluate(&above));
}

#[test]
fn is_true_shorthand_constructor() {
    let cond = Condition::is_true("enabled");
    let vals = values(&[("enabled", Value::Bool(true))]);
    assert!(cond.evaluate(&vals));
    let vals_false = values(&[("enabled", Value::Bool(false))]);
    assert!(!cond.evaluate(&vals_false));
}
```

**Step 2: Run test to verify it fails**

Run: `rtk cargo nextest run -p nebula-parameter gt_shorthand`
Expected: FAIL — method doesn't exist

**Step 3: Add the constructors**

After `Condition::not` (line 186), add:

```rust
/// Create a [`Gt`](Self::Gt) condition (numeric greater-than).
#[must_use]
pub fn gt(field: impl Into<ParameterPath>, value: impl Into<Value>) -> Self {
    Self::Gt {
        field: field.into(),
        value: value.into(),
    }
}

/// Create a [`Lt`](Self::Lt) condition (numeric less-than).
#[must_use]
pub fn lt(field: impl Into<ParameterPath>, value: impl Into<Value>) -> Self {
    Self::Lt {
        field: field.into(),
        value: value.into(),
    }
}

/// Create an [`IsTrue`](Self::IsTrue) condition.
#[must_use]
pub fn is_true(field: impl Into<ParameterPath>) -> Self {
    Self::IsTrue {
        field: field.into(),
    }
}
```

**Step 4: Update existing tests to use the new constructors**

Replace struct literal usage in existing tests (lines 368-395):

```rust
#[test]
fn is_true_matches_boolean_true() {
    let cond = Condition::is_true("enabled");
    let vals = values(&[("enabled", Value::Bool(true))]);
    assert!(cond.evaluate(&vals));
}

#[test]
fn is_true_rejects_false() {
    let cond = Condition::is_true("enabled");
    let vals = values(&[("enabled", Value::Bool(false))]);
    assert!(!cond.evaluate(&vals));
}

#[test]
fn gt_compares_numerically() {
    let cond = Condition::gt("count", 5);
    let above = values(&[("count", serde_json::json!(10))]);
    let below = values(&[("count", serde_json::json!(3))]);
    assert!(cond.evaluate(&above));
    assert!(!cond.evaluate(&below));
}

#[test]
fn lt_compares_numerically() {
    let cond = Condition::lt("count", 5);
    let below = values(&[("count", serde_json::json!(3))]);
    let above = values(&[("count", serde_json::json!(10))]);
    assert!(cond.evaluate(&below));
    assert!(!cond.evaluate(&above));
}
```

Also update `field_references_collects_all_leaves` to use `Condition::is_true("c")` instead of struct literal.

**Step 5: Run tests**

Run: `rtk cargo nextest run -p nebula-parameter`
Expected: PASS

**Step 6: Commit**

```bash
rtk git add crates/parameter/src/conditions.rs
rtk git commit -m "feat(parameter): add Condition::gt(), lt(), is_true() shorthand constructors"
```

---

### Task 3: Condition::one_of ergonomics

**Files:**
- Modify: `crates/parameter/src/conditions.rs:143-149`

**Step 1: Write the failing test**

```rust
#[test]
fn one_of_accepts_string_slices() {
    // Should compile without json!() wrappers
    let cond = Condition::one_of("color", ["red", "blue"]);
    let vals = values(&[("color", Value::String("blue".into()))]);
    assert!(cond.evaluate(&vals));
}

#[test]
fn one_of_accepts_mixed_into_value() {
    let cond = Condition::one_of("count", [1, 2, 3]);
    let vals = values(&[("count", serde_json::json!(2))]);
    assert!(cond.evaluate(&vals));
}
```

**Step 2: Run test to verify it fails**

Run: `rtk cargo nextest run -p nebula-parameter one_of_accepts_string`
Expected: FAIL — type mismatch

**Step 3: Change signature**

Replace the `one_of` constructor:

```rust
/// Create a [`OneOf`](Self::OneOf) condition.
#[must_use]
pub fn one_of<V: Into<Value>>(
    field: impl Into<ParameterPath>,
    values: impl IntoIterator<Item = V>,
) -> Self {
    Self::OneOf {
        field: field.into(),
        values: values.into_iter().map(Into::into).collect(),
    }
}
```

**Step 4: Verify existing callers still compile**

Existing code passes `Vec<Value>` which implements `IntoIterator<Item = Value>`, so this is backward compatible.

**Step 5: Run tests**

Run: `rtk cargo nextest run -p nebula-parameter`
Expected: PASS

**Step 6: Commit**

```bash
rtk git add crates/parameter/src/conditions.rs
rtk git commit -m "feat(parameter): make Condition::one_of accept IntoIterator<Item: Into<Value>>"
```

---

### Task 4: ParameterCollection::extend and FromIterator

**Files:**
- Modify: `crates/parameter/src/collection.rs`

**Step 1: Write the failing tests**

Add to `mod tests` in collection.rs:

```rust
#[test]
fn extend_adds_multiple_parameters() {
    let base = ParameterCollection::new()
        .add(Parameter::string("name"));

    let extra = vec![
        Parameter::integer("age"),
        Parameter::boolean("active"),
    ];

    let coll = base.extend(extra);
    assert_eq!(coll.len(), 3);
    assert!(coll.contains("name"));
    assert!(coll.contains("age"));
    assert!(coll.contains("active"));
}

#[test]
fn from_iterator_builds_collection() {
    let params = vec![
        Parameter::string("host"),
        Parameter::integer("port"),
    ];

    let coll: ParameterCollection = params.into_iter().collect();
    assert_eq!(coll.len(), 2);
    assert!(coll.contains("host"));
}

#[test]
fn iter_yields_parameters() {
    let coll = ParameterCollection::new()
        .add(Parameter::string("a"))
        .add(Parameter::string("b"));

    let ids: Vec<&str> = coll.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b"]);
}
```

**Step 2: Run test to verify it fails**

Run: `rtk cargo nextest run -p nebula-parameter extend_adds`
Expected: FAIL — method doesn't exist

**Step 3: Add the methods**

In `collection.rs`, add to `impl ParameterCollection`:

```rust
/// Appends all parameters from an iterator.
#[must_use]
pub fn extend(mut self, params: impl IntoIterator<Item = Parameter>) -> Self {
    self.parameters.extend(params);
    self
}

/// Returns an iterator over the parameters.
pub fn iter(&self) -> std::slice::Iter<'_, Parameter> {
    self.parameters.iter()
}
```

Add `FromIterator` impl after the `impl ParameterCollection` block:

```rust
impl FromIterator<Parameter> for ParameterCollection {
    fn from_iter<I: IntoIterator<Item = Parameter>>(iter: I) -> Self {
        Self {
            parameters: iter.into_iter().collect(),
        }
    }
}

impl<'a> IntoIterator for &'a ParameterCollection {
    type Item = &'a Parameter;
    type IntoIter = std::slice::Iter<'a, Parameter>;

    fn into_iter(self) -> Self::IntoIter {
        self.parameters.iter()
    }
}

impl IntoIterator for ParameterCollection {
    type Item = Parameter;
    type IntoIter = std::vec::IntoIter<Parameter>;

    fn into_iter(self) -> Self::IntoIter {
        self.parameters.into_iter()
    }
}
```

**Step 4: Run tests**

Run: `rtk cargo nextest run -p nebula-parameter`
Expected: PASS

**Step 5: Commit**

```bash
rtk git add crates/parameter/src/collection.rs
rtk git commit -m "feat(parameter): add extend(), iter(), FromIterator, IntoIterator to ParameterCollection"
```

---

### Task 5: ValidatedValues accessor delegation

**Files:**
- Modify: `crates/parameter/src/runtime.rs`

**Step 1: Write the failing tests**

Add a test block in `runtime.rs` (currently no tests module — create one):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_validated(pairs: &[(&str, serde_json::Value)]) -> ValidatedValues {
        let mut values = ParameterValues::new();
        for (k, v) in pairs {
            values.set(*k, v.clone());
        }
        ValidatedValues::new(values)
    }

    #[test]
    fn get_delegates_to_inner() {
        let v = make_validated(&[("host", json!("localhost"))]);
        assert_eq!(v.get("host"), Some(&json!("localhost")));
        assert_eq!(v.get("missing"), None);
    }

    #[test]
    fn get_string_delegates() {
        let v = make_validated(&[("name", json!("Alice"))]);
        assert_eq!(v.get_string("name"), Some("Alice"));
    }

    #[test]
    fn get_bool_delegates() {
        let v = make_validated(&[("active", json!(true))]);
        assert_eq!(v.get_bool("active"), Some(true));
    }

    #[test]
    fn get_f64_delegates() {
        let v = make_validated(&[("score", json!(42.5))]);
        assert_eq!(v.get_f64("score"), Some(42.5));
    }

    #[test]
    fn get_i64_delegates() {
        let v = make_validated(&[("port", json!(8080))]);
        assert_eq!(v.get_i64("port"), Some(8080));
    }

    #[test]
    fn contains_delegates() {
        let v = make_validated(&[("key", json!("val"))]);
        assert!(v.contains("key"));
        assert!(!v.contains("other"));
    }

    #[test]
    fn len_delegates() {
        let v = make_validated(&[("a", json!(1)), ("b", json!(2))]);
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn is_empty_delegates() {
        let v = make_validated(&[]);
        assert!(v.is_empty());
    }

    #[test]
    fn index_delegates() {
        let v = make_validated(&[("key", json!("val"))]);
        assert_eq!(v["key"], json!("val"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `rtk cargo nextest run -p nebula-parameter get_delegates_to_inner`
Expected: FAIL — method doesn't exist on ValidatedValues

**Step 3: Add delegation methods**

In `runtime.rs`, add to `impl ValidatedValues`:

```rust
/// Returns the value for `key`, if present.
#[must_use]
pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
    self.values.get(key)
}

/// Returns the value as a string, if it is one.
#[must_use]
pub fn get_string(&self, key: &str) -> Option<&str> {
    self.values.get_string(key)
}

/// Returns the value as a bool, if it is one.
#[must_use]
pub fn get_bool(&self, key: &str) -> Option<bool> {
    self.values.get_bool(key)
}

/// Returns the value as f64, if it is numeric.
#[must_use]
pub fn get_f64(&self, key: &str) -> Option<f64> {
    self.values.get_f64(key)
}

/// Returns the value as i64, if it is an integer.
#[must_use]
pub fn get_i64(&self, key: &str) -> Option<i64> {
    self.values.get_i64(key)
}

/// Returns the value as an array slice, if it is one.
#[must_use]
pub fn get_array(&self, key: &str) -> Option<&[serde_json::Value]> {
    self.values.get_array(key)
}

/// Returns the value as a JSON object, if it is one.
#[must_use]
pub fn get_object(&self, key: &str) -> Option<&serde_json::Map<String, serde_json::Value>> {
    self.values.get_object(key)
}

/// Returns the mode selection details, if the value is mode-based.
#[must_use]
pub fn get_mode(&self, key: &str) -> Option<ModeValueRef<'_>> {
    self.values.get_mode(key)
}

/// Checks whether a value exists for `key`.
#[must_use]
pub fn contains(&self, key: &str) -> bool {
    self.values.contains(key)
}

/// Returns the number of values.
#[must_use]
pub fn len(&self) -> usize {
    self.values.len()
}

/// Returns `true` if there are no values.
#[must_use]
pub fn is_empty(&self) -> bool {
    self.values.is_empty()
}
```

Add `Index` impl:

```rust
impl std::ops::Index<&str> for ValidatedValues {
    type Output = serde_json::Value;

    fn index(&self, key: &str) -> &Self::Output {
        &self.values[key]
    }
}
```

**Step 4: Run tests**

Run: `rtk cargo nextest run -p nebula-parameter`
Expected: PASS

**Step 5: Commit**

```bash
rtk git add crates/parameter/src/runtime.rs
rtk git commit -m "feat(parameter): delegate accessor methods on ValidatedValues"
```

---

### Task 6: lint_collection in prelude + runnable doctests

**Files:**
- Modify: `crates/parameter/src/lib.rs`
- Modify: `crates/parameter/src/collection.rs` (doctest)
- Modify: `crates/parameter/src/parameter.rs` (module-level doctest)
- Modify: `crates/parameter/src/conditions.rs` (module-level doctest)

**Step 1: Add lint re-exports**

In `lib.rs`, add to top-level re-exports:

```rust
pub use lint::{lint_collection, LintDiagnostic, LintLevel};
```

Add to prelude:

```rust
pub use crate::lint::{lint_collection, LintDiagnostic, LintLevel};
```

**Step 2: Fix doctests from `ignore` to runnable**

In `lib.rs` Quick Start, change `ignore` to working doctest:

```rust
//! ## Quick Start
//!
//! ```
//! use nebula_parameter::prelude::*;
//!
//! let params = ParameterCollection::new()
//!     .add(Parameter::string("api_key").label("API Key").required().secret())
//!     .add(Parameter::integer("timeout_ms").label("Timeout (ms)"));
//!
//! assert_eq!(params.len(), 2);
//! ```
```

In `collection.rs`, fix the doctest:

```rust
/// ```
/// use nebula_parameter::collection::ParameterCollection;
/// use nebula_parameter::parameter::Parameter;
/// use serde_json::json;
///
/// let params = ParameterCollection::new()
///     .add(Parameter::string("name").label("Name").required())
///     .add(Parameter::integer("age").label("Age").default(json!(18)));
///
/// assert_eq!(params.len(), 2);
/// assert!(params.contains("name"));
/// ```
```

In `parameter.rs`, fix the module doctest:

```rust
//! ```
//! use nebula_parameter::parameter::Parameter;
//! use nebula_parameter::conditions::Condition;
//!
//! let schema = vec![
//!     Parameter::string("api_key").label("API Key").required().secret(),
//!     Parameter::integer("timeout_ms").label("Timeout (ms)").default(serde_json::json!(30_000)),
//!     Parameter::select("region")
//!         .label("Region")
//!         .option(serde_json::json!("us-east-1"), "US East")
//!         .option(serde_json::json!("eu-west-1"), "EU West")
//!         .searchable(),
//! ];
//!
//! assert_eq!(schema.len(), 3);
//! ```
```

In `conditions.rs`, fix the module doctest:

```rust
//! ```
//! use nebula_parameter::conditions::Condition;
//!
//! let cond = Condition::all(vec![
//!     Condition::eq("auth_mode", "oauth2"),
//!     Condition::set("client_id"),
//! ]);
//! ```
```

**Step 3: Run doctests**

Run: `rtk cargo test -p nebula-parameter --doc`
Expected: PASS

**Step 4: Run all tests**

Run: `rtk cargo nextest run -p nebula-parameter`
Expected: PASS

**Step 5: Commit**

```bash
rtk git add crates/parameter/src/lib.rs crates/parameter/src/collection.rs crates/parameter/src/parameter.rs crates/parameter/src/conditions.rs
rtk git commit -m "feat(parameter): add lint to prelude, fix all doctests to be runnable"
```

---

### Task 7: Remove backward-compat deprecated aliases

**Files:**
- Modify: `crates/parameter/src/runtime.rs`
- Modify: `crates/parameter/src/values.rs`

**Step 1: Check if deprecated aliases are used anywhere**

Run: `rtk grep -r "FieldValue\b\|FieldValues\b" crates/` (excluding parameter crate itself)

If no external usage exists, remove:

From `runtime.rs`, delete:
```rust
// Backward compat
#[deprecated(note = "renamed to ParameterValue")]
pub use crate::values::ParameterValue as FieldValue;
#[deprecated(note = "renamed to ParameterValues")]
pub use crate::values::ParameterValues as FieldValues;
```

From `values.rs`, delete:
```rust
/// Backward-compatible type alias.
#[deprecated(note = "renamed to ParameterValues")]
pub type FieldValues = ParameterValues;

/// Backward-compatible type alias.
#[deprecated(note = "renamed to ParameterValue")]
pub type FieldValue = ParameterValue;
```

**Step 2: Run full workspace check**

Run: `rtk cargo check --workspace`
Expected: PASS

**Step 3: Commit**

```bash
rtk git add crates/parameter/src/runtime.rs crates/parameter/src/values.rs
rtk git commit -m "refactor(parameter): remove deprecated FieldValue/FieldValues aliases"
```

---

### Task 8: Full validation & context file update

**Step 1: Run full validation**

```bash
rtk cargo fmt && rtk cargo clippy -p nebula-parameter -- -D warnings && rtk cargo nextest run -p nebula-parameter && rtk cargo test -p nebula-parameter --doc
```

Expected: all PASS

**Step 2: Run workspace check (ensure no downstream breakage)**

```bash
rtk cargo check --workspace
```

Expected: PASS

**Step 3: Update `.claude/crates/parameter.md`**

Update the Traps section:
- Remove: "Type-specific builders silently no-op on wrong `ParameterType` variant."
- Add: "Type-specific builders panic in debug builds on wrong `ParameterType` variant (zero cost in release)."

Update Key Decisions:
- Add: "`ParameterCollection` implements `FromIterator`, `IntoIterator`, and `extend()`."
- Add: "`ValidatedValues` delegates typed accessors directly — no `.raw()` needed for common access."
- Add: "`Condition::gt()`, `lt()`, `is_true()` — shorthand constructors for all 11 variants."
- Add: "`lint_collection` re-exported in prelude."

Update the reviewed date.

**Step 4: Commit**

```bash
rtk git add .claude/crates/parameter.md
rtk git commit -m "docs(parameter): update crate context after DX polish"
```
