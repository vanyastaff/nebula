# Remove `value` Field from Parameter Types Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Separate runtime values from parameter schema definitions by removing the `value` field from all Parameter types, using the new `ParameterValues` storage instead.

**Architecture:** Parameters become pure schema definitions (metadata, options, validation rules, display conditions). Runtime values are stored externally in `ParameterValues`. This separation follows the principle that schema should not contain runtime state, making parameters lightweight, serializable, and reusable.

**Tech Stack:** Rust 2024 Edition, nebula-parameter crate, nebula-parameter-ui crate, serde for serialization

---

## Overview

Currently, each Parameter type (e.g., `TextParameter`, `NumberParameter`) contains a `value: Option<T>` field that stores runtime data alongside schema definitions. This mixing of concerns creates several issues:

1. **Conceptual confusion**: Schema (static definition) mixed with runtime state (dynamic value)
2. **Memory overhead**: Cloning parameters clones values too
3. **Serialization complexity**: Must handle optional runtime state in schema serialization
4. **Testing difficulty**: Can't easily share parameter definitions across tests

The solution is to:
1. Remove `value` field from all parameter types
2. Remove `HasValue` trait and related value-storage traits
3. Use the already-created `ParameterValues` for external value storage
4. Update UI widgets to work with external value storage

---

## Complete List of Parameter Types

**With `value` field (21 types to modify):**
1. `text.rs` - TextParameter
2. `number.rs` - NumberParameter
3. `checkbox.rs` - CheckboxParameter
4. `select.rs` - SelectParameter
5. `textarea.rs` - TextareaParameter
6. `code.rs` - CodeParameter
7. `color.rs` - ColorParameter
8. `date.rs` - DateParameter
9. `datetime.rs` - DateTimeParameter
10. `time.rs` - TimeParameter
11. `radio.rs` - RadioParameter
12. `multi_select.rs` - MultiSelectParameter
13. `hidden.rs` - HiddenParameter
14. `secret.rs` - SecretParameter
15. `file.rs` - FileParameter
16. `list.rs` - ListParameter
17. `object.rs` - ObjectParameter
18. `group.rs` - GroupParameter
19. `resource.rs` - ResourceParameter
20. `routing.rs` - RoutingParameter
21. `expirable.rs` - ExpirableParameter
22. `mode.rs` - ModeParameter

**Without `value` field (no changes needed):**
- `notice.rs` - NoticeParameter (display-only, no value)
- `panel.rs` - PanelParameter (container, no value)
- `credential.rs` - Empty placeholder

---

## Task 1: Update Core Traits - Remove HasValue

**Files:**
- Modify: `crates/nebula-parameter/src/core/traits.rs`

**Step 1: Read the current traits.rs file**

Run: Review `crates/nebula-parameter/src/core/traits.rs` to understand current structure

**Step 2: Remove HasValue trait and related code**

Remove these items from `traits.rs`:
- `HasValue` trait
- `HasValueExt` trait
- The blanket implementation `impl<T: HasValue + ?Sized> HasValueExt for T {}`

**Step 3: Update Validatable trait**

Change `Validatable` trait to not require `HasValue`:

```rust
/// Trait for parameters that support validation
#[async_trait]
pub trait Validatable: Parameter + Send + Sync {
    /// Synchronous validation (fast, local checks)
    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError>;

    /// Asynchronous validation (slow, I/O-bound checks)
    async fn validate_async(&self, _value: &Value) -> Result<(), ParameterError> {
        Ok(())
    }

    /// Complete validation (runs both sync and async)
    async fn validate(&self, value: &Value) -> Result<(), ParameterError> {
        self.validate_sync(value)?;
        self.validate_async(value).await?;
        
        if let Some(validation) = self.validation() {
            validation
                .validate(value, None)
                .await
                .map_err(|e| ParameterError::InvalidValue {
                    key: self.metadata().key.clone(),
                    reason: format!("{e}"),
                })?;
        }
        
        Ok(())
    }

    /// Get the validation configuration
    fn validation(&self) -> Option<&ParameterValidation> {
        None
    }

    /// Check if a value is considered empty for this parameter type
    fn is_empty(&self, value: &Value) -> bool {
        value.is_null()
    }
}
```

**Step 4: Update Expressible trait**

Remove the `HasValue` bound and work with `Value` directly:

```rust
#[async_trait::async_trait]
pub trait Expressible: Parameter {
    /// Check if a value is an expression string
    fn is_expression_value(&self, value: &Value) -> bool {
        if let Some(s) = value.as_text() {
            s.starts_with("{{") && s.ends_with("}}")
        } else {
            false
        }
    }

    /// Evaluate an expression value
    async fn evaluate(
        &self,
        value: &Value,
        engine: &ExpressionEngine,
        context: &EvaluationContext,
    ) -> Result<Value, ParameterError> {
        if let Some(expr) = value.as_text() {
            if expr.starts_with("{{") && expr.ends_with("}}") {
                return engine
                    .evaluate(expr, context)
                    .map_err(|e| ParameterError::InvalidValue {
                        key: self.metadata().key.clone(),
                        reason: format!("Expression evaluation failed: {e}"),
                    });
            }
        }
        Ok(value.clone())
    }
}
```

**Step 5: Update ParameterValue trait**

Change to work with external values:

```rust
/// Type-erased access to parameter operations
pub trait ParameterValue: Parameter {
    /// Validate a value against this parameter's rules
    fn validate_value(
        &self,
        value: &Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ParameterError>> + Send + '_>>;

    /// Check if a value is valid for this parameter type
    fn accepts_value(&self, value: &Value) -> bool;

    /// Get the expected value type as a string
    fn expected_type(&self) -> &'static str;

    /// Downcast to concrete type
    fn as_any(&self) -> &dyn std::any::Any;

    /// Downcast to concrete type (mutable)
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}
```

**Step 6: Run cargo check to verify changes compile**

Run: `cargo check -p nebula-parameter`
Expected: Compilation errors in parameter types (expected, we'll fix next)

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/core/traits.rs
git commit -m "refactor(nebula-parameter): remove HasValue trait from core"
```

---

## Task 2: Update TextParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/text.rs`

**Step 1: Remove value field from struct**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
#[builder(on(String, into))]
pub struct TextParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    // REMOVED: pub value: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(into)]
    pub default: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<TextParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}
```

**Step 2: Remove HasValue implementation**

Delete the entire `impl HasValue for TextParameter` block.

**Step 3: Update Validatable implementation**

```rust
impl Validatable for TextParameter {
    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        if self.is_required() && self.is_empty(value) {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
            });
        }

        if !value.is_null() && value.as_text().is_none() {
            return Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected text value".to_string(),
            });
        }

        if let Some(text) = value.as_text() {
            if let Some(opts) = &self.options {
                if let Some(min) = opts.min_length {
                    if text.len() < min {
                        return Err(ParameterError::InvalidValue {
                            key: self.metadata.key.clone(),
                            reason: format!("Text length {} below minimum {}", text.len(), min),
                        });
                    }
                }
                if let Some(max) = opts.max_length {
                    if text.len() > max {
                        return Err(ParameterError::InvalidValue {
                            key: self.metadata.key.clone(),
                            reason: format!("Text length {} above maximum {}", text.len(), max),
                        });
                    }
                }
            }
        }

        Ok(())
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null() || value.as_text().map(|s| s.is_empty()).unwrap_or(false)
    }
}
```

**Step 4: Update Expressible implementation**

```rust
#[async_trait::async_trait]
impl Expressible for TextParameter {
    // Use default implementations from trait
}
```

**Step 5: Add ParameterValue implementation**

```rust
impl ParameterValue for TextParameter {
    fn validate_value(
        &self,
        value: &Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ParameterError>> + Send + '_>> {
        Box::pin(async move { self.validate(value).await })
    }

    fn accepts_value(&self, value: &Value) -> bool {
        value.is_null() || value.as_text().is_some()
    }

    fn expected_type(&self) -> &'static str {
        "text"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
```

**Step 6: Run cargo check**

Run: `cargo check -p nebula-parameter`

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/text.rs
git commit -m "refactor(nebula-parameter): remove value field from TextParameter"
```

---

## Task 3: Update NumberParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/number.rs`

**Step 1: Remove value field from struct**

Remove `pub value: Option<f64>` from struct.

**Step 2: Remove HasValue implementation**

**Step 3: Update Validatable implementation**

```rust
impl Validatable for NumberParameter {
    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        if self.is_required() && value.is_null() {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
            });
        }

        if value.is_null() {
            return Ok(());
        }

        let num = value.as_float().or_else(|| value.as_integer().map(|i| i as f64))
            .ok_or_else(|| ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected numeric value".to_string(),
            })?;

        self.validate_number(num)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null()
    }
}
```

**Step 4: Update is_within_bounds to accept value**

```rust
pub fn is_within_bounds(&self, value: f64) -> bool {
    self.validate_number(value).is_ok()
}
```

**Step 5: Add ParameterValue implementation**

**Step 6: Run cargo check**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/number.rs
git commit -m "refactor(nebula-parameter): remove value field from NumberParameter"
```

---

## Task 4: Update CheckboxParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/checkbox.rs`

**Step 1: Remove value field**

Remove `pub value: Option<Boolean>` from struct.

**Step 2: Remove HasValue implementation**

**Step 3: Update Validatable implementation**

```rust
impl Validatable for CheckboxParameter {
    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        if self.is_required() && value.is_null() {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
            });
        }

        if !value.is_null() && value.as_bool().is_none() {
            return Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected boolean value".to_string(),
            });
        }

        Ok(())
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null()
    }
}
```

**Step 4: Add ParameterValue implementation**

**Step 5: Run cargo check**

**Step 6: Commit**

```bash
git add crates/nebula-parameter/src/types/checkbox.rs
git commit -m "refactor(nebula-parameter): remove value field from CheckboxParameter"
```

---

## Task 5: Update SelectParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/select.rs`

**Step 1: Remove value field**

**Step 2: Remove HasValue implementation**

**Step 3: Update Validatable implementation**

**Step 4: Update get_display_name to take value as parameter**

```rust
pub fn get_display_name(&self, value: &str) -> Option<String> {
    self.get_option_by_value(value)
        .map(|option| option.name.clone())
}
```

**Step 5: Add ParameterValue implementation**

**Step 6: Run cargo check**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/select.rs
git commit -m "refactor(nebula-parameter): remove value field from SelectParameter"
```

---

## Task 6: Update TextareaParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/textarea.rs`

Follow same pattern as TextParameter.

**Step 1: Remove value field**

**Step 2: Remove HasValue implementation**

**Step 3: Update Validatable implementation**

**Step 4: Add ParameterValue implementation**

**Step 5: Run cargo check**

**Step 6: Commit**

```bash
git add crates/nebula-parameter/src/types/textarea.rs
git commit -m "refactor(nebula-parameter): remove value field from TextareaParameter"
```

---

## Task 7: Update CodeParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/code.rs`

Follow same pattern as TextParameter.

**Step 1-6: Same steps as previous tasks**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/code.rs
git commit -m "refactor(nebula-parameter): remove value field from CodeParameter"
```

---

## Task 8: Update ColorParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/color.rs`

**Step 1-6: Same steps as previous tasks**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/color.rs
git commit -m "refactor(nebula-parameter): remove value field from ColorParameter"
```

---

## Task 9: Update DateParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/date.rs`

**Step 1-6: Same steps as previous tasks**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/date.rs
git commit -m "refactor(nebula-parameter): remove value field from DateParameter"
```

---

## Task 10: Update DateTimeParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/datetime.rs`

**Step 1-6: Same steps as previous tasks**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/datetime.rs
git commit -m "refactor(nebula-parameter): remove value field from DateTimeParameter"
```

---

## Task 11: Update TimeParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/time.rs`

**Step 1-6: Same steps as previous tasks**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/time.rs
git commit -m "refactor(nebula-parameter): remove value field from TimeParameter"
```

---

## Task 12: Update RadioParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/radio.rs`

**Step 1-6: Same steps as SelectParameter**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/radio.rs
git commit -m "refactor(nebula-parameter): remove value field from RadioParameter"
```

---

## Task 13: Update MultiSelectParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/multi_select.rs`

Note: Value type is array/collection.

**Step 1-6: Same steps, but validate as array**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/multi_select.rs
git commit -m "refactor(nebula-parameter): remove value field from MultiSelectParameter"
```

---

## Task 14: Update HiddenParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/hidden.rs`

**Step 1-6: Same steps as previous tasks**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/hidden.rs
git commit -m "refactor(nebula-parameter): remove value field from HiddenParameter"
```

---

## Task 15: Update SecretParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/secret.rs`

**Step 1-6: Same steps as TextParameter**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/secret.rs
git commit -m "refactor(nebula-parameter): remove value field from SecretParameter"
```

---

## Task 16: Update FileParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/file.rs`

Note: Value type is FileReference (custom struct).

**Step 1-6: Same steps, validate FileReference from Value**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/file.rs
git commit -m "refactor(nebula-parameter): remove value field from FileParameter"
```

---

## Task 17: Update ListParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/list.rs`

Note: Value type is Array. Has child parameter items.

**Step 1-6: Same steps, validate as array with item validation**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/list.rs
git commit -m "refactor(nebula-parameter): remove value field from ListParameter"
```

---

## Task 18: Update ObjectParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/object.rs`

Note: Value type is ObjectValue. Has child properties.

**Step 1-6: Same steps, validate as object with property validation**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/object.rs
git commit -m "refactor(nebula-parameter): remove value field from ObjectParameter"
```

---

## Task 19: Update GroupParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/group.rs`

Note: Value type is GroupValue. Container for grouped fields.

**Step 1-6: Same steps, validate grouped fields**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/group.rs
git commit -m "refactor(nebula-parameter): remove value field from GroupParameter"
```

---

## Task 20: Update ResourceParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/resource.rs`

Note: Value type is ResourceValue.

**Step 1-6: Same steps**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/resource.rs
git commit -m "refactor(nebula-parameter): remove value field from ResourceParameter"
```

---

## Task 21: Update RoutingParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/routing.rs`

Note: Value type is RoutingValue.

**Step 1-6: Same steps**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/routing.rs
git commit -m "refactor(nebula-parameter): remove value field from RoutingParameter"
```

---

## Task 22: Update ExpirableParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/expirable.rs`

Note: Value type is ExpirableValue (value with TTL).

**Step 1-6: Same steps**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/expirable.rs
git commit -m "refactor(nebula-parameter): remove value field from ExpirableParameter"
```

---

## Task 23: Update ModeParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/mode.rs`

Note: Value type is ModeValue.

**Step 1-6: Same steps**

**Step 7: Commit**

```bash
git add crates/nebula-parameter/src/types/mode.rs
git commit -m "refactor(nebula-parameter): remove value field from ModeParameter"
```

---

## Task 24: Update ParameterCollection

**Files:**
- Modify: `crates/nebula-parameter/src/core/collection.rs`

**Step 1: Remove value-related methods**

Remove or update:
- `value()` method
- `typed_value()` method
- `snapshot()` / `restore()` - move to use ParameterValues

**Step 2: Update validate_all to accept ParameterValues**

```rust
pub async fn validate_all(&self, values: &ParameterValues) -> ValidationResult {
    let mut errors = Vec::new();

    for key in self.topological_sort() {
        if let Some(param) = self.parameters.get(&key) {
            let value = values.get(key.clone()).cloned().unwrap_or(Value::Null);
            if let Err(e) = param.validate_value(&value).await {
                errors.push((key.clone(), e));
            }
        }
    }

    if errors.is_empty() {
        ValidationResult::Valid
    } else {
        ValidationResult::Invalid(errors)
    }
}
```

**Step 3: Run cargo check**

**Step 4: Commit**

```bash
git add crates/nebula-parameter/src/core/collection.rs
git commit -m "refactor(nebula-parameter): update ParameterCollection for external values"
```

---

## Task 25: Update Core Module Exports

**Files:**
- Modify: `crates/nebula-parameter/src/core/mod.rs`
- Modify: `crates/nebula-parameter/src/lib.rs`

**Step 1: Add values module export in core/mod.rs**

```rust
pub mod values;
pub use values::{ParameterValues, ParameterSnapshot, ParameterDiff};
```

**Step 2: Update lib.rs prelude**

Add ParameterValues to prelude exports.

**Step 3: Run cargo check**

**Step 4: Commit**

```bash
git add crates/nebula-parameter/src/core/mod.rs crates/nebula-parameter/src/lib.rs
git commit -m "refactor(nebula-parameter): export ParameterValues from core"
```

---

## Task 26: Update Tests

**Files:**
- Modify: `crates/nebula-parameter/src/core/collection.rs` (tests)
- Modify: `crates/nebula-parameter/src/core/values.rs` (tests)

**Step 1: Update collection tests**

```rust
#[tokio::test]
async fn test_validate_with_values() {
    let collection = ParameterCollection::new()
        .with(TextParameter::builder()
            .metadata(ParameterMetadata::builder()
                .key("name")
                .name("Name")
                .description("")
                .required(true)
                .build()
                .unwrap())
            .build());

    let mut values = ParameterValues::new();
    values.set(key("name"), Value::text("Alice"));

    let result = collection.validate_all(&values).await;
    assert!(result.is_valid());
}
```

**Step 2: Run all tests**

Run: `cargo test -p nebula-parameter`

**Step 3: Fix any failures**

**Step 4: Commit**

```bash
git add crates/nebula-parameter/
git commit -m "test(nebula-parameter): update tests for value-less parameters"
```

---

## Task 27: Final Verification

**Step 1: Run cargo check on workspace**

Run: `cargo check --workspace`

**Step 2: Run cargo clippy**

Run: `cargo clippy --workspace -- -D warnings`

**Step 3: Run cargo fmt**

Run: `cargo fmt --all`

**Step 4: Run all tests**

Run: `cargo test --workspace`

**Step 5: Final commit**

```bash
git add .
git commit -m "refactor(nebula-parameter): complete separation of values from schema"
```

---

## Summary

After completing all 27 tasks:

**Parameter Types Updated (22 types):**
1. TextParameter
2. NumberParameter
3. CheckboxParameter
4. SelectParameter
5. TextareaParameter
6. CodeParameter
7. ColorParameter
8. DateParameter
9. DateTimeParameter
10. TimeParameter
11. RadioParameter
12. MultiSelectParameter
13. HiddenParameter
14. SecretParameter
15. FileParameter
16. ListParameter
17. ObjectParameter
18. GroupParameter
19. ResourceParameter
20. RoutingParameter
21. ExpirableParameter
22. ModeParameter

**Benefits Achieved:**
- Parameters are pure schema definitions (no runtime state)
- Values stored externally in `ParameterValues`
- Clean separation of concerns
- Lightweight, serializable parameters
- Same parameter definition reusable across contexts

---

## Future Work (Separate Plan)

**UI Widgets** - Update `nebula-parameter-ui` widgets to work with external values:
- Update `ParameterWidget` trait to accept `value: Option<&Value>`
- Update `WidgetResponse` to include `new_value: Option<Value>`
- Update all 17 widget implementations
