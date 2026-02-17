# Validator Crate Simplification Design

**Date:** 2026-02-16
**Branch:** 010-validator-foundation
**Goal:** Reduce code volume and improve DX by eliminating duplication, removing unused features, and introducing a unified macro DSL.

## Problem

The validator crate has ~2670 lines of duplicated or overly verbose code:
- ~2000 lines of metadata boilerplate across 30+ validators
- ~150 lines of repeated range/min/max validation logic
- ~120 lines of duplicated combinator metadata
- ~120 lines of numeric widening implementations
- ~80 lines of near-identical WithMessage/WithCode
- 20+ domain-specific validators that belong outside core

## Approach: Macro-Driven Reduction

Single unified `validator!` macro with clean DSL, metadata system removal, and domain validator deletion.

## Design Decisions

### 1. Remove Metadata System Entirely

**Delete:**
- `ValidatorMetadata`, `ValidatorMetadataBuilder`, `ValidationComplexity` from `foundation/metadata.rs`
- `RegisteredValidatorMetadata`, `ValidatorStatistics` (feature-gated under `optimizer`)
- `validator_metadata!` macro
- `metadata()` and `name()` methods from `Validate` trait
- All `fn metadata(&self)` implementations in 30+ validators and combinators
- `combinators/optimizer.rs`

**Simplified trait:**
```rust
pub trait Validate {
    type Input: ?Sized;
    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError>;
    fn validate_any<S>(&self, value: &S) -> Result<(), ValidationError>
    where
        Self: Sized,
        S: AsValidatable<Self::Input> + ?Sized,
        for<'a> <S as AsValidatable<Self::Input>>::Output<'a>: Borrow<Self::Input>,
    {
        let output = value.as_validatable()?;
        self.validate(output.borrow())
    }
}
```

### 2. Remove Domain-Specific Validators

**Keep (7 files, basic validators):**
- `length.rs` - MinLength, MaxLength, ExactLength, LengthRange, NotEmpty
- `range.rs` - Min, Max, InRange, GreaterThan, LessThan, ExclusiveRange
- `size.rs` - MinSize, MaxSize, ExactSize, NotEmptyCollection, SizeRange
- `pattern.rs` - Contains, StartsWith, EndsWith, Regex-based
- `content.rs` - Alphanumeric, Alphabetic, Numeric, etc.
- `boolean.rs` - IsTrue, IsFalse
- `nullable.rs` - NotNull, Required

**Delete (~20 files):**
- credit_card, iban, phone, password
- mac_address, slug, semver
- uuid, base64, hex
- datetime, time
- percentage, float, divisibility, properties
- hostname, ip_address, port
- elements, structure
- json_string

### 3. Unified `validator!` Macro DSL

**Design principles:**
- Single macro covers all validator patterns
- `Debug + Clone` always derived; additional derives via `#[derive(...)]`
- Supports: unit types, structs with fields, generics, custom constructors
- Auto-generates factory function

**Syntax:**

```rust
// Struct with fields + custom constructor
validator! {
    /// Validates minimum string length.
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub MinLength { min: usize, mode: LengthMode } for str;

    rule(self, input) {
        let len = self.mode.measure(input);
        len >= self.min
    }

    error(self, input) {
        ValidationError::min_length("", self.min, self.mode.measure(input))
    }

    new(min: usize) { Self { min, mode: LengthMode::Chars } }
    fn min_length(min: usize);
}

// Unit validator (ZST)
validator! {
    pub NotEmpty for str;
    rule(input) { !input.is_empty() }
    error(input) { ValidationError::new("not_empty", "String must not be empty") }
    fn not_empty();
}

// Generic validator
validator! {
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub Min<T: PartialOrd + Display + Copy> { min: T } for T;

    rule(self, input) { *input >= self.min }

    error(self, input) {
        ValidationError::new("min", format!("Value must be at least {}", self.min))
            .with_param("min", self.min.to_string())
            .with_param("actual", input.to_string())
    }

    fn min(value: T);
}
```

**Generated code per invocation:**
1. Struct with `#[derive(Debug, Clone, ...)]` and public fields
2. Constructor `impl Name { pub fn new(...) }` (auto all-fields or custom)
3. `impl Validate for Name { type Input = ...; fn validate(...) { if rule { Ok(()) } else { Err(error) } } }`
4. Factory function `pub fn snake_case(...) -> Name { Name::new(...) }`

**Type compatibility:**
- `chrono::NaiveDate` - works (Copy + Eq + Hash + PartialOrd + Display)
- `serde_json::Value` - works (omit Copy/Eq from derives)
- `f64` - works (use PartialEq only, no Hash)
- `str`, `[T]` - works as unsized input types via `for str`

### 4. Combinator Cleanup

- Remove all `fn metadata()` implementations from combinators
- Merge `WithMessage` + `WithCode` into `WithOverride`
- Delete `combinators/optimizer.rs`
- Keep: and, or, not, optional, when, unless, lazy, message, field, each, nested, cached (feature-gated), json_field (feature-gated)

### 5. Foundation Cleanup

- Delete `foundation/metadata.rs` entirely
- Simplify `Validate` trait (remove metadata/name)
- Macro-ize numeric widening impls in `validatable.rs` (~120 lines -> ~20 lines)
- Review `context.rs` and `category.rs` for metadata dependencies

### 6. Macro Consolidation

- Replace old macros (`validator!`, `validator_fn!`, `validator_const!`, `validator_metadata!`, `validate!`) with single unified `validator!`
- Keep `compose!` and `any_of!` (small, useful)

### 7. Test Strategy

- Delete tests with deleted validators
- Create shared `#[cfg(test)] mod test_utils` in `lib.rs`
- Remove metadata assertions from remaining tests
- Add comprehensive tests for new `validator!` macro (all variants)

## Final Structure

```
crates/validator/src/
├── lib.rs
├── macros.rs           # validator!, compose!, any_of!
├── prelude.rs
├── foundation/
│   ├── mod.rs
│   ├── traits.rs       # Validate, ValidateExt (no metadata)
│   ├── error.rs        # ValidationError, ValidationErrors
│   ├── context.rs      # ContextualValidator
│   ├── validatable.rs  # AsValidatable (macro-generated impls)
│   └── category.rs     # if needed
├── combinators/
│   ├── mod.rs
│   ├── and.rs, or.rs, not.rs
│   ├── optional.rs, when.rs, unless.rs, lazy.rs
│   ├── message.rs      # WithOverride
│   ├── field.rs, nested.rs, each.rs
│   ├── cached.rs       # feature-gated
│   ├── json_field.rs   # feature-gated
│   └── error.rs
└── validators/
    ├── mod.rs
    ├── length.rs, range.rs, size.rs
    ├── pattern.rs, content.rs
    ├── boolean.rs, nullable.rs
    └── (7 files total)
```

## Estimated Impact

- ~3500+ lines removed
- 20+ files deleted
- Adding a new validator: ~10 lines instead of ~50
- Cleaner trait surface
