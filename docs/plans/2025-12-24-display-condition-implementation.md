# DisplayCondition Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable and fix the display condition system in nebula-parameter to control parameter visibility based on other parameter values.

**Architecture:** Replace the stub implementation with a self-contained display system that doesn't depend on nebula-validator. Use simple condition types (Equals, NotEquals, GreaterThan, etc.) that operate directly on `Value` types. Display rules can be combined with AND/OR/NOT logic.

**Tech Stack:** Rust, nebula-value (Value, ValueKind), nebula-core (ParameterKey), serde for serialization

---

## Current State Analysis

The display system currently has:
1. `display.rs` - Full implementation that depends on non-existent nebula-validator APIs (`Validator` trait, `WhenFieldValidator`, `FieldsEqualValidator`, etc.)
2. `display_stub.rs` - Minimal stub with empty implementations (currently used)
3. `core/mod.rs` - Imports `display_stub`, has `display` commented out

The original `display.rs` tried to reuse nebula-validator, but those APIs don't exist. We need a self-contained solution.

---

## Task 1: Create DisplayCondition Enum

**Files:**
- Create: `crates/nebula-parameter/src/core/display.rs` (replace stub)

**Step 1: Write the failing test**

Add to end of new `display.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_value::Value;

    #[test]
    fn test_condition_equals() {
        let condition = DisplayCondition::Equals(Value::text("api_key"));
        assert!(condition.evaluate(&Value::text("api_key")));
        assert!(!condition.evaluate(&Value::text("oauth")));
    }

    #[test]
    fn test_condition_not_equals() {
        let condition = DisplayCondition::NotEquals(Value::text("disabled"));
        assert!(condition.evaluate(&Value::text("enabled")));
        assert!(!condition.evaluate(&Value::text("disabled")));
    }

    #[test]
    fn test_condition_is_set() {
        let condition = DisplayCondition::IsSet;
        assert!(condition.evaluate(&Value::text("hello")));
        assert!(condition.evaluate(&Value::integer(0)));
        assert!(!condition.evaluate(&Value::Null));
    }

    #[test]
    fn test_condition_is_empty() {
        let condition = DisplayCondition::IsEmpty;
        assert!(condition.evaluate(&Value::text("")));
        assert!(condition.evaluate(&Value::Array(vec![])));
        assert!(!condition.evaluate(&Value::text("hello")));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-parameter test_condition_equals`
Expected: FAIL with compilation error (DisplayCondition doesn't exist)

**Step 3: Write minimal implementation**

```rust
//! Display condition system for conditional parameter visibility
//!
//! This module provides a self-contained system for controlling when parameters
//! should be displayed based on the values of other parameters.

use nebula_core::ParameterKey;
use nebula_value::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A condition that can be evaluated against a Value
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DisplayCondition {
    /// Value equals the specified value
    Equals(Value),
    /// Value does not equal the specified value
    NotEquals(Value),
    /// Value is not null
    IsSet,
    /// Value is null
    IsNull,
    /// Value is empty (empty string, empty array, empty object)
    IsEmpty,
    /// Value is not empty
    IsNotEmpty,
    /// Value is true (for booleans)
    IsTrue,
    /// Value is false (for booleans)
    IsFalse,
    /// Numeric value is greater than threshold
    GreaterThan(f64),
    /// Numeric value is less than threshold
    LessThan(f64),
    /// Numeric value is in range [min, max]
    InRange { min: f64, max: f64 },
    /// String contains substring
    Contains(String),
    /// String starts with prefix
    StartsWith(String),
    /// String ends with suffix
    EndsWith(String),
    /// Value is one of the specified values
    OneOf(Vec<Value>),
}

impl DisplayCondition {
    /// Evaluate this condition against a value
    pub fn evaluate(&self, value: &Value) -> bool {
        match self {
            DisplayCondition::Equals(expected) => value == expected,
            DisplayCondition::NotEquals(expected) => value != expected,
            DisplayCondition::IsSet => !matches!(value, Value::Null),
            DisplayCondition::IsNull => matches!(value, Value::Null),
            DisplayCondition::IsEmpty => Self::is_value_empty(value),
            DisplayCondition::IsNotEmpty => !Self::is_value_empty(value),
            DisplayCondition::IsTrue => matches!(value, Value::Boolean(true)),
            DisplayCondition::IsFalse => matches!(value, Value::Boolean(false)),
            DisplayCondition::GreaterThan(threshold) => {
                Self::get_numeric(value).map_or(false, |n| n > *threshold)
            }
            DisplayCondition::LessThan(threshold) => {
                Self::get_numeric(value).map_or(false, |n| n < *threshold)
            }
            DisplayCondition::InRange { min, max } => {
                Self::get_numeric(value).map_or(false, |n| n >= *min && n <= *max)
            }
            DisplayCondition::Contains(substring) => {
                Self::get_string(value).map_or(false, |s| s.contains(substring))
            }
            DisplayCondition::StartsWith(prefix) => {
                Self::get_string(value).map_or(false, |s| s.starts_with(prefix))
            }
            DisplayCondition::EndsWith(suffix) => {
                Self::get_string(value).map_or(false, |s| s.ends_with(suffix))
            }
            DisplayCondition::OneOf(values) => values.contains(value),
        }
    }

    /// Check if a value is considered empty
    fn is_value_empty(value: &Value) -> bool {
        match value {
            Value::Null => true,
            Value::String(s) => s.is_empty(),
            Value::Array(arr) => arr.is_empty(),
            Value::Object(obj) => obj.is_empty(),
            _ => false,
        }
    }

    /// Extract numeric value as f64
    fn get_numeric(value: &Value) -> Option<f64> {
        match value {
            Value::Integer(n) => Some(*n as f64),
            Value::Float(f) => Some(*f),
            Value::Decimal(d) => d.to_string().parse().ok(),
            _ => None,
        }
    }

    /// Extract string value
    fn get_string(value: &Value) -> Option<&str> {
        match value {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-parameter test_condition`
Expected: PASS (all 4 tests)

**Step 5: Commit**

```bash
git add crates/nebula-parameter/src/core/display.rs
git commit -m "feat(nebula-parameter): add DisplayCondition enum with evaluation logic"
```

---

## Task 2: Add DisplayRule for Field-Based Conditions

**Files:**
- Modify: `crates/nebula-parameter/src/core/display.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_display_rule_single() {
    let rule = DisplayRule::when("auth_type", DisplayCondition::Equals(Value::text("api_key")));
    
    let ctx = DisplayContext::new()
        .with_value("auth_type", Value::text("api_key"));
    
    assert!(rule.evaluate(&ctx));
}

#[test]
fn test_display_rule_missing_field() {
    let rule = DisplayRule::when("auth_type", DisplayCondition::Equals(Value::text("api_key")));
    
    let ctx = DisplayContext::new(); // No auth_type
    
    assert!(!rule.evaluate(&ctx)); // Missing field = condition not met
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-parameter test_display_rule`
Expected: FAIL (DisplayRule doesn't exist)

**Step 3: Write minimal implementation**

Add after `DisplayCondition` impl:

```rust
/// Context containing resolved parameter values for display evaluation
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayContext {
    values: HashMap<ParameterKey, Value>,
}

impl DisplayContext {
    /// Create a new empty context
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a parameter value by key
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(&ParameterKey::from(key))
    }

    /// Builder pattern: add a value and return self
    #[must_use]
    pub fn with_value(mut self, key: impl Into<ParameterKey>, value: Value) -> Self {
        self.values.insert(key.into(), value);
        self
    }

    /// Insert a value
    pub fn insert(&mut self, key: impl Into<ParameterKey>, value: Value) {
        self.values.insert(key.into(), value);
    }

    /// Check if context contains a key
    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(&ParameterKey::from(key))
    }

    /// Get all values as HashMap reference
    pub fn values(&self) -> &HashMap<ParameterKey, Value> {
        &self.values
    }
}

/// A display rule that checks a specific field against a condition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayRule {
    /// The parameter key to check
    pub field: ParameterKey,
    /// The condition to evaluate
    pub condition: DisplayCondition,
}

impl DisplayRule {
    /// Create a new display rule
    pub fn when(field: impl Into<ParameterKey>, condition: DisplayCondition) -> Self {
        Self {
            field: field.into(),
            condition,
        }
    }

    /// Evaluate this rule against a context
    pub fn evaluate(&self, ctx: &DisplayContext) -> bool {
        match ctx.get(self.field.as_str()) {
            Some(value) => self.condition.evaluate(value),
            None => false, // Missing field = condition not met
        }
    }

    /// Get the field this rule depends on
    pub fn dependency(&self) -> &ParameterKey {
        &self.field
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-parameter test_display_rule`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/nebula-parameter/src/core/display.rs
git commit -m "feat(nebula-parameter): add DisplayRule and DisplayContext"
```

---

## Task 3: Add DisplayRuleSet for Logical Combinations

**Files:**
- Modify: `crates/nebula-parameter/src/core/display.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_ruleset_and() {
    let ruleset = DisplayRuleSet::all([
        DisplayRule::when("enabled", DisplayCondition::IsTrue),
        DisplayRule::when("level", DisplayCondition::GreaterThan(10.0)),
    ]);

    let ctx_pass = DisplayContext::new()
        .with_value("enabled", Value::Boolean(true))
        .with_value("level", Value::Integer(15));

    let ctx_fail = DisplayContext::new()
        .with_value("enabled", Value::Boolean(true))
        .with_value("level", Value::Integer(5));

    assert!(ruleset.evaluate(&ctx_pass));
    assert!(!ruleset.evaluate(&ctx_fail));
}

#[test]
fn test_ruleset_or() {
    let ruleset = DisplayRuleSet::any([
        DisplayRule::when("role", DisplayCondition::Equals(Value::text("admin"))),
        DisplayRule::when("superuser", DisplayCondition::IsTrue),
    ]);

    let ctx_admin = DisplayContext::new()
        .with_value("role", Value::text("admin"));

    let ctx_superuser = DisplayContext::new()
        .with_value("superuser", Value::Boolean(true));

    let ctx_neither = DisplayContext::new()
        .with_value("role", Value::text("user"));

    assert!(ruleset.evaluate(&ctx_admin));
    assert!(ruleset.evaluate(&ctx_superuser));
    assert!(!ruleset.evaluate(&ctx_neither));
}

#[test]
fn test_ruleset_not() {
    let ruleset = DisplayRuleSet::not(
        DisplayRule::when("disabled", DisplayCondition::IsTrue)
    );

    let ctx_enabled = DisplayContext::new()
        .with_value("disabled", Value::Boolean(false));

    let ctx_disabled = DisplayContext::new()
        .with_value("disabled", Value::Boolean(true));

    assert!(ruleset.evaluate(&ctx_enabled));
    assert!(!ruleset.evaluate(&ctx_disabled));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-parameter test_ruleset`
Expected: FAIL (DisplayRuleSet doesn't exist)

**Step 3: Write minimal implementation**

```rust
/// A set of display rules combined with logical operators
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DisplayRuleSet {
    /// A single rule
    Single(DisplayRule),
    /// All rules must pass (AND)
    All(Vec<DisplayRuleSet>),
    /// Any rule must pass (OR)
    Any(Vec<DisplayRuleSet>),
    /// Rule must not pass (NOT)
    Not(Box<DisplayRuleSet>),
}

impl DisplayRuleSet {
    /// Create from a single rule
    pub fn single(rule: DisplayRule) -> Self {
        DisplayRuleSet::Single(rule)
    }

    /// Create an ALL ruleset (AND)
    pub fn all(rules: impl IntoIterator<Item = impl Into<DisplayRuleSet>>) -> Self {
        DisplayRuleSet::All(rules.into_iter().map(Into::into).collect())
    }

    /// Create an ANY ruleset (OR)
    pub fn any(rules: impl IntoIterator<Item = impl Into<DisplayRuleSet>>) -> Self {
        DisplayRuleSet::Any(rules.into_iter().map(Into::into).collect())
    }

    /// Create a NOT ruleset
    pub fn not(rule: impl Into<DisplayRuleSet>) -> Self {
        DisplayRuleSet::Not(Box::new(rule.into()))
    }

    /// Evaluate this ruleset against a context
    pub fn evaluate(&self, ctx: &DisplayContext) -> bool {
        match self {
            DisplayRuleSet::Single(rule) => rule.evaluate(ctx),
            DisplayRuleSet::All(rules) => rules.iter().all(|r| r.evaluate(ctx)),
            DisplayRuleSet::Any(rules) => rules.iter().any(|r| r.evaluate(ctx)),
            DisplayRuleSet::Not(rule) => !rule.evaluate(ctx),
        }
    }

    /// Get all parameter dependencies from this ruleset
    pub fn dependencies(&self) -> Vec<ParameterKey> {
        let mut deps = Vec::new();
        self.collect_dependencies(&mut deps);
        deps.sort();
        deps.dedup();
        deps
    }

    fn collect_dependencies(&self, deps: &mut Vec<ParameterKey>) {
        match self {
            DisplayRuleSet::Single(rule) => {
                deps.push(rule.field.clone());
            }
            DisplayRuleSet::All(rules) | DisplayRuleSet::Any(rules) => {
                for rule in rules {
                    rule.collect_dependencies(deps);
                }
            }
            DisplayRuleSet::Not(rule) => {
                rule.collect_dependencies(deps);
            }
        }
    }
}

impl From<DisplayRule> for DisplayRuleSet {
    fn from(rule: DisplayRule) -> Self {
        DisplayRuleSet::Single(rule)
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-parameter test_ruleset`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/nebula-parameter/src/core/display.rs
git commit -m "feat(nebula-parameter): add DisplayRuleSet with AND/OR/NOT logic"
```

---

## Task 4: Add ParameterDisplay Configuration

**Files:**
- Modify: `crates/nebula-parameter/src/core/display.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_parameter_display_show_when() {
    let display = ParameterDisplay::new()
        .show_when(DisplayRule::when("auth_type", DisplayCondition::Equals(Value::text("api_key"))));

    let ctx_show = DisplayContext::new()
        .with_value("auth_type", Value::text("api_key"));

    let ctx_hide = DisplayContext::new()
        .with_value("auth_type", Value::text("oauth"));

    assert!(display.should_display(&ctx_show));
    assert!(!display.should_display(&ctx_hide));
}

#[test]
fn test_parameter_display_hide_when() {
    let display = ParameterDisplay::new()
        .hide_when(DisplayRule::when("disabled", DisplayCondition::IsTrue));

    let ctx_show = DisplayContext::new()
        .with_value("disabled", Value::Boolean(false));

    let ctx_hide = DisplayContext::new()
        .with_value("disabled", Value::Boolean(true));

    assert!(display.should_display(&ctx_show));
    assert!(!display.should_display(&ctx_hide));
}

#[test]
fn test_parameter_display_hide_takes_priority() {
    // hide_when is checked first
    let display = ParameterDisplay::new()
        .show_when(DisplayRule::when("enabled", DisplayCondition::IsTrue))
        .hide_when(DisplayRule::when("maintenance", DisplayCondition::IsTrue));

    let ctx = DisplayContext::new()
        .with_value("enabled", Value::Boolean(true))
        .with_value("maintenance", Value::Boolean(true));

    // Even though show condition is met, hide takes priority
    assert!(!display.should_display(&ctx));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-parameter test_parameter_display`
Expected: FAIL (ParameterDisplay doesn't have these methods)

**Step 3: Write minimal implementation**

```rust
/// Configuration determining when a parameter should be displayed
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParameterDisplay {
    /// Conditions that must be met to show the parameter
    #[serde(skip_serializing_if = "Option::is_none")]
    show_when: Option<DisplayRuleSet>,
    /// Conditions that cause the parameter to be hidden (takes priority)
    #[serde(skip_serializing_if = "Option::is_none")]
    hide_when: Option<DisplayRuleSet>,
}

impl ParameterDisplay {
    /// Create a new empty display configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a show condition
    #[must_use]
    pub fn show_when(mut self, rule: impl Into<DisplayRuleSet>) -> Self {
        let ruleset = rule.into();
        self.show_when = Some(match self.show_when.take() {
            Some(existing) => DisplayRuleSet::All(vec![existing, ruleset]),
            None => ruleset,
        });
        self
    }

    /// Add a hide condition
    #[must_use]
    pub fn hide_when(mut self, rule: impl Into<DisplayRuleSet>) -> Self {
        let ruleset = rule.into();
        self.hide_when = Some(match self.hide_when.take() {
            Some(existing) => DisplayRuleSet::Any(vec![existing, ruleset]),
            None => ruleset,
        });
        self
    }

    /// Check if parameter should be displayed
    pub fn should_display(&self, ctx: &DisplayContext) -> bool {
        // Priority: hide_when is checked first
        if let Some(hide_rules) = &self.hide_when {
            if hide_rules.evaluate(ctx) {
                return false;
            }
        }

        // Then check show_when
        if let Some(show_rules) = &self.show_when {
            return show_rules.evaluate(ctx);
        }

        // Default: show
        true
    }

    /// Check if this display has no conditions
    pub fn is_empty(&self) -> bool {
        self.show_when.is_none() && self.hide_when.is_none()
    }

    /// Get all parameter dependencies
    pub fn dependencies(&self) -> Vec<ParameterKey> {
        let mut deps = Vec::new();

        if let Some(show) = &self.show_when {
            deps.extend(show.dependencies());
        }

        if let Some(hide) = &self.hide_when {
            deps.extend(hide.dependencies());
        }

        deps.sort();
        deps.dedup();
        deps
    }

    /// Convenience: show when field equals value
    #[must_use]
    pub fn show_when_equals(self, field: impl Into<ParameterKey>, value: Value) -> Self {
        self.show_when(DisplayRule::when(field, DisplayCondition::Equals(value)))
    }

    /// Convenience: show when field is true
    #[must_use]
    pub fn show_when_true(self, field: impl Into<ParameterKey>) -> Self {
        self.show_when(DisplayRule::when(field, DisplayCondition::IsTrue))
    }

    /// Convenience: hide when field equals value
    #[must_use]
    pub fn hide_when_equals(self, field: impl Into<ParameterKey>, value: Value) -> Self {
        self.hide_when(DisplayRule::when(field, DisplayCondition::Equals(value)))
    }

    /// Convenience: hide when field is true
    #[must_use]
    pub fn hide_when_true(self, field: impl Into<ParameterKey>) -> Self {
        self.hide_when(DisplayRule::when(field, DisplayCondition::IsTrue))
    }
}

/// Error type for display validation
#[derive(Debug, Clone, thiserror::Error)]
#[error("Display condition not met: {message}")]
pub struct ParameterDisplayError {
    /// Error message
    pub message: String,
}

impl ParameterDisplayError {
    /// Create a new error
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-parameter test_parameter_display`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/nebula-parameter/src/core/display.rs
git commit -m "feat(nebula-parameter): add ParameterDisplay with show_when/hide_when"
```

---

## Task 5: Enable Display Module and Update Exports

**Files:**
- Modify: `crates/nebula-parameter/src/core/mod.rs`
- Delete: `crates/nebula-parameter/src/core/display_stub.rs`
- Modify: `crates/nebula-parameter/src/lib.rs`

**Step 1: Run all tests to establish baseline**

Run: `cargo test -p nebula-parameter`
Expected: All existing tests pass

**Step 2: Update mod.rs to use display instead of display_stub**

Replace in `crates/nebula-parameter/src/core/mod.rs`:

```rust
// OLD:
pub mod display_stub;
// mod display;  // TODO: Temporarily disabled, needs rewrite
pub use display_stub::*; // TODO: Temporary stub

// NEW:
pub mod display;
pub use display::*;
```

**Step 3: Delete the stub file**

```bash
rm crates/nebula-parameter/src/core/display_stub.rs
```

**Step 4: Update traits.rs imports**

In `crates/nebula-parameter/src/core/traits.rs`, change:

```rust
// OLD:
use crate::core::display_stub::{
    DisplayContext, ParameterCondition, ParameterDisplay, ParameterDisplayError,
};

// NEW:
use crate::core::display::{
    DisplayContext, DisplayCondition, ParameterDisplay, ParameterDisplayError,
};
```

Also remove `ParameterCondition` usage in trait methods (replace with `DisplayCondition`).

**Step 5: Run tests to verify everything works**

Run: `cargo test -p nebula-parameter`
Expected: All tests pass

**Step 6: Commit**

```bash
git add -A
git commit -m "feat(nebula-parameter): enable display module, remove stub"
```

---

## Task 6: Update Displayable Trait

**Files:**
- Modify: `crates/nebula-parameter/src/core/traits.rs`

**Step 1: Write failing test**

In `crates/nebula-parameter/tests/display_test.rs` (new file):

```rust
use nebula_parameter::prelude::*;
use nebula_value::Value;

#[test]
fn test_text_parameter_display_condition() {
    let param = TextParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("api_key")
                .name("API Key")
                .description("Enter your API key")
                .build()
                .unwrap(),
        )
        .display(
            ParameterDisplay::new()
                .show_when_equals("auth_type", Value::text("api_key"))
        )
        .build();

    let ctx_show = DisplayContext::new()
        .with_value("auth_type", Value::text("api_key"));

    let ctx_hide = DisplayContext::new()
        .with_value("auth_type", Value::text("oauth"));

    assert!(param.should_display(&ctx_show));
    assert!(!param.should_display(&ctx_hide));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-parameter test_text_parameter_display`
Expected: FAIL (TextParameter doesn't have display method in builder)

**Step 3: Fix DisplayableMut trait to use DisplayCondition**

In `traits.rs`, update the `add_condition` method signature:

```rust
/// Extension trait for mutable display operations
pub trait DisplayableMut: Displayable {
    /// Add a display condition for a field
    fn add_show_condition(&mut self, field: impl Into<ParameterKey>, condition: DisplayCondition) {
        let rule = DisplayRule::when(field, condition);
        let mut display = self.display().cloned().unwrap_or_default();
        display = display.show_when(rule);
        self.set_display(Some(display));
    }

    /// Add a hide condition for a field
    fn add_hide_condition(&mut self, field: impl Into<ParameterKey>, condition: DisplayCondition) {
        let rule = DisplayRule::when(field, condition);
        let mut display = self.display().cloned().unwrap_or_default();
        display = display.hide_when(rule);
        self.set_display(Some(display));
    }

    /// Clear all display conditions
    fn clear_conditions(&mut self) {
        self.set_display(None);
    }
}
```

**Step 4: Add display field to TextParameter builder**

In `crates/nebula-parameter/src/types/text.rs`, add to builder:

```rust
impl TextParameterBuilder {
    // ... existing methods ...

    /// Set display conditions
    pub fn display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }
}
```

And add to struct:

```rust
pub struct TextParameter {
    // ... existing fields ...
    display: Option<ParameterDisplay>,
}
```

**Step 5: Run test to verify it passes**

Run: `cargo test -p nebula-parameter test_text_parameter_display`
Expected: PASS

**Step 6: Commit**

```bash
git add -A
git commit -m "feat(nebula-parameter): update Displayable trait with DisplayCondition"
```

---

## Task 7: Add Display Field to All Parameter Types

**Files:**
- Modify: All files in `crates/nebula-parameter/src/types/*.rs`

For each parameter type, add:

1. `display: Option<ParameterDisplay>` field to struct
2. `display` method to builder
3. `Displayable` trait implementation

**Step 1: Create a checklist of files**

Files to update:
- `checkbox.rs`
- `code.rs`
- `color.rs`
- `date.rs`
- `datetime.rs`
- `expirable.rs`
- `file.rs`
- `group.rs`
- `hidden.rs`
- `list.rs`
- `mode.rs`
- `multi_select.rs`
- `number.rs`
- `object.rs`
- `radio.rs`
- `resource.rs`
- `routing.rs`
- `secret.rs`
- `select.rs`
- `text.rs`
- `textarea.rs`
- `time.rs`

**Step 2: Add display field pattern to each**

For each file, add:

```rust
// In struct definition:
pub struct XxxParameter {
    // ... existing fields ...
    /// Display conditions for this parameter
    display: Option<ParameterDisplay>,
}

// In builder:
impl XxxParameterBuilder {
    /// Set display conditions
    pub fn display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }
}

// In build() method, include display field

// Add Displayable impl:
impl Displayable for XxxParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}
```

**Step 3: Run full test suite**

Run: `cargo test -p nebula-parameter`
Expected: All tests pass

**Step 4: Commit**

```bash
git add -A
git commit -m "feat(nebula-parameter): add display field to all parameter types"
```

---

## Task 8: Add Prelude Exports

**Files:**
- Modify: `crates/nebula-parameter/src/lib.rs`

**Step 1: Update prelude to include display types**

```rust
pub mod prelude {
    pub use crate::core::{
        DisplayCondition, DisplayContext, DisplayRule, DisplayRuleSet,
        ParameterDisplay, ParameterDisplayError,
        // ... existing exports ...
    };
}
```

**Step 2: Run tests**

Run: `cargo test -p nebula-parameter`
Expected: All tests pass

**Step 3: Commit**

```bash
git add crates/nebula-parameter/src/lib.rs
git commit -m "feat(nebula-parameter): export display types in prelude"
```

---

## Task 9: Add Integration Tests

**Files:**
- Create: `crates/nebula-parameter/tests/display_test.rs`

**Step 1: Write comprehensive integration tests**

```rust
//! Integration tests for the display condition system

use nebula_parameter::prelude::*;
use nebula_value::Value;

#[test]
fn test_complex_display_conditions() {
    // Show API key field when auth_type is "api_key" AND advanced mode is enabled
    let display = ParameterDisplay::new()
        .show_when(DisplayRuleSet::all([
            DisplayRule::when("auth_type", DisplayCondition::Equals(Value::text("api_key"))),
            DisplayRule::when("advanced_mode", DisplayCondition::IsTrue),
        ]));

    let ctx1 = DisplayContext::new()
        .with_value("auth_type", Value::text("api_key"))
        .with_value("advanced_mode", Value::Boolean(true));

    let ctx2 = DisplayContext::new()
        .with_value("auth_type", Value::text("api_key"))
        .with_value("advanced_mode", Value::Boolean(false));

    assert!(display.should_display(&ctx1));
    assert!(!display.should_display(&ctx2));
}

#[test]
fn test_display_dependencies() {
    let display = ParameterDisplay::new()
        .show_when(DisplayRule::when("auth_type", DisplayCondition::Equals(Value::text("api_key"))))
        .hide_when(DisplayRule::when("disabled", DisplayCondition::IsTrue));

    let deps = display.dependencies();
    assert!(deps.contains(&ParameterKey::from("auth_type")));
    assert!(deps.contains(&ParameterKey::from("disabled")));
}

#[test]
fn test_display_serialization() {
    let display = ParameterDisplay::new()
        .show_when_equals("mode", Value::text("advanced"));

    let json = serde_json::to_string(&display).unwrap();
    let restored: ParameterDisplay = serde_json::from_str(&json).unwrap();

    assert_eq!(display, restored);
}
```

**Step 2: Run tests**

Run: `cargo test -p nebula-parameter display`
Expected: All tests pass

**Step 3: Commit**

```bash
git add crates/nebula-parameter/tests/display_test.rs
git commit -m "test(nebula-parameter): add display condition integration tests"
```

---

## Task 10: Final Cleanup and Documentation

**Files:**
- Modify: `crates/nebula-parameter/src/core/display.rs` (add docs)

**Step 1: Add module documentation**

Add comprehensive documentation at the top of `display.rs`:

```rust
//! Display condition system for conditional parameter visibility
//!
//! This module provides a complete system for controlling when parameters
//! should be displayed based on the values of other parameters.
//!
//! # Architecture
//!
//! - [`DisplayCondition`] - Atomic conditions (equals, greater than, etc.)
//! - [`DisplayRule`] - A condition applied to a specific field
//! - [`DisplayRuleSet`] - Composition of rules with AND/OR/NOT logic
//! - [`ParameterDisplay`] - Configuration for when to show/hide a parameter
//! - [`DisplayContext`] - Runtime context with parameter values
//!
//! # Examples
//!
//! ## Simple condition
//!
//! ```rust
//! use nebula_parameter::prelude::*;
//! use nebula_value::Value;
//!
//! let display = ParameterDisplay::new()
//!     .show_when_equals("auth_type", Value::text("api_key"));
//!
//! let ctx = DisplayContext::new()
//!     .with_value("auth_type", Value::text("api_key"));
//!
//! assert!(display.should_display(&ctx));
//! ```
//!
//! ## Complex conditions with AND/OR
//!
//! ```rust
//! use nebula_parameter::prelude::*;
//! use nebula_value::Value;
//!
//! let display = ParameterDisplay::new()
//!     .show_when(DisplayRuleSet::all([
//!         DisplayRule::when("enabled", DisplayCondition::IsTrue),
//!         DisplayRule::when("level", DisplayCondition::GreaterThan(10.0)),
//!     ]))
//!     .hide_when(DisplayRule::when("maintenance", DisplayCondition::IsTrue));
//! ```
```

**Step 2: Run all tests and clippy**

```bash
cargo test -p nebula-parameter
cargo clippy -p nebula-parameter -- -D warnings
```

**Step 3: Final commit**

```bash
git add -A
git commit -m "docs(nebula-parameter): add display module documentation"
```

---

## Summary

After completing all tasks:

1. **DisplayCondition** - Enum with all condition types (Equals, GreaterThan, Contains, etc.)
2. **DisplayRule** - Applies a condition to a specific field
3. **DisplayRuleSet** - Combines rules with AND/OR/NOT logic
4. **ParameterDisplay** - show_when/hide_when configuration
5. **DisplayContext** - Runtime context with parameter values
6. All parameter types have `display` field and implement `Displayable` trait
7. Full serialization support with serde
8. Comprehensive test coverage
