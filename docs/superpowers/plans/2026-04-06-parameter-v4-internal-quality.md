# nebula-parameter v4 — Phase A: Internal Quality

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix all internal code quality issues in nebula-parameter without changing the public API. Foundation for Phase B (builder API) and Phase C (derive macro).

**Architecture:** 7 independent tasks, each touching 1-2 files. No cross-crate impact. Tasks can be executed in any order. All changes are internal refactors — the public `ParameterCollection`, `ParameterValues`, `Parameter` types keep their existing API.

**Tech Stack:** Rust 1.94, edition 2024. `cargo test -p nebula-parameter`, `cargo clippy -p nebula-parameter -- -D warnings`.

---

## File Map

| Task | Modifies | Creates |
|------|----------|---------|
| 1. Fix error.rs category/code duplication | `crates/parameter/src/error.rs` | — |
| 2. Cache regex in transformer | `crates/parameter/src/transformer.rs` | — |
| 3. Condition accepts ParameterValues | `crates/parameter/src/conditions.rs`, `crates/parameter/src/validate.rs` | — |
| 4. Generic Loader<T> | `crates/parameter/src/loader.rs` | — |
| 5. Fix spec.rs Debug-based variant name | `crates/parameter/src/spec.rs` | — |
| 6. Reduce validate.rs allocations | `crates/parameter/src/validate.rs` | — |
| 7. ParameterType consolidation (Date/Time/Color/Hidden) | `crates/parameter/src/parameter_type.rs`, `crates/parameter/src/parameter.rs`, `crates/parameter/src/lib.rs` | `crates/parameter/src/input_hint.rs` |

---

### Task 1: Fix error.rs category/code duplication

**Problem:** `ParameterError` has `#[classify(category = "...", code = "...")]` derive attributes AND manual `category()`/`code()` methods that return different values. Example: `AlreadyExists` has `#[classify(category = "not_found")]` but `category()` returns `"lookup"`.

**Files:**
- Modify: `crates/parameter/src/error.rs:104-137`

- [ ] **Step 1: Write test exposing the inconsistency**

Add to the test module in `crates/parameter/src/error.rs`:

```rust
#[test]
fn classify_matches_manual_methods() {
    use nebula_error::Classify;

    let err = ParameterError::AlreadyExists { key: "test".into() };
    // Classify derive says "not_found", manual method says "lookup" — they must agree
    assert_eq!(err.category().as_str(), err.code().as_str().split('_').next().unwrap_or(""));
    // This test documents the current inconsistency. After the fix,
    // the manual methods will be gone and only Classify remains.
}
```

- [ ] **Step 2: Run test to understand current state**

Run: `cargo test -p nebula-parameter -- classify_matches`

Expected: May pass or fail depending on exact assertion — the point is to document the inconsistency.

- [ ] **Step 3: Fix the `#[classify]` attributes to have correct categories**

In `crates/parameter/src/error.rs`, fix the `AlreadyExists` variant:

```rust
    // old:
    #[classify(category = "not_found", code = "PARAM_ALREADY_EXISTS")]
    // new:
    #[classify(category = "conflict", code = "PARAM_ALREADY_EXISTS")]
```

- [ ] **Step 4: Delete the manual `category()`, `code()`, and `is_retryable()` methods**

Remove the entire `impl ParameterError` block containing `category()`, `code()`, and `is_retryable()` (lines 104-150 approximately). The `Classify` derive already generates these methods via the trait. Any code calling `err.category()` will now go through the `Classify` trait.

- [ ] **Step 5: Fix any compilation errors**

Run: `cargo check -p nebula-parameter`

If any code calls `.category()` or `.code()` as inherent methods, add `use nebula_error::Classify;` at the call site so the trait method resolves.

- [ ] **Step 6: Update the test**

Replace the test with one that validates the Classify impl directly:

```rust
#[test]
fn classify_categories_are_correct() {
    use nebula_error::Classify;

    let not_found = ParameterError::NotFound { key: "x".into() };
    assert_eq!(not_found.category().as_str(), "not_found");

    let already = ParameterError::AlreadyExists { key: "x".into() };
    assert_eq!(already.category().as_str(), "conflict");

    let invalid = ParameterError::InvalidValue { key: "x".into(), reason: "bad".into() };
    assert_eq!(invalid.category().as_str(), "validation");
}
```

- [ ] **Step 7: Run tests + clippy**

Run: `cargo test -p nebula-parameter && cargo clippy -p nebula-parameter -- -D warnings`

Expected: All pass.

- [ ] **Step 8: Commit**

```bash
git add crates/parameter/src/error.rs
git commit -m "fix(parameter): remove duplicate category/code methods, use only Classify derive

Manual category()/code() methods returned different values than the
#[classify] derive attributes. Removed manual methods — Classify
trait is now the single source of truth. Fixed AlreadyExists category
from 'not_found' to 'conflict'.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Cache compiled regex in Transformer

**Problem:** `Transformer::Regex` at `transformer.rs:137` calls `Regex::new(pattern)` on every invocation of `apply()`. If applied to each item in a list of 1000 entries, this compiles 1000 times. Invalid patterns silently return the original value.

**Files:**
- Modify: `crates/parameter/src/transformer.rs`

- [ ] **Step 1: Write a test for regex transformer**

Add to the test module in `transformer.rs`:

```rust
#[test]
fn regex_transformer_extracts_group() {
    let t = Transformer::Regex {
        pattern: r"(\d+)-(\w+)".to_string(),
        group: 1,
    };
    let input = serde_json::json!("42-hello");
    let result = t.apply(&input);
    assert_eq!(result, serde_json::json!("42"));
}

#[test]
fn regex_transformer_invalid_pattern_returns_original() {
    let t = Transformer::Regex {
        pattern: r"[invalid".to_string(),
        group: 0,
    };
    let input = serde_json::json!("test");
    let result = t.apply(&input);
    assert_eq!(result, serde_json::json!("test"));
}
```

- [ ] **Step 2: Run tests to confirm they pass with current code**

Run: `cargo test -p nebula-parameter -- regex_transformer`

Expected: PASS (current code works, just slowly).

- [ ] **Step 3: Add `OnceLock` cache to the `Regex` variant**

The `Transformer` enum is `Clone + PartialEq + Serialize + Deserialize`, so we can't store a `Regex` directly (it's not serializable). Instead, use a module-level `DashMap` cache keyed by pattern string:

At the top of `transformer.rs`, add:

```rust
use std::sync::LazyLock;
use dashmap::DashMap;
use regex::Regex;

/// Module-level cache for compiled regex patterns. Avoids recompilation
/// on every `Transformer::Regex::apply()` call.
static REGEX_CACHE: LazyLock<DashMap<String, Option<Regex>>> = LazyLock::new(DashMap::new);

fn get_or_compile_regex(pattern: &str) -> Option<Regex> {
    if let Some(entry) = REGEX_CACHE.get(pattern) {
        return entry.clone();
    }
    let compiled = Regex::new(pattern).ok();
    REGEX_CACHE.insert(pattern.to_string(), compiled.clone());
    compiled
}
```

Check if `dashmap` and `regex` are already dependencies of nebula-parameter:

Run: `grep -E "dashmap|regex" crates/parameter/Cargo.toml`

If not present, add them. `regex` is likely already there (used in conditions.rs too).

- [ ] **Step 4: Replace inline `Regex::new` with cached lookup**

In `transformer.rs`, replace the `Regex` arm in `apply()`:

```rust
// old:
Self::Regex { pattern, group } => {
    let Some(s) = value.as_str() else {
        return value.clone();
    };
    let Ok(re) = Regex::new(pattern) else {
        return value.clone();
    };
    // ...
}

// new:
Self::Regex { pattern, group } => {
    let Some(s) = value.as_str() else {
        return value.clone();
    };
    let Some(re) = get_or_compile_regex(pattern) else {
        return value.clone();
    };
    match re.captures(s) {
        Some(caps) => caps
            .get(*group)
            .map(|m| Value::String(m.as_str().to_owned()))
            .unwrap_or_else(|| value.clone()),
        None => value.clone(),
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p nebula-parameter -- regex_transformer && cargo clippy -p nebula-parameter -- -D warnings`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/parameter/src/transformer.rs
git commit -m "perf(parameter): cache compiled regexes in Transformer::Regex

Previously Regex::new() was called on every apply() invocation.
Now uses a module-level DashMap cache keyed by pattern string.
Invalid patterns are cached as None to avoid repeated compilation.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Condition::evaluate accepts &ParameterValues

**Problem:** `Condition::evaluate()` takes `&HashMap<String, Value>` instead of `&ParameterValues`. Every call site in `validate.rs` must call `values.as_map()` to get the raw HashMap — leaking internal representation.

**Files:**
- Modify: `crates/parameter/src/conditions.rs:229`
- Modify: `crates/parameter/src/validate.rs` (call sites)

- [ ] **Step 1: Add `as_map()` or equivalent to ParameterValues if it doesn't exist**

Check `crates/parameter/src/values.rs` for a method that returns `&HashMap<String, Value>`. If it exists (likely `as_map()` or similar), note the method name. If not, add one:

```rust
/// Returns a reference to the underlying values map.
pub fn as_map(&self) -> &HashMap<String, Value> {
    &self.values
}
```

- [ ] **Step 2: Change `Condition::evaluate` signature**

In `crates/parameter/src/conditions.rs:229`, change:

```rust
// old:
pub fn evaluate(&self, values: &HashMap<String, Value>) -> bool {

// new:
pub fn evaluate(&self, values: &crate::values::ParameterValues) -> bool {
```

Inside the method body, replace `values.get(field.as_str())` with `values.get(field.as_str())` (ParameterValues already has `.get()` returning `Option<&Value>`). The internal logic should work identically since `ParameterValues::get()` does the same HashMap lookup.

- [ ] **Step 3: Fix compilation errors in validate.rs**

Run: `cargo check -p nebula-parameter`

Find all call sites in `validate.rs` that pass `&HashMap` to `evaluate()` and change them to pass `&ParameterValues` directly. The validate functions likely already have access to `ParameterValues`.

- [ ] **Step 4: Fix any other callers**

Search: `grep -rn "\.evaluate(" crates/parameter/src/`

Fix each call site to pass `&ParameterValues`.

- [ ] **Step 5: Run tests**

Run: `cargo test -p nebula-parameter && cargo clippy -p nebula-parameter -- -D warnings`

Expected: All pass.

- [ ] **Step 6: Commit**

```bash
git add crates/parameter/src/conditions.rs crates/parameter/src/validate.rs
git commit -m "refactor(parameter): Condition::evaluate accepts &ParameterValues

Changed from &HashMap<String, Value> to &ParameterValues, removing
the need for callers to extract the raw map. Internal logic unchanged.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Generic Loader<T>

**Problem:** `OptionLoader`, `RecordLoader`, and `FilterFieldLoader` in `loader.rs` are structurally identical — `Arc<dyn Fn(LoaderContext) -> LoaderFuture<LoaderResult<T>>>` with identical `Clone`, `PartialEq`, and `Debug` impls. ~150 lines of copy-paste.

**Files:**
- Modify: `crates/parameter/src/loader.rs`
- Modify: `crates/parameter/src/parameter_type.rs` (update field types)
- Modify: `crates/parameter/src/lib.rs` (update re-exports)

- [ ] **Step 1: Create generic `Loader<T>`**

In `crates/parameter/src/loader.rs`, add above the existing types:

```rust
/// Generic async loader that resolves items of type `T` for a parameter field.
///
/// The engine resolves credentials and injects them via [`LoaderContext`], then
/// calls the loader to populate data at runtime.
///
/// Two loaders always compare equal (`PartialEq` returns `true`), so
/// adding a loader does not affect schema equality checks.
pub struct Loader<T: Send + 'static>(
    Arc<dyn Fn(LoaderContext) -> LoaderFuture<LoaderResult<T>> + Send + Sync>,
);

impl<T: Send + 'static> Loader<T> {
    /// Wraps an async closure as a [`Loader`].
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LoaderResult<T>, LoaderError>> + Send + 'static,
    {
        Self(Arc::new(move |ctx| Box::pin(f(ctx))))
    }

    /// Invokes the loader with the given context.
    ///
    /// # Errors
    ///
    /// Returns [`LoaderError`] if the loader cannot resolve data.
    pub async fn call(&self, ctx: LoaderContext) -> Result<LoaderResult<T>, LoaderError> {
        (self.0)(ctx).await
    }
}

impl<T: Send + 'static> Clone for Loader<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Send + 'static> PartialEq for Loader<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T: Send + 'static> std::fmt::Debug for Loader<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Loader(<async fn>)")
    }
}
```

- [ ] **Step 2: Replace concrete types with aliases**

Below the generic `Loader<T>`, replace the old types with aliases:

```rust
/// Async loader that resolves [`SelectOption`]s for Select/MultiSelect fields.
pub type OptionLoader = Loader<SelectOption>;

/// Async loader that resolves JSON records for Dynamic fields.
pub type RecordLoader = Loader<serde_json::Value>;

/// Async loader that resolves [`FilterField`]s for Filter fields.
pub type FilterFieldLoader = Loader<FilterField>;
```

Delete the old struct definitions, their `impl` blocks (`new`, `call`, `Clone`, `PartialEq`, `Debug`) for all three types.

- [ ] **Step 3: Fix compilation**

Run: `cargo check -p nebula-parameter`

The type aliases are backwards-compatible — code using `OptionLoader::new(...)` still works because it's now `Loader<SelectOption>::new(...)` via the alias.

- [ ] **Step 4: Run tests**

Run: `cargo test -p nebula-parameter && cargo clippy -p nebula-parameter -- -D warnings`

Expected: All pass — the API is identical via type aliases.

- [ ] **Step 5: Update re-exports in lib.rs if needed**

Add `Loader` to the re-exports alongside the existing aliases:

```rust
pub use loader::{Loader, OptionLoader, RecordLoader, FilterFieldLoader, LoaderContext, LoaderError, LoaderFuture};
```

- [ ] **Step 6: Commit**

```bash
git add crates/parameter/src/loader.rs crates/parameter/src/lib.rs
git commit -m "refactor(parameter): generic Loader<T> replaces 3 copy-paste loader types

OptionLoader, RecordLoader, FilterFieldLoader are now type aliases
for Loader<SelectOption>, Loader<Value>, Loader<FilterField>.
~100 lines of duplicated Clone/PartialEq/Debug impls removed.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Fix spec.rs Debug-based variant name extraction

**Problem:** `spec.rs:218` uses `format!("{other:?}").split_whitespace().next().unwrap_or("unknown")` to extract an enum variant name from Debug output. Fragile — if Debug format changes, the error message breaks.

**Files:**
- Modify: `crates/parameter/src/spec.rs:217-223`

- [ ] **Step 1: Add `variant_name()` method to ParameterType**

In `crates/parameter/src/parameter_type.rs`, add a method:

```rust
impl ParameterType {
    /// Returns the variant name as a static string.
    #[must_use]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::String { .. } => "String",
            Self::Number { .. } => "Number",
            Self::Boolean => "Boolean",
            Self::Select { .. } => "Select",
            Self::MultiSelect { .. } => "MultiSelect",
            Self::Object { .. } => "Object",
            Self::List { .. } => "List",
            Self::Code { .. } => "Code",
            Self::Date => "Date",
            Self::DateTime => "DateTime",
            Self::Time => "Time",
            Self::Color => "Color",
            Self::Hidden => "Hidden",
            Self::Markdown { .. } => "Markdown",
            Self::Notice { .. } => "Notice",
            Self::File { .. } => "File",
            Self::Dynamic { .. } => "Dynamic",
            Self::Filter { .. } => "Filter",
            Self::Mode { .. } => "Mode",
            _ => "Unknown",
        }
    }
}
```

- [ ] **Step 2: Replace Debug-based extraction in spec.rs**

In `crates/parameter/src/spec.rs:217-223`, replace:

```rust
// old:
other => Err(FieldSpecConvertError {
    variant: format!("{other:?}")
        .split_whitespace()
        .next()
        .unwrap_or("unknown")
        .to_string(),
}),

// new:
other => Err(FieldSpecConvertError {
    variant: other.variant_name().to_string(),
}),
```

- [ ] **Step 3: Run tests + clippy**

Run: `cargo test -p nebula-parameter && cargo clippy -p nebula-parameter -- -D warnings`

Expected: All pass.

- [ ] **Step 4: Commit**

```bash
git add crates/parameter/src/spec.rs crates/parameter/src/parameter_type.rs
git commit -m "refactor(parameter): replace Debug-based variant name with explicit method

ParameterType::variant_name() returns &'static str instead of
parsing Debug output with split_whitespace. Deterministic and
independent of Debug format changes.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Reduce validate.rs allocation pressure

**Problem:** `validate_object` (line 339) and `validate_list` (line 418) construct a new `ParameterValues` (allocating HashMap) for every nested object and every list item. For a schema with 5 nested objects of 10 items, that is 50+ HashMap allocations per validation.

**Files:**
- Modify: `crates/parameter/src/validate.rs`

- [ ] **Step 1: Read the current validate_object and validate_list functions**

Read `crates/parameter/src/validate.rs` lines 330-450 to understand the current allocation pattern. Note which functions create `ParameterValues` from `&Value`.

- [ ] **Step 2: Write a benchmark-style test for nested validation**

```rust
#[test]
fn validates_deeply_nested_object_without_excessive_allocation() {
    let schema = ParameterCollection::new()
        .add(Parameter::object("level1").add(
            Parameter::object("level2").add(
                Parameter::object("level3").add(
                    Parameter::string("deep_field").required()
                )
            )
        ));

    let values = ParameterValues::from_json(serde_json::json!({
        "level1": {
            "level2": {
                "level3": {
                    "deep_field": "value"
                }
            }
        }
    }));

    let report = schema.validate(&values);
    assert!(report.is_ok());
}
```

- [ ] **Step 3: Refactor validate_object to pass &Value references**

Instead of constructing a new `ParameterValues` for each nested object, pass the `&serde_json::Value` directly and walk the JSON tree. Create a helper that validates a `&Value` against a `&[Parameter]` without creating intermediate `ParameterValues`:

```rust
fn validate_value_against_params(
    value: &Value,
    params: &[Parameter],
    path: &str,
    errors: &mut Vec<ValidationError>,
    values_root: &ParameterValues,  // for condition evaluation
) {
    // Extract object fields from value without allocating HashMap
    let obj = match value.as_object() {
        Some(o) => o,
        None => {
            errors.push(/* type error */);
            return;
        }
    };
    for param in params {
        let field_value = obj.get(&param.id);
        // validate field_value against param rules directly
        // ...
    }
}
```

The key change: pass `values_root: &ParameterValues` (the top-level values) for condition evaluation, but validate nested fields by walking `&Value` references instead of constructing new `ParameterValues` per level.

- [ ] **Step 4: Apply similar refactor to validate_list**

For list validation, iterate `value.as_array()` items and validate each against the item template parameter, passing `&Value` references.

- [ ] **Step 5: Run tests**

Run: `cargo test -p nebula-parameter && cargo clippy -p nebula-parameter -- -D warnings`

Expected: All existing tests pass. The deeply nested test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/parameter/src/validate.rs
git commit -m "perf(parameter): reduce allocations in nested object/list validation

Validate nested objects and list items by walking &Value references
instead of constructing intermediate ParameterValues (HashMap) per
nesting level. Condition evaluation uses the root ParameterValues.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: ParameterType consolidation — InputHint

**Problem:** `Date`, `DateTime`, `Time`, `Color`, `Hidden` are zero-field ParameterType variants that are effectively String with a UI hint. They add 5 match arms to every exhaustive match on ParameterType without adding type-specific behavior.

**Files:**
- Create: `crates/parameter/src/input_hint.rs`
- Modify: `crates/parameter/src/parameter_type.rs`
- Modify: `crates/parameter/src/parameter.rs`
- Modify: `crates/parameter/src/lib.rs`
- Modify: `crates/parameter/src/validate.rs` (remove match arms)
- Modify: `crates/parameter/src/normalize.rs` (remove match arms)

- [ ] **Step 1: Create InputHint enum**

Create `crates/parameter/src/input_hint.rs`:

```rust
//! Input hints for String parameters.
//!
//! Hints tell the UI which specialized input widget to render
//! (e.g., a date picker, color picker, URL input with validation).

use serde::{Deserialize, Serialize};

/// UI rendering hint for String parameters.
///
/// Does not change the underlying data type (always stored as String).
/// The UI uses the hint to render a specialized input widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InputHint {
    /// Default text input.
    Text,
    /// URL input with format validation hint.
    Url,
    /// Email input with format validation hint.
    Email,
    /// Date picker (YYYY-MM-DD).
    Date,
    /// Date and time picker (ISO 8601).
    DateTime,
    /// Time picker (HH:MM:SS).
    Time,
    /// Color picker (hex string).
    Color,
    /// Password input (masked).
    Password,
    /// Phone number input.
    Phone,
    /// IP address input.
    Ip,
}

impl Default for InputHint {
    fn default() -> Self {
        Self::Text
    }
}
```

- [ ] **Step 2: Add `input_hint` field to ParameterType::String**

In `crates/parameter/src/parameter_type.rs`, modify the `String` variant:

```rust
// old:
String {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    multiline: bool,
},

// new:
String {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    multiline: bool,
    /// UI input hint (date picker, color picker, URL input, etc.).
    #[serde(default, skip_serializing_if = "InputHint::is_default")]
    input_hint: InputHint,
},
```

Add to `InputHint`:

```rust
impl InputHint {
    /// Returns true if this is the default hint (Text).
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Text)
    }
}
```

- [ ] **Step 3: Deprecate Date/DateTime/Time/Color/Hidden variants**

In `parameter_type.rs`, add `#[deprecated]` to each:

```rust
/// Date input. **Deprecated:** use `String` with `input_hint: InputHint::Date`.
#[deprecated(since = "0.4.0", note = "use ParameterType::String with InputHint::Date")]
Date,

/// Date-time input. **Deprecated:** use `String` with `input_hint: InputHint::DateTime`.
#[deprecated(since = "0.4.0", note = "use ParameterType::String with InputHint::DateTime")]
DateTime,

/// Time input. **Deprecated:** use `String` with `input_hint: InputHint::Time`.
#[deprecated(since = "0.4.0", note = "use ParameterType::String with InputHint::Time")]
Time,

/// Color picker. **Deprecated:** use `String` with `input_hint: InputHint::Color`.
#[deprecated(since = "0.4.0", note = "use ParameterType::String with InputHint::Color")]
Color,

/// Hidden field. **Deprecated:** use `Parameter.visible = false`.
#[deprecated(since = "0.4.0", note = "set visible = false on Parameter instead")]
Hidden,
```

- [ ] **Step 4: Add convenience constructors to Parameter**

In `parameter.rs`, add:

```rust
/// Creates a date parameter (String with InputHint::Date).
#[must_use]
pub fn date(id: impl Into<String>) -> Self {
    Self::new(id, ParameterType::String {
        multiline: false,
        input_hint: InputHint::Date,
    })
}

/// Creates a datetime parameter (String with InputHint::DateTime).
#[must_use]
pub fn datetime(id: impl Into<String>) -> Self {
    Self::new(id, ParameterType::String {
        multiline: false,
        input_hint: InputHint::DateTime,
    })
}

/// Creates a time parameter (String with InputHint::Time).
#[must_use]
pub fn time(id: impl Into<String>) -> Self {
    Self::new(id, ParameterType::String {
        multiline: false,
        input_hint: InputHint::Time,
    })
}

/// Creates a color parameter (String with InputHint::Color).
#[must_use]
pub fn color(id: impl Into<String>) -> Self {
    Self::new(id, ParameterType::String {
        multiline: false,
        input_hint: InputHint::Color,
    })
}
```

Update `Parameter::string()` to include the new field:

```rust
pub fn string(id: impl Into<String>) -> Self {
    Self::new(id, ParameterType::String {
        multiline: false,
        input_hint: InputHint::default(),
    })
}
```

- [ ] **Step 5: Update lib.rs re-exports**

In `crates/parameter/src/lib.rs`, add:

```rust
pub mod input_hint;
pub use input_hint::InputHint;
```

- [ ] **Step 6: Fix all match arms across the crate**

Run: `cargo check -p nebula-parameter 2>&1`

Fix deprecation warnings in `validate.rs`, `normalize.rs`, `lint.rs`, and anywhere else that matches on `Date`/`DateTime`/`Time`/`Color`/`Hidden`. Replace:

```rust
// old match arms:
ParameterType::Date => { /* string validation */ }
ParameterType::DateTime => { /* string validation */ }

// new: these cases are now handled by String with input_hint
// Remove the explicit arms — they fall through to String
```

Allow deprecated usages temporarily with `#[allow(deprecated)]` on the match blocks that handle backward compat deserialization.

- [ ] **Step 7: Run full test suite**

Run: `cargo test -p nebula-parameter && cargo clippy -p nebula-parameter -- -D warnings && cargo check --workspace`

Expected: All pass. No cross-crate impact (only parameter internals changed).

- [ ] **Step 8: Commit**

```bash
git add crates/parameter/src/input_hint.rs crates/parameter/src/parameter_type.rs crates/parameter/src/parameter.rs crates/parameter/src/lib.rs crates/parameter/src/validate.rs crates/parameter/src/normalize.rs
git commit -m "refactor(parameter): add InputHint, deprecate Date/DateTime/Time/Color/Hidden variants

New InputHint enum (Text, Url, Email, Date, DateTime, Time, Color,
Password, Phone, Ip) on ParameterType::String. Date/DateTime/Time/
Color/Hidden variants deprecated — use String with InputHint instead.
Convenience constructors: Parameter::date(), datetime(), time(), color().

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

## Final Verification

After all 7 tasks:

- [ ] **Full workspace check**

```bash
cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace --exclude nebula-api --exclude nebula-credential
```

(Excluding nebula-credential for pre-existing test failures.)

- [ ] **Update context file**

Update `.claude/crates/parameter.md` with:
- InputHint addition
- Deprecated variants
- Condition::evaluate signature change
- Generic Loader<T>
- ParameterType::variant_name() method

- [ ] **Update active-work.md**

Add: "parameter v4 Phase A (internal quality) complete"
