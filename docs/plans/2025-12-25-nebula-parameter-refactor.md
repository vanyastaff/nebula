# nebula-parameter Enterprise Refactor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Refactor nebula-parameter crate to world-class enterprise-grade, idiomatic Rust 2024+ patterns with SOLID, DRY, KISS principles.

**Architecture:** Maintain the excellent Schema/Values/State/Context separation while improving type safety, removing code duplication, completing unimplemented features, and adopting modern Rust 2024 idioms. Breaking changes are allowed.

**Tech Stack:** Rust 2024 Edition (1.90+), async-trait, bon, thiserror, serde, tokio, bitflags, downcast-rs

---

## Analysis Summary

### Current Strengths (Keep)
1. Excellent separation: Schema (`ParameterCollection`) / Values (`ParameterValues`) / State (`ParameterState`) / Context (`ParameterContext`)
2. Event-driven reactive architecture with broadcast channels
3. Comprehensive display condition system (18 condition types)
4. Type-safe validation with nebula-validator integration
5. Snapshot/restore for undo/redo
6. Extensive test coverage (~100+ tests)

### Issues to Fix
1. **Incomplete implementations**: `get_validatable()` and `get_displayable()` have `// TODO: Implement` stubs
2. **Code duplication**: All 25 parameter types repeat the same trait implementations
3. **is_empty() footgun**: Default returns `false`, silently passes validation for empty strings/arrays
4. **Expressible trait unused**: Defined but no types implement it
5. **Validation code duplication**: `validate_sync()` duplicates base trait logic in TextParameter
6. **Missing `#[inline]`**: Trivial accessor methods lack inline hints
7. **No const fn**: Where applicable for compile-time evaluation
8. **Builder API inconsistency**: Some use `bon`, some manual patterns

---

## Task 1: Fix ParameterCollection Trait Accessors

**Files:**
- Modify: `crates/nebula-parameter/src/core/collection.rs:59-67`
- Test: `crates/nebula-parameter/tests/collection_test.rs`

**Step 1: Write failing tests for get_validatable and get_displayable**

Add to `tests/collection_test.rs`:

```rust
#[test]
fn test_get_validatable() {
    let mut collection = ParameterCollection::new();
    collection.add(
        TextParameter::builder()
            .metadata(
                ParameterMetadata::builder()
                    .key("email")
                    .name("Email")
                    .description("User email")
                    .required(true)
                    .build()
                    .unwrap(),
            )
            .build(),
    );

    let validatable = collection.get_validatable(key("email"));
    assert!(validatable.is_some());
    
    let validatable = collection.get_validatable(key("nonexistent"));
    assert!(validatable.is_none());
}

#[test]
fn test_get_displayable() {
    let mut collection = ParameterCollection::new();
    collection.add(
        TextParameter::builder()
            .metadata(
                ParameterMetadata::builder()
                    .key("name")
                    .name("Name")
                    .description("")
                    .build()
                    .unwrap(),
            )
            .display(ParameterDisplay::new().show_when_true(key("enabled")))
            .build(),
    );

    let displayable = collection.get_displayable(key("name"));
    assert!(displayable.is_some());
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p nebula-parameter test_get_validatable test_get_displayable -- --nocapture`
Expected: FAIL with compilation error (TODO stub doesn't return proper type)

**Step 3: Implement get_validatable and get_displayable**

In `crates/nebula-parameter/src/core/collection.rs`, replace the TODO stubs:

```rust
/// Get a parameter as Validatable if it implements the trait
pub fn get_validatable(&self, key: impl Into<ParameterKey>) -> Option<&dyn Validatable> {
    self.parameters
        .get(&key.into())
        .and_then(|p| p.as_any().downcast_ref::<dyn Validatable>())
}

/// Get a parameter as Displayable if it implements the trait  
pub fn get_displayable(&self, key: impl Into<ParameterKey>) -> Option<&dyn Displayable> {
    self.parameters
        .get(&key.into())
        .and_then(|p| p.as_any().downcast_ref::<dyn Displayable>())
}
```

Wait - this won't work because we can't downcast `dyn Parameter` to `dyn Validatable`. We need a different approach.

**Step 3 (Revised): Store parameters as trait objects that implement all required traits**

Create a new supertrait that combines all parameter traits:

```rust
/// Supertrait combining all parameter capabilities
pub trait FullParameter: Parameter + Validatable + Displayable {}

// Blanket implementation
impl<T: Parameter + Validatable + Displayable> FullParameter for T {}
```

Then modify `ParameterCollection` to store `Box<dyn FullParameter>`:

```rust
pub struct ParameterCollection {
    parameters: HashMap<ParameterKey, Box<dyn FullParameter>>,
    dependencies: HashMap<ParameterKey, Vec<ParameterKey>>,
}

impl ParameterCollection {
    pub fn get_validatable(&self, key: impl Into<ParameterKey>) -> Option<&dyn Validatable> {
        self.parameters.get(&key.into()).map(|p| p.as_ref() as &dyn Validatable)
    }

    pub fn get_displayable(&self, key: impl Into<ParameterKey>) -> Option<&dyn Displayable> {
        self.parameters.get(&key.into()).map(|p| p.as_ref() as &dyn Displayable)
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p nebula-parameter -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/nebula-parameter/src/core/collection.rs crates/nebula-parameter/tests/collection_test.rs
git commit -m "feat(nebula-parameter): implement get_validatable and get_displayable with FullParameter supertrait"
```

---

## Task 2: Create Parameter Derive Macro Infrastructure

**Files:**
- Create: `crates/nebula-parameter/src/core/base.rs`
- Modify: `crates/nebula-parameter/src/core/mod.rs`
- Test: inline tests

**Step 1: Define BaseParameter struct to eliminate duplication**

Create `crates/nebula-parameter/src/core/base.rs`:

```rust
//! Base parameter structure to reduce code duplication across parameter types.

use serde::{Deserialize, Serialize};

use crate::core::{ParameterDisplay, ParameterMetadata, ParameterValidation};

/// Common fields shared by all parameter types.
///
/// This struct contains the fields that every parameter type needs,
/// eliminating duplication across the 25+ parameter implementations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterBase {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

impl ParameterBase {
    /// Create a new base with just metadata
    #[must_use]
    pub fn new(metadata: ParameterMetadata) -> Self {
        Self {
            metadata,
            display: None,
            validation: None,
        }
    }

    /// Builder: set display configuration
    #[must_use]
    pub fn with_display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }

    /// Builder: set validation configuration
    #[must_use]
    pub fn with_validation(mut self, validation: ParameterValidation) -> Self {
        self.validation = Some(validation);
        self
    }
}
```

**Step 2: Run cargo check to verify compilation**

Run: `cargo check -p nebula-parameter`
Expected: PASS

**Step 3: Add module to mod.rs**

In `crates/nebula-parameter/src/core/mod.rs`, add:

```rust
mod base;
pub use base::ParameterBase;
```

**Step 4: Commit**

```bash
git add crates/nebula-parameter/src/core/base.rs crates/nebula-parameter/src/core/mod.rs
git commit -m "feat(nebula-parameter): add ParameterBase to reduce code duplication"
```

---

## Task 3: Refactor TextParameter to Use ParameterBase

**Files:**
- Modify: `crates/nebula-parameter/src/types/text.rs`
- Test: existing tests should still pass

**Step 1: Refactor TextParameter to use ParameterBase**

```rust
use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, Parameter, ParameterBase, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for single-line text input
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
#[builder(on(String, into))]
pub struct TextParameter {
    #[serde(flatten)]
    #[builder(into)]
    pub base: ParameterBase,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(into)]
    pub default: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<TextParameterOptions>,
}

// ... rest of TextParameterOptions stays the same ...

impl Parameter for TextParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Text
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.base.metadata
    }
}

impl Validatable for TextParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::String)
    }

    // Remove validate_sync override - use default implementation
    // Only keep the is_empty and validation overrides

    fn validation(&self) -> Option<&ParameterValidation> {
        self.base.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null() || value.as_text().is_some_and(|s| s.is_empty())
    }
}

impl Displayable for TextParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.base.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.base.display = display;
    }
}
```

**Step 2: Run tests to verify nothing broke**

Run: `cargo test -p nebula-parameter -- --nocapture`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/nebula-parameter/src/types/text.rs
git commit -m "refactor(nebula-parameter): TextParameter uses ParameterBase, removes duplication"
```

---

## Task 4: Add Validation Options to ParameterBase

**Files:**
- Modify: `crates/nebula-parameter/src/core/traits.rs`
- Test: `crates/nebula-parameter/src/core/traits.rs` (inline tests)

**Step 1: Write failing test for validate_sync with min/max length**

Add test to `crates/nebula-parameter/src/core/traits.rs`:

```rust
#[cfg(test)]
mod validation_tests {
    use super::*;
    
    #[tokio::test]
    async fn test_validate_sync_calls_expected_kind() {
        // Test that default validate_sync properly checks expected_kind
        // This test ensures we don't duplicate logic in each parameter type
    }
}
```

**Step 2: Ensure default validate_sync handles all common cases**

The default `validate_sync` in `Validatable` trait already handles:
1. Type checking via `expected_kind()`
2. Required field check via `is_empty()` and `is_required()`

Parameter types should NOT override `validate_sync` unless they have type-specific validation (like min_length/max_length for text).

**Step 3: Add extension trait for common validations**

Create `crates/nebula-parameter/src/core/validate_ext.rs`:

```rust
//! Extension methods for common validation patterns.

use crate::core::ParameterError;
use nebula_core::ParameterKey;
use nebula_value::Value;

/// Extension trait for string validation
pub trait StringValidationExt {
    /// Validate string length constraints
    fn validate_string_length(
        &self,
        value: &Value,
        key: &ParameterKey,
        min_length: Option<usize>,
        max_length: Option<usize>,
    ) -> Result<(), ParameterError> {
        if let Some(text) = value.as_text() {
            let len = text.len();
            
            if let Some(min) = min_length {
                if len < min {
                    return Err(ParameterError::InvalidValue {
                        key: key.clone(),
                        reason: format!("Text length {len} below minimum {min}"),
                    });
                }
            }
            
            if let Some(max) = max_length {
                if len > max {
                    return Err(ParameterError::InvalidValue {
                        key: key.clone(),
                        reason: format!("Text length {len} above maximum {max}"),
                    });
                }
            }
        }
        Ok(())
    }

    /// Validate string pattern
    fn validate_string_pattern(
        &self,
        value: &Value,
        key: &ParameterKey,
        pattern: Option<&str>,
    ) -> Result<(), ParameterError> {
        if let (Some(text), Some(pat)) = (value.as_text(), pattern) {
            let regex = regex::Regex::new(pat).map_err(|e| ParameterError::InvalidValue {
                key: key.clone(),
                reason: format!("Invalid regex pattern: {e}"),
            })?;
            
            if !regex.is_match(text.as_str()) {
                return Err(ParameterError::InvalidValue {
                    key: key.clone(),
                    reason: format!("Value does not match pattern: {pat}"),
                });
            }
        }
        Ok(())
    }
}

// Blanket implementation for all types
impl<T> StringValidationExt for T {}
```

**Step 4: Use in TextParameter validate_sync**

```rust
impl Validatable for TextParameter {
    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Base validation (type check + required)
        self.validate_base(value)?;
        
        // Type-specific validation
        if let Some(opts) = &self.options {
            self.validate_string_length(
                value,
                &self.base.metadata.key,
                opts.min_length,
                opts.max_length,
            )?;
            self.validate_string_pattern(
                value,
                &self.base.metadata.key,
                opts.pattern.as_deref(),
            )?;
        }
        
        Ok(())
    }
}
```

**Step 5: Commit**

```bash
git add crates/nebula-parameter/src/core/validate_ext.rs crates/nebula-parameter/src/types/text.rs
git commit -m "refactor(nebula-parameter): extract string validation to reusable extension trait"
```

---

## Task 5: Fix is_empty Default and Add Tests

**Files:**
- Modify: `crates/nebula-parameter/src/core/traits.rs:160-175`
- Create: `crates/nebula-parameter/tests/is_empty_test.rs`

**Step 1: Create comprehensive is_empty test matrix**

Create `tests/is_empty_test.rs`:

```rust
//! Tests for is_empty behavior across all parameter types

use nebula_parameter::prelude::*;
use nebula_value::Value;

fn create_required_text() -> TextParameter {
    TextParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("test")
                .name("Test")
                .description("")
                .required(true)
                .build()
                .unwrap(),
        )
        .build()
}

#[test]
fn test_text_parameter_is_empty() {
    let param = create_required_text();
    
    // Empty string should be empty
    assert!(param.is_empty(&Value::text("")));
    
    // Null should be empty
    assert!(param.is_empty(&Value::Null));
    
    // Non-empty string should not be empty
    assert!(!param.is_empty(&Value::text("hello")));
    
    // Whitespace-only should NOT be empty (that's a validation rule, not emptiness)
    assert!(!param.is_empty(&Value::text("   ")));
}

#[tokio::test]
async fn test_required_text_validation_rejects_empty() {
    let param = create_required_text();
    
    // Empty string should fail validation
    let result = param.validate(&Value::text("")).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ParameterError::MissingValue { .. }));
    
    // Null should fail validation
    let result = param.validate(&Value::Null).await;
    assert!(result.is_err());
    
    // Non-empty should pass
    let result = param.validate(&Value::text("hello")).await;
    assert!(result.is_ok());
}

#[test]
fn test_textarea_is_empty() {
    let param = TextareaParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("content")
                .name("Content")
                .description("")
                .build()
                .unwrap(),
        )
        .build();
    
    assert!(param.is_empty(&Value::text("")));
    assert!(!param.is_empty(&Value::text("content")));
}

#[test]
fn test_list_parameter_is_empty() {
    let param = ListParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("items")
                .name("Items")
                .description("")
                .build()
                .unwrap(),
        )
        .build();
    
    assert!(param.is_empty(&Value::array_empty()));
    assert!(!param.is_empty(&Value::array(vec![Value::integer(1)])));
}

#[test]
fn test_object_parameter_is_empty() {
    let param = ObjectParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("data")
                .name("Data")
                .description("")
                .build()
                .unwrap(),
        )
        .build();
    
    assert!(param.is_empty(&Value::object_empty()));
    // Non-empty object should not be empty
}

#[test]
fn test_number_parameter_is_empty() {
    let param = NumberParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("count")
                .name("Count")
                .description("")
                .build()
                .unwrap(),
        )
        .build();
    
    // Numbers don't have an "empty" concept - only null
    assert!(param.is_empty(&Value::Null));
    assert!(!param.is_empty(&Value::integer(0)));
    assert!(!param.is_empty(&Value::float(0.0)));
}
```

**Step 2: Run tests to verify current behavior**

Run: `cargo test -p nebula-parameter is_empty -- --nocapture`
Expected: Some may fail if is_empty not properly implemented in all types

**Step 3: Fix any parameter types that don't properly implement is_empty**

Check each text-like and collection parameter type:
- TextParameter: ✓ (already implemented)
- TextareaParameter: verify/add
- SecretParameter: verify/add
- ListParameter: verify/add
- ObjectParameter: verify/add

**Step 4: Run tests to verify all pass**

Run: `cargo test -p nebula-parameter is_empty -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/nebula-parameter/tests/is_empty_test.rs crates/nebula-parameter/src/types/
git commit -m "test(nebula-parameter): comprehensive is_empty test matrix, fix implementations"
```

---

## Task 6: Add #[inline] to Trivial Methods

**Files:**
- Modify: `crates/nebula-parameter/src/core/traits.rs`
- Modify: `crates/nebula-parameter/src/core/metadata.rs`
- Modify: `crates/nebula-parameter/src/core/state.rs`
- Modify: `crates/nebula-parameter/src/core/values.rs`

**Step 1: Add #[inline] to accessor methods in traits.rs**

```rust
impl_downcast!(Parameter);

// In Parameter trait
#[inline]
fn key(&self) -> &str {
    self.metadata().key.as_str()
}

#[inline]
fn name(&self) -> &str {
    &self.metadata().name
}

#[inline]
fn is_required(&self) -> bool {
    self.metadata().required
}
```

**Step 2: Add #[inline] to all accessor methods in state.rs**

All the `is_*` methods and simple flag checks should have `#[inline]`:

```rust
#[inline]
pub fn is_dirty(&self) -> bool {
    self.has_flag(ParameterFlags::DIRTY)
}

#[inline]  
pub fn is_touched(&self) -> bool {
    self.has_flag(ParameterFlags::TOUCHED)
}
// ... etc for all simple accessors
```

**Step 3: Add #[inline] to accessor methods in metadata.rs**

Already has `#[inline]` on most methods - verify completeness.

**Step 4: Add #[inline] to accessor methods in values.rs**

```rust
#[inline]
pub fn len(&self) -> usize {
    self.values.len()
}

#[inline]
pub fn is_empty(&self) -> bool {
    self.values.is_empty()
}

#[inline]
pub fn contains(&self, key: impl Into<ParameterKey>) -> bool {
    self.values.contains_key(&key.into())
}
```

**Step 5: Run cargo clippy to verify no issues**

Run: `cargo clippy -p nebula-parameter -- -D warnings`
Expected: PASS

**Step 6: Commit**

```bash
git add crates/nebula-parameter/src/core/
git commit -m "perf(nebula-parameter): add #[inline] to trivial accessor methods"
```

---

## Task 7: Implement Expressible Trait for TextParameter

**Files:**
- Modify: `crates/nebula-parameter/src/types/text.rs`
- Create: `crates/nebula-parameter/tests/expression_test.rs`

**Step 1: Write failing test for expression evaluation**

Create `tests/expression_test.rs`:

```rust
//! Tests for expression evaluation in parameters

use nebula_parameter::prelude::*;
use nebula_parameter::core::Expressible;
use nebula_expression::{ExpressionEngine, EvaluationContext};
use nebula_value::Value;

fn create_text_param() -> TextParameter {
    TextParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("greeting")
                .name("Greeting")
                .description("")
                .build()
                .unwrap(),
        )
        .build()
}

#[test]
fn test_is_expression_value() {
    let param = create_text_param();
    
    // Expression syntax detected
    assert!(param.is_expression_value(&Value::text("{{ $input.name }}")));
    assert!(param.is_expression_value(&Value::text("Hello {{ $input.name }}!")));
    
    // Not expressions
    assert!(!param.is_expression_value(&Value::text("Hello World")));
    assert!(!param.is_expression_value(&Value::text("")));
    assert!(!param.is_expression_value(&Value::integer(42)));
}

#[tokio::test]
async fn test_evaluate_concrete_value() {
    let param = create_text_param();
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();
    
    // Concrete values return unchanged
    let value = Value::text("Hello World");
    let result = param.evaluate(&value, &engine, &context).await.unwrap();
    assert_eq!(result, value);
}

#[tokio::test]
async fn test_evaluate_expression() {
    let param = create_text_param();
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set("input", serde_json::json!({"name": "Alice"}));
    
    let value = Value::text("{{ $input.name }}");
    let result = param.evaluate(&value, &engine, &context).await.unwrap();
    assert_eq!(result, Value::text("Alice"));
}
```

**Step 2: Implement Expressible for TextParameter**

In `crates/nebula-parameter/src/types/text.rs`:

```rust
use crate::core::{Expressible, ParameterError};
use nebula_expression::{ExpressionEngine, EvaluationContext};

#[async_trait::async_trait]
impl Expressible for TextParameter {
    fn is_expression_value(&self, value: &Value) -> bool {
        value
            .as_text()
            .is_some_and(|s| s.contains("{{") && s.contains("}}"))
    }

    async fn evaluate(
        &self,
        value: &Value,
        engine: &ExpressionEngine,
        context: &EvaluationContext,
    ) -> Result<Value, ParameterError> {
        // If not an expression, return as-is
        if !self.is_expression_value(value) {
            return Ok(value.clone());
        }

        // Extract string and evaluate
        let text = value.as_text().ok_or_else(|| ParameterError::InvalidType {
            key: self.metadata().key.clone(),
            expected_type: "String".to_string(),
            actual_details: value.kind().name().to_string(),
        })?;

        engine
            .evaluate(text.as_str(), context)
            .map_err(|e| ParameterError::InvalidValue {
                key: self.metadata().key.clone(),
                reason: format!("Expression evaluation failed: {e}"),
            })
    }
}
```

**Step 3: Run tests**

Run: `cargo test -p nebula-parameter expression -- --nocapture`
Expected: PASS (or identify nebula-expression API adjustments needed)

**Step 4: Commit**

```bash
git add crates/nebula-parameter/src/types/text.rs crates/nebula-parameter/tests/expression_test.rs
git commit -m "feat(nebula-parameter): implement Expressible trait for TextParameter"
```

---

## Task 8: Add FullParameter Supertrait with Downcast Support

**Files:**
- Modify: `crates/nebula-parameter/src/core/traits.rs`
- Modify: `crates/nebula-parameter/src/core/collection.rs`
- Test: existing collection tests

**Step 1: Create FullParameter supertrait**

Add to `crates/nebula-parameter/src/core/traits.rs`:

```rust
use downcast_rs::{Downcast, impl_downcast};

/// Combined trait for fully-featured parameters.
///
/// This supertrait combines all the capabilities a parameter can have:
/// - `Parameter`: Core identification and metadata
/// - `Validatable`: Value validation
/// - `Displayable`: Conditional visibility
///
/// All parameter types in this crate implement this trait, enabling
/// type-safe storage in `ParameterCollection`.
pub trait FullParameter: Parameter + Validatable + Displayable + Downcast + Send + Sync {}

impl_downcast!(FullParameter);

// Blanket implementation for all qualifying types
impl<T> FullParameter for T where T: Parameter + Validatable + Displayable + Send + Sync + 'static {}
```

**Step 2: Update ParameterCollection to use FullParameter**

In `crates/nebula-parameter/src/core/collection.rs`:

```rust
use crate::core::{Displayable, FullParameter, Parameter, Validatable};

pub struct ParameterCollection {
    parameters: HashMap<ParameterKey, Box<dyn FullParameter>>,
    dependencies: HashMap<ParameterKey, Vec<ParameterKey>>,
}

impl ParameterCollection {
    pub fn add<P>(&mut self, param: P) -> &mut Self
    where
        P: FullParameter + 'static,
    {
        let key = param.metadata().key.clone();

        if let Some(display) = param.display() {
            let deps = display.dependencies();
            if !deps.is_empty() {
                self.dependencies.insert(key.clone(), deps);
            }
        }

        self.parameters.insert(key, Box::new(param));
        self
    }

    pub fn get_validatable(&self, key: impl Into<ParameterKey>) -> Option<&dyn Validatable> {
        self.parameters
            .get(&key.into())
            .map(|p| p.as_ref() as &dyn Validatable)
    }

    pub fn get_displayable(&self, key: impl Into<ParameterKey>) -> Option<&dyn Displayable> {
        self.parameters
            .get(&key.into())
            .map(|p| p.as_ref() as &dyn Displayable)
    }
}
```

**Step 3: Run all tests**

Run: `cargo test -p nebula-parameter -- --nocapture`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/nebula-parameter/src/core/traits.rs crates/nebula-parameter/src/core/collection.rs
git commit -m "feat(nebula-parameter): add FullParameter supertrait for type-safe collection storage"
```

---

## Task 9: Refactor All Parameter Types to Use ParameterBase

**Files:**
- Modify: All files in `crates/nebula-parameter/src/types/`

This is a large refactoring task. Apply the same pattern used for TextParameter to all other parameter types:

1. Replace individual `metadata`, `display`, `validation` fields with `base: ParameterBase`
2. Update `Parameter` impl to delegate to `self.base.metadata`
3. Update `Validatable` impl to use `self.base.validation.as_ref()`
4. Update `Displayable` impl to use `self.base.display`
5. Remove duplicated validation logic, use default trait implementations where possible

**Files to modify:**
- `textarea.rs`
- `secret.rs`
- `number.rs`
- `checkbox.rs`
- `select.rs`
- `multi_select.rs`
- `radio.rs`
- `date.rs`
- `time.rs`
- `datetime.rs`
- `code.rs`
- `color.rs`
- `file.rs`
- `group.rs`
- `object.rs`
- `list.rs`
- `hidden.rs`
- `notice.rs`
- `panel.rs`
- `resource.rs`
- `routing.rs`
- `mode.rs`
- `expirable.rs`

**For each file:**
1. Replace fields with `base: ParameterBase`
2. Update trait impls
3. Run tests
4. Commit

This should be done in batches of 4-5 files per commit to keep changes reviewable.

**Commit pattern:**

```bash
git add crates/nebula-parameter/src/types/{textarea,secret,number,checkbox}.rs
git commit -m "refactor(nebula-parameter): textarea, secret, number, checkbox use ParameterBase"
```

---

## Task 10: Clean Up Prelude and Public API

**Files:**
- Modify: `crates/nebula-parameter/src/lib.rs`
- Modify: `crates/nebula-parameter/src/core/mod.rs`

**Step 1: Review and organize prelude exports**

Ensure prelude contains only the most commonly needed types:

```rust
pub mod prelude {
    // Core traits
    pub use crate::core::{
        Displayable, FullParameter, Parameter, Validatable,
    };
    
    // Core types
    pub use crate::core::{
        ParameterBase, ParameterCollection, ParameterContext, ParameterError,
        ParameterKind, ParameterMetadata, ParameterValues,
    };
    
    // Display system
    pub use crate::core::{
        DisplayCondition, DisplayContext, DisplayRule, DisplayRuleSet,
        ParameterDisplay,
    };
    
    // Validation
    pub use crate::core::ParameterValidation;
    
    // State management
    pub use crate::core::{ParameterFlags, ParameterSnapshot, ParameterState};
    
    // All parameter types
    pub use crate::types::*;
    
    // Re-exports
    pub use nebula_core::ParameterKey;
    pub use nebula_value::ValueKind;
}
```

**Step 2: Verify all public types are documented**

Run: `cargo doc -p nebula-parameter --no-deps`
Expected: No warnings about missing docs

**Step 3: Commit**

```bash
git add crates/nebula-parameter/src/lib.rs crates/nebula-parameter/src/core/mod.rs
git commit -m "refactor(nebula-parameter): clean up prelude and public API exports"
```

---

## Task 11: Add Async Validation Examples

**Files:**
- Create: `crates/nebula-parameter/examples/async_validation.rs`
- Modify: `crates/nebula-parameter/Cargo.toml`

**Step 1: Create example demonstrating async validation**

Create `examples/async_validation.rs`:

```rust
//! Example: Async validation for database uniqueness check
//!
//! This example shows how to implement async validation for scenarios like:
//! - Database uniqueness checks
//! - External API validation
//! - Rate limiting checks

use nebula_parameter::prelude::*;
use nebula_value::Value;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Simulated database for username uniqueness
struct FakeDatabase {
    usernames: RwLock<HashSet<String>>,
}

impl FakeDatabase {
    fn new() -> Self {
        let mut usernames = HashSet::new();
        usernames.insert("admin".to_string());
        usernames.insert("root".to_string());
        Self {
            usernames: RwLock::new(usernames),
        }
    }

    async fn is_username_taken(&self, username: &str) -> bool {
        // Simulate async database call
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        self.usernames.read().await.contains(username)
    }
}

/// Custom parameter with async validation
struct UniqueUsernameParameter {
    base: ParameterBase,
    db: Arc<FakeDatabase>,
}

impl Parameter for UniqueUsernameParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Text
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.base.metadata
    }
}

#[async_trait::async_trait]
impl Validatable for UniqueUsernameParameter {
    fn expected_kind(&self) -> Option<nebula_value::ValueKind> {
        Some(nebula_value::ValueKind::String)
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null() || value.as_text().is_some_and(|s| s.is_empty())
    }

    async fn validate_async(&self, value: &Value) -> Result<(), ParameterError> {
        if let Some(username) = value.as_text() {
            if self.db.is_username_taken(username.as_str()).await {
                return Err(ParameterError::validation(
                    self.metadata().key.clone(),
                    format!("Username '{}' is already taken", username),
                ));
            }
        }
        Ok(())
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.base.validation.as_ref()
    }
}

impl Displayable for UniqueUsernameParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.base.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.base.display = display;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Arc::new(FakeDatabase::new());
    
    let param = UniqueUsernameParameter {
        base: ParameterBase::new(
            ParameterMetadata::builder()
                .key("username")
                .name("Username")
                .description("Choose a unique username")
                .required(true)
                .build()?,
        ),
        db,
    };

    // Test with available username
    let result = param.validate(&Value::text("newuser")).await;
    println!("'newuser' validation: {:?}", result);
    assert!(result.is_ok());

    // Test with taken username
    let result = param.validate(&Value::text("admin")).await;
    println!("'admin' validation: {:?}", result);
    assert!(result.is_err());

    println!("Async validation example completed successfully!");
    Ok(())
}
```

**Step 2: Add example to Cargo.toml**

```toml
[[example]]
name = "async_validation"
path = "examples/async_validation.rs"
```

**Step 3: Run example**

Run: `cargo run -p nebula-parameter --example async_validation`
Expected: Runs successfully with output

**Step 4: Commit**

```bash
git add crates/nebula-parameter/examples/async_validation.rs crates/nebula-parameter/Cargo.toml
git commit -m "docs(nebula-parameter): add async validation example"
```

---

## Task 12: Final Cleanup and Documentation

**Files:**
- Modify: Various files for doc improvements
- Run: Full test suite and clippy

**Step 1: Run full test suite**

Run: `cargo test -p nebula-parameter -- --nocapture`
Expected: All tests PASS

**Step 2: Run clippy with strict settings**

Run: `cargo clippy -p nebula-parameter -- -D warnings -W clippy::pedantic`
Expected: PASS (fix any warnings)

**Step 3: Run cargo doc**

Run: `cargo doc -p nebula-parameter --no-deps --open`
Expected: Documentation builds without warnings, looks professional

**Step 4: Run cargo fmt**

Run: `cargo fmt -p nebula-parameter -- --check`
Expected: No formatting issues

**Step 5: Final commit**

```bash
git add -A
git commit -m "chore(nebula-parameter): final cleanup, documentation, and lint fixes"
```

---

## Summary of Breaking Changes

1. **ParameterCollection now stores `dyn FullParameter`** instead of `dyn Parameter`
   - All parameters must implement `Parameter + Validatable + Displayable`
   - This was already true in practice, now enforced at type level

2. **Parameter types use `ParameterBase`** instead of individual fields
   - API for accessing `metadata`, `display`, `validation` may change slightly
   - Builder patterns updated

3. **`validate_sync` may behave differently**
   - Now uses default implementation for common cases
   - Type-specific validation moved to extensions

4. **Removed duplicated validation logic**
   - Parameters no longer override `validate_sync` unless necessary

---

## Estimated Task Count: 12 major tasks

Each task is broken into 4-6 steps following TDD approach:
1. Write failing test
2. Run test to verify failure
3. Implement minimal code
4. Run test to verify pass
5. Commit

Total estimated steps: ~60 atomic actions
