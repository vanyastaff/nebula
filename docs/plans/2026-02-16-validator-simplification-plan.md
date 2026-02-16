# Validator Crate Simplification — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Cut ~3500 lines from `crates/validator/` by removing the metadata system, deleting domain validators, and introducing a unified `validator!` macro DSL.

**Architecture:** Phase 1 deletes unused code (safe mass removal). Phase 2 strips the metadata system from Validate trait, combinators, and remaining validators. Phase 3 creates the new `validator!` macro and rewrites validators with it. Phase 4 polishes (merge WithMessage/WithCode, macro-ize widening impls, clean deps, add docs).

**Tech Stack:** Rust 2024 Edition, macro_rules!, thiserror, regex, serde/serde_json

**Design Doc:** `docs/plans/2026-02-16-validator-simplification-design.md`

---

## Phase 1: Delete Domain Validators & Optimizer

### Task 1: Delete domain validator files

**Files:**
- Delete: `crates/validator/src/validators/credit_card.rs`
- Delete: `crates/validator/src/validators/iban.rs`
- Delete: `crates/validator/src/validators/phone.rs`
- Delete: `crates/validator/src/validators/password.rs`
- Delete: `crates/validator/src/validators/mac_address.rs`
- Delete: `crates/validator/src/validators/slug.rs`
- Delete: `crates/validator/src/validators/semver.rs`
- Delete: `crates/validator/src/validators/uuid.rs`
- Delete: `crates/validator/src/validators/base64.rs`
- Delete: `crates/validator/src/validators/hex.rs`
- Delete: `crates/validator/src/validators/datetime.rs`
- Delete: `crates/validator/src/validators/time.rs`
- Delete: `crates/validator/src/validators/percentage.rs`
- Delete: `crates/validator/src/validators/float.rs`
- Delete: `crates/validator/src/validators/divisibility.rs`
- Delete: `crates/validator/src/validators/properties.rs`
- Delete: `crates/validator/src/validators/hostname.rs`
- Delete: `crates/validator/src/validators/ip_address.rs`
- Delete: `crates/validator/src/validators/port.rs`
- Delete: `crates/validator/src/validators/elements.rs`
- Delete: `crates/validator/src/validators/structure.rs`
- Delete: `crates/validator/src/validators/json_string.rs`
- Delete: `crates/validator/src/combinators/optimizer.rs`

**Step 1: Delete all files listed above**

```bash
cd crates/validator/src
rm validators/credit_card.rs validators/iban.rs validators/phone.rs \
   validators/password.rs validators/mac_address.rs validators/slug.rs \
   validators/semver.rs validators/uuid.rs validators/base64.rs \
   validators/hex.rs validators/datetime.rs validators/time.rs \
   validators/percentage.rs validators/float.rs validators/divisibility.rs \
   validators/properties.rs validators/hostname.rs validators/ip_address.rs \
   validators/port.rs validators/elements.rs validators/structure.rs \
   validators/json_string.rs combinators/optimizer.rs
```

**Step 2: Rewrite `crates/validator/src/validators/mod.rs`**

Replace entire file with:

```rust
//! Built-in validators
//!
//! Basic validators for common validation scenarios:
//! string length, numeric range, collection size, patterns, content checks,
//! boolean logic, and nullable handling.

// Validator modules
pub mod boolean;
pub mod content;
pub mod length;
pub mod nullable;
pub mod pattern;
pub mod range;
pub mod size;

// ============================================================================
// RE-EXPORTS: String validators
// ============================================================================

pub use length::{
    ExactLength, LengthRange, MaxLength, MinLength, NotEmpty, exact_length, length_range,
    max_length, min_length, not_empty,
};

pub use pattern::{
    Alphabetic, Alphanumeric, Contains, EndsWith, Lowercase, Numeric, StartsWith, Uppercase,
    alphabetic, alphanumeric, contains, ends_with, lowercase, numeric, starts_with, uppercase,
};

pub use content::{Email, MatchesRegex, Url, email, matches_regex, url};

// ============================================================================
// RE-EXPORTS: Numeric validators
// ============================================================================

pub use range::{
    ExclusiveRange, GreaterThan, InRange, LessThan, Max, Min, exclusive_range, greater_than,
    in_range, less_than, max, min,
};

// ============================================================================
// RE-EXPORTS: Collection validators
// ============================================================================

pub use size::{
    ExactSize, MaxSize, MinSize, NotEmptyCollection, SizeRange, exact_size, max_size, min_size,
    not_empty_collection, size_range,
};

// ============================================================================
// RE-EXPORTS: Logical validators
// ============================================================================

pub use boolean::{IsFalse, IsTrue, is_false, is_true};
pub use nullable::{NotNull, Required, not_null, required};
```

**Step 3: Rewrite `crates/validator/src/combinators/mod.rs`**

Remove the `optimizer` module declaration and its re-exports. In the file:
- Remove line 87-88: `#[cfg(feature = "optimizer")] pub mod optimizer;`
- Remove lines 111-115: the `#[cfg(feature = "optimizer")] pub use optimizer::{...}` block
- Remove `optimizer` from the prelude sub-module if present

**Step 4: Rewrite `crates/validator/src/prelude.rs`**

Replace entire file with:

```rust
//! Prelude module for convenient imports.
//!
//! ```rust,ignore
//! use nebula_validator::prelude::*;
//!
//! let username = min_length(3).and(max_length(20)).and(alphanumeric());
//! let age = in_range(18, 100);
//! ```

// Foundation
pub use crate::foundation::{
    AsValidatable, ErrorSeverity, Validate, ValidateExt, ValidationComplexity, ValidationError,
    ValidationErrors, ValidatorMetadata,
};

// All validators
#[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
pub use crate::validators::*;

// Combinators
pub use crate::combinators::{
    And, Each, Field, FieldValidateExt, Lazy, Not, Optional, Or, Unless, When, WithCode,
    WithMessage, and, each, each_fail_fast, field, lazy, named_field, not, optional, or, unless,
    when, with_code, with_message,
};

#[cfg(feature = "serde")]
pub use crate::combinators::{JsonField, json_field, json_field_optional};

#[cfg(feature = "caching")]
pub use crate::combinators::{Cached, cached};
```

**Step 5: Update `crates/validator/Cargo.toml`**

Remove dependencies only used by deleted validators:
- Remove `base64` (was used by base64.rs)
- Remove `url` (was used by content.rs Email/Url — **check first if content.rs uses url crate**)
- Remove `uuid` (was used by uuid.rs)
- Remove `optimizer` feature

**IMPORTANT:** Before removing `url`, check `content.rs` — if `Email` or `Url` validators use the `url` crate, keep it. Same for `regex` — pattern.rs likely uses it, so keep `regex`.

**Step 6: Delete test & bench files that reference deleted validators**

- Delete: `tests/optimizer_test.rs`
- Modify: `tests/prelude_test.rs` — remove tests referencing deleted validators
- Modify: `tests/json_integration.rs` — remove references to deleted validators (but keep if it only tests JSON combinator)
- Modify: `benches/string_validators.rs` — remove benchmarks for deleted validators
- Delete: `examples/json_validation.rs` if it references deleted validators heavily

**Step 7: Run build and fix**

```bash
cargo check -p nebula-validator --all-features 2>&1 | head -50
```

Fix any remaining compilation errors from dangling references.

**Step 8: Run tests**

```bash
cargo test -p nebula-validator --all-features
```

**Step 9: Commit**

```bash
git add -A crates/validator/
git commit -m "refactor(validator): delete domain validators and optimizer

Remove 22 domain-specific validator files (credit_card, iban, phone,
password, mac_address, slug, semver, uuid, base64, hex, datetime,
time, percentage, float, divisibility, properties, hostname,
ip_address, port, elements, structure, json_string) and the
optimizer module. Keep only basic validators: length, range, size,
pattern, content, boolean, nullable."
```

---

## Phase 2: Remove Metadata System

### Task 2: Strip metadata from Validate trait and delete metadata.rs

**Files:**
- Modify: `crates/validator/src/foundation/traits.rs:1-8, 116-129`
- Delete: `crates/validator/src/foundation/metadata.rs` (entire 529-line file)
- Modify: `crates/validator/src/foundation/mod.rs:76, 83-85, 104-108`

**Step 1: Simplify the Validate trait**

In `crates/validator/src/foundation/traits.rs`:

Remove line 5: `use crate::foundation::ValidatorMetadata;`

Remove lines 116-129 (the `metadata()` and `name()` methods from Validate trait):
```rust
    // DELETE these lines:
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::default()
    }

    fn name(&self) -> &str {
        std::any::type_name::<Self>()
    }
```

The trait should now only contain `type Input`, `validate()`, and `validate_any()`.

**Step 2: Delete metadata.rs**

```bash
rm crates/validator/src/foundation/metadata.rs
```

**Step 3: Update foundation/mod.rs**

- Remove line 76: `pub mod metadata;`
- Remove lines 83-85: the `pub use metadata::...` re-exports
- Remove `ValidatorMetadata`, `ValidationComplexity`, `ValidatorMetadataBuilder`, `RegisteredValidatorMetadata`, `ValidatorStatistics` from foundation prelude (lines 104-108)

**Step 4: Remove metadata from all remaining validators**

In each of these files, delete the `fn metadata(&self) -> ValidatorMetadata { ... }` block and any `use crate::foundation::{..., ValidatorMetadata, ValidationComplexity, ...}` imports:

- `crates/validator/src/validators/length.rs` — 5 metadata blocks (lines 90-101, 174-185, 255-266, 354-371, 413-424) + remove `ValidatorMetadata` and `ValidationComplexity` from use statement on line 8
- `crates/validator/src/validators/range.rs` — 6 metadata blocks (lines 41-52, 94-105, 147-160, 217-228, 290-301, 369-380) + remove from use statement line 3
- `crates/validator/src/validators/size.rs` — 5 `validator_metadata!()` calls (lines 37-42, 94-99, 151-156, 204-209, 264-269)
- `crates/validator/src/validators/pattern.rs` — 3 manual metadata blocks + 5 `validator_metadata!()` calls
- `crates/validator/src/validators/content.rs` — 3 metadata blocks (lines 55, 115, 171)
- `crates/validator/src/validators/boolean.rs` — 2 `validator_metadata!()` calls (lines 24, 56)
- `crates/validator/src/validators/nullable.rs` — 1 `validator_metadata!()` call (line 27)

**Step 5: Remove metadata from all combinators**

In each combinator file, delete the `fn metadata(&self) -> ValidatorMetadata { ... }` block and remove `ValidatorMetadata` from imports:

- `crates/validator/src/combinators/and.rs` — 2 blocks (lines 44, 117)
- `crates/validator/src/combinators/or.rs` — 2 blocks (lines 54, 140)
- `crates/validator/src/combinators/not.rs` — 1 block (line 42)
- `crates/validator/src/combinators/when.rs` — 1 block (line 49)
- `crates/validator/src/combinators/unless.rs` — 1 block (line 82)
- `crates/validator/src/combinators/optional.rs` — 1 block (line 39)
- `crates/validator/src/combinators/lazy.rs` — 1 block (line 86)
- `crates/validator/src/combinators/message.rs` — 2 blocks (lines 86, 170)
- `crates/validator/src/combinators/each.rs` — 1 block (line 128)
- `crates/validator/src/combinators/field.rs` — 2 blocks (lines 216, 379)
- `crates/validator/src/combinators/nested.rs` — 3 blocks (lines 39, 116, 176)
- `crates/validator/src/combinators/cached.rs` — 1 block (line 155)

**Step 6: Clean up category.rs**

In `crates/validator/src/foundation/category.rs`:
- The category traits (`StringValidator`, `NumericValidator`, etc.) have `description()` methods — these don't depend on metadata but evaluate if the whole category system is still useful. If `description()` is the only metadata-like thing, consider keeping category.rs as-is (it's only 282 lines and provides type-safety for combinator composition).
- Remove any import of `ValidatorMetadata` if present.

**Step 7: Update prelude.rs**

Remove `ValidationComplexity` and `ValidatorMetadata` from the foundation re-exports in prelude.

**Step 8: Remove `optimizer` feature from Cargo.toml**

In `crates/validator/Cargo.toml`:
- Remove line: `optimizer = []`
- Update `full` feature to: `full = ["serde", "caching"]`

**Step 9: Fix remaining tests**

- Remove metadata assertions from `crates/validator/src/validators/length.rs` tests (lines 552-560)
- Remove metadata assertions from `crates/validator/src/foundation/traits.rs` tests (line 341-343 `test_validator_name`)
- Remove metadata tests from `crates/validator/src/foundation/metadata.rs` tests (deleted with file)
- Remove `test_validator_metadata` from `crates/validator/src/macros.rs` tests
- Update any test files in `tests/` that reference metadata

**Step 10: Build and test**

```bash
cargo check -p nebula-validator --all-features 2>&1 | head -80
cargo test -p nebula-validator --all-features
```

**Step 11: Commit**

```bash
git add -A crates/validator/
git commit -m "refactor(validator): remove metadata system entirely

Delete foundation/metadata.rs (ValidatorMetadata, ValidationComplexity,
ValidatorMetadataBuilder, ValidatorStatistics). Remove metadata() and
name() from Validate trait. Strip all metadata implementations from
validators and combinators. Remove optimizer feature."
```

---

## Phase 3: New validator! Macro & Rewrite

### Task 3: Create the unified validator! macro

**Files:**
- Rewrite: `crates/validator/src/macros.rs`

**Step 1: Write macro tests first**

At the bottom of `crates/validator/src/macros.rs`, write the test module that exercises all macro variants:

```rust
#[cfg(test)]
mod tests {
    use crate::foundation::{Validate, ValidationError};

    // Test 1: Unit validator (no fields)
    validator! {
        /// A test unit validator.
        TestNotEmpty for str;
        rule(input) { !input.is_empty() }
        error(input) { ValidationError::new("not_empty", "must not be empty") }
        fn test_not_empty();
    }

    #[test]
    fn test_unit_validator() {
        let v = TestNotEmpty;
        assert!(v.validate("hello").is_ok());
        assert!(v.validate("").is_err());
    }

    #[test]
    fn test_unit_factory() {
        let v = test_not_empty();
        assert!(v.validate("x").is_ok());
    }

    // Test 2: Struct with fields + auto new
    validator! {
        #[derive(Copy, PartialEq, Eq, Hash)]
        TestMinLen { min: usize } for str;
        rule(self, input) { input.len() >= self.min }
        error(self, input) {
            ValidationError::new("min_len", format!("need {} chars", self.min))
        }
        fn test_min_len(min: usize);
    }

    #[test]
    fn test_struct_validator() {
        let v = TestMinLen { min: 3 };
        assert!(v.validate("abc").is_ok());
        assert!(v.validate("ab").is_err());
    }

    #[test]
    fn test_struct_factory() {
        let v = test_min_len(5);
        assert!(v.validate("hello").is_ok());
        assert!(v.validate("hi").is_err());
    }

    // Test 3: Generic validator
    use std::fmt::Display;

    validator! {
        #[derive(Copy, PartialEq, Eq, Hash)]
        TestMin<T: PartialOrd + Display + Copy> { min: T } for T;
        rule(self, input) { *input >= self.min }
        error(self, input) {
            ValidationError::new("min", format!("must be >= {}", self.min))
        }
        fn test_min(value: T);
    }

    #[test]
    fn test_generic_validator() {
        let v = test_min(5_i32);
        assert!(v.validate(&5).is_ok());
        assert!(v.validate(&4).is_err());
    }

    // Test 4: Custom constructor
    validator! {
        #[derive(Copy, PartialEq, Eq, Hash)]
        TestRange { lo: usize, hi: usize } for usize;
        rule(self, input) { *input >= self.lo && *input <= self.hi }
        error(self, input) {
            ValidationError::new("range", format!("{} not in {}..{}", input, self.lo, self.hi))
        }
        new(lo: usize, hi: usize) { Self { lo, hi } }
        fn test_range(lo: usize, hi: usize);
    }

    #[test]
    fn test_custom_new() {
        let v = test_range(1, 10);
        assert!(v.validate(&5).is_ok());
        assert!(v.validate(&0).is_err());
    }

    // Test 5: compose! and any_of! still work
    #[test]
    fn test_compose_still_works() {
        let v = compose![TestMinLen { min: 3 }, TestMinLen { min: 1 }];
        assert!(v.validate("abc").is_ok());
    }

    #[test]
    fn test_any_of_still_works() {
        let v = any_of![TestMinLen { min: 100 }, TestMinLen { min: 1 }];
        assert!(v.validate("x").is_ok());
    }
}
```

**Step 2: Run tests to see them fail**

```bash
cargo test -p nebula-validator --lib macros::tests 2>&1 | head -30
```

Expected: compilation errors because the new `validator!` syntax doesn't exist yet.

**Step 3: Implement the validator! macro**

Replace the contents of `crates/validator/src/macros.rs` (keeping `compose!` and `any_of!`) with:

```rust
//! Macros for creating validators with minimal boilerplate.
//!
//! # Available Macros
//!
//! - [`validator!`] — Create a complete validator (struct + Validate impl + factory fn)
//! - [`compose!`] — AND-chain multiple validators
//! - [`any_of!`] — OR-chain multiple validators
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::validator;
//! use nebula_validator::foundation::{Validate, ValidationError};
//!
//! // Unit validator (no fields)
//! validator! {
//!     pub NotEmpty for str;
//!     rule(input) { !input.is_empty() }
//!     error(input) { ValidationError::new("not_empty", "must not be empty") }
//!     fn not_empty();
//! }
//!
//! // Struct with fields
//! validator! {
//!     #[derive(Copy, PartialEq, Eq, Hash)]
//!     pub MinLength { min: usize } for str;
//!     rule(self, input) { input.len() >= self.min }
//!     error(self, input) { ValidationError::min_length("", self.min, input.len()) }
//!     fn min_length(min: usize);
//! }
//! ```

// ============================================================================
// VALIDATOR MACRO
// ============================================================================

/// Creates a complete validator: struct definition, `Validate` implementation,
/// constructor, and factory function.
///
/// `#[derive(Debug, Clone)]` is always applied. Add extra derives via `#[derive(...)]`.
///
/// # Variants
///
/// **Unit validator** (zero-sized, no fields):
/// ```rust,ignore
/// validator! {
///     pub NotEmpty for str;
///     rule(input) { !input.is_empty() }
///     error(input) { ValidationError::new("not_empty", "empty") }
///     fn not_empty();
/// }
/// ```
///
/// **Struct with fields** (auto `new` from all fields):
/// ```rust,ignore
/// validator! {
///     #[derive(Copy, PartialEq, Eq, Hash)]
///     pub MinLength { min: usize } for str;
///     rule(self, input) { input.len() >= self.min }
///     error(self, input) { ValidationError::min_length("", self.min, input.len()) }
///     fn min_length(min: usize);
/// }
/// ```
///
/// **Custom constructor** (overrides auto `new`):
/// ```rust,ignore
/// validator! {
///     pub LengthRange { min: usize, max: usize } for str;
///     rule(self, input) { let l = input.len(); l >= self.min && l <= self.max }
///     error(self, input) { ValidationError::new("range", "out of range") }
///     new(min: usize, max: usize) { Self { min, max } }
///     fn length_range(min: usize, max: usize);
/// }
/// ```
///
/// **Generic validator**:
/// ```rust,ignore
/// validator! {
///     #[derive(Copy, PartialEq, Eq, Hash)]
///     pub Min<T: PartialOrd + Display + Copy> { min: T } for T;
///     rule(self, input) { *input >= self.min }
///     error(self, input) { ValidationError::new("min", format!("must be >= {}", self.min)) }
///     fn min(value: T);
/// }
/// ```
#[macro_export]
macro_rules! validator {
    // ── Variant 1: Unit validator (no fields) ────────────────────────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident for $input:ty;
        rule($inp:ident) $rule:block
        error($einp:ident) $err:block
        $(fn $factory:ident();)?
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        $vis struct $name;

        impl $crate::foundation::Validate for $name {
            type Input = $input;

            fn validate(&self, $inp: &Self::Input) -> Result<(), $crate::foundation::ValidationError> {
                if $rule {
                    Ok(())
                } else {
                    Err($err)
                }
            }
        }

        $(
            #[must_use]
            $vis const fn $factory() -> $name { $name }
        )?
    };

    // ── Variant 2: Struct with fields + auto new ─────────────────────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident { $($field:ident: $fty:ty),+ $(,)? } for $input:ty;
        rule($self1:ident, $inp:ident) $rule:block
        error($self2:ident, $einp:ident) $err:block
        $(fn $factory:ident($($farg:ident: $faty:ty),* $(,)?);)?
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone)]
        $vis struct $name {
            $(pub $field: $fty,)+
        }

        impl $name {
            #[must_use]
            pub fn new($($field: $fty),+) -> Self {
                Self { $($field),+ }
            }
        }

        impl $crate::foundation::Validate for $name {
            type Input = $input;

            fn validate(&$self1, $inp: &Self::Input) -> Result<(), $crate::foundation::ValidationError> {
                if $rule {
                    Ok(())
                } else {
                    Err($err)
                }
            }
        }

        $(
            #[must_use]
            $vis fn $factory($($farg: $faty),*) -> $name {
                $name::new($($farg),*)
            }
        )?
    };

    // ── Variant 3: Struct with fields + custom new ───────────────────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident { $($field:ident: $fty:ty),+ $(,)? } for $input:ty;
        rule($self1:ident, $inp:ident) $rule:block
        error($self2:ident, $einp:ident) $err:block
        new($($narg:ident: $naty:ty),* $(,)?) $new_body:block
        $(fn $factory:ident($($farg:ident: $faty:ty),* $(,)?);)?
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone)]
        $vis struct $name {
            $(pub $field: $fty,)+
        }

        impl $name {
            #[must_use]
            pub fn new($($narg: $naty),*) -> Self $new_body
        }

        impl $crate::foundation::Validate for $name {
            type Input = $input;

            fn validate(&$self1, $inp: &Self::Input) -> Result<(), $crate::foundation::ValidationError> {
                if $rule {
                    Ok(())
                } else {
                    Err($err)
                }
            }
        }

        $(
            #[must_use]
            $vis fn $factory($($farg: $faty),*) -> $name {
                $name::new($($farg),*)
            }
        )?
    };

    // ── Variant 4: Generic struct + auto new ─────────────────────────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident<$($gen:ident: $bound:tt $(+ $bounds:tt)*),+>
            { $($field:ident: $fty:ty),+ $(,)? } for $input:ty;
        rule($self1:ident, $inp:ident) $rule:block
        error($self2:ident, $einp:ident) $err:block
        $(fn $factory:ident($($farg:ident: $faty:ty),* $(,)?);)?
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone)]
        $vis struct $name<$($gen),+> {
            $(pub $field: $fty,)+
        }

        impl<$($gen: $bound $(+ $bounds)*),+> $name<$($gen),+> {
            #[must_use]
            pub fn new($($field: $fty),+) -> Self {
                Self { $($field),+ }
            }
        }

        impl<$($gen: $bound $(+ $bounds)*),+> $crate::foundation::Validate for $name<$($gen),+> {
            type Input = $input;

            fn validate(&$self1, $inp: &Self::Input) -> Result<(), $crate::foundation::ValidationError> {
                if $rule {
                    Ok(())
                } else {
                    Err($err)
                }
            }
        }

        $(
            #[must_use]
            $vis fn $factory<$($gen: $bound $(+ $bounds)*),+>($($farg: $faty),*) -> $name<$($gen),+> {
                $name::new($($farg),*)
            }
        )?
    };
}

// ============================================================================
// COMPOSE MACRO
// ============================================================================

/// Composes multiple validators using AND logic.
///
/// ```rust,ignore
/// let validator = compose![min_length(5), max_length(20), alphanumeric()];
/// ```
#[macro_export]
macro_rules! compose {
    ($first:expr) => { $first };
    ($first:expr, $($rest:expr),+ $(,)?) => {
        $first$(.and($rest))+
    };
}

/// Composes multiple validators using OR logic.
///
/// ```rust,ignore
/// let validator = any_of![exact_length(5), exact_length(10)];
/// ```
#[macro_export]
macro_rules! any_of {
    ($first:expr) => { $first };
    ($first:expr, $($rest:expr),+ $(,)?) => {
        $first$(.or($rest))+
    };
}
```

**Step 4: Run tests**

```bash
cargo test -p nebula-validator --lib macros::tests
```

Expected: all tests pass.

**Step 5: Commit**

```bash
git add crates/validator/src/macros.rs
git commit -m "refactor(validator): replace old macros with unified validator! DSL

New validator! macro supports: unit validators, structs with fields,
generic types, custom constructors, and factory functions. Replaces
validator!, validator_fn!, validator_const!, validator_metadata!, and
validate! macros. Keeps compose! and any_of!."
```

---

### Task 4: Rewrite remaining validators with the new macro

**Files:**
- Rewrite: `crates/validator/src/validators/length.rs`
- Rewrite: `crates/validator/src/validators/range.rs`
- Rewrite: `crates/validator/src/validators/size.rs`
- Rewrite: `crates/validator/src/validators/boolean.rs`
- Rewrite: `crates/validator/src/validators/nullable.rs`
- Modify: `crates/validator/src/validators/pattern.rs` (remove metadata, consider partial macro use)
- Modify: `crates/validator/src/validators/content.rs` (remove metadata, keep regex-based validators manual)

**Step 1: Rewrite boolean.rs (~90 lines -> ~20 lines)**

```rust
//! Boolean validators

use crate::foundation::ValidationError;

crate::validator! {
    /// Validates that a boolean value is `true`.
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub IsTrue for bool;
    rule(input) { *input }
    error(input) { ValidationError::new("is_true", "Value must be true") }
    fn is_true();
}

crate::validator! {
    /// Validates that a boolean value is `false`.
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub IsFalse for bool;
    rule(input) { !*input }
    error(input) { ValidationError::new("is_false", "Value must be false") }
    fn is_false();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Validate;

    #[test]
    fn test_is_true() {
        assert!(is_true().validate(&true).is_ok());
        assert!(is_true().validate(&false).is_err());
    }

    #[test]
    fn test_is_false() {
        assert!(is_false().validate(&false).is_ok());
        assert!(is_false().validate(&true).is_err());
    }
}
```

**Step 2: Rewrite nullable.rs (~78 lines -> ~25 lines)**

```rust
//! Nullable validators for Option types

use crate::foundation::{Validate, ValidationError};

/// Validates that an `Option` is `Some`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Required;

impl<T> Validate for Required {
    type Input = Option<T>;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if input.is_some() {
            Ok(())
        } else {
            Err(ValidationError::new("required", "Value is required"))
        }
    }
}

/// Creates a Required validator.
#[must_use]
pub const fn required() -> Required {
    Required
}

/// Alias for Required.
pub type NotNull = Required;

/// Creates a NotNull validator.
#[must_use]
pub const fn not_null() -> NotNull {
    Required
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_required() {
        assert!(required().validate(&Some(42)).is_ok());
        assert!(required().validate(&None::<i32>).is_err());
    }

    #[test]
    fn test_not_null() {
        assert!(not_null().validate(&Some("x")).is_ok());
        assert!(not_null().validate(&None::<&str>).is_err());
    }
}
```

Note: `nullable.rs` can't use the macro directly because `Required` is generic over `Option<T>` which is more complex than the macro patterns. Keep it manual — it's already small.

**Step 3: Rewrite range.rs**

For each of the 6 validators (Min, Max, InRange, GreaterThan, LessThan, ExclusiveRange), use the generic macro variant. Remove all metadata blocks. The file should shrink from ~471 lines to ~150 lines.

Example for Min:
```rust
use std::fmt::Display;

crate::validator! {
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

Do the same for Max, InRange, GreaterThan, LessThan, ExclusiveRange.

**Step 4: Rewrite length.rs**

Length validators have `LengthMode` which requires custom constructors. Use Variant 3 (custom new) for MinLength, MaxLength, ExactLength. LengthRange returns `Result` from its constructor, so keep it manual. NotEmpty uses unit variant.

**Step 5: Rewrite size.rs**

Size validators use `PhantomData<T>`, which complicates macro use. Keep them manual but remove all metadata blocks. The code is already clean after removing `validator_metadata!()` calls.

**Step 6: Clean up pattern.rs and content.rs**

These use regex and have complex internal logic. Don't rewrite with macro — just remove all `fn metadata()` blocks and `validator_metadata!()` calls. This is already done in Task 2 but verify.

**Step 7: Build and test**

```bash
cargo check -p nebula-validator --all-features
cargo test -p nebula-validator --all-features
```

**Step 8: Commit**

```bash
git add -A crates/validator/src/validators/
git commit -m "refactor(validator): rewrite validators with unified macro DSL

Rewrite boolean.rs, range.rs, and parts of length.rs using the new
validator! macro. Remove all remaining metadata blocks from size.rs,
pattern.rs, and content.rs."
```

---

## Phase 4: Polish

### Task 5: Merge WithMessage and WithCode

**Files:**
- Rewrite: `crates/validator/src/combinators/message.rs`
- Modify: `crates/validator/src/combinators/mod.rs` (update re-exports)
- Modify: `crates/validator/src/prelude.rs` (update re-exports)

**Step 1: Rewrite message.rs**

`WithMessage` already has an optional `code` field. `WithCode` is just a subset. Merge them by keeping `WithMessage` as the primary type (it already supports both message and code override). Alias `WithCode = WithMessage` or just remove `WithCode` and update its usages.

Simplest approach: keep both types but reduce `WithCode` to delegate to `WithMessage`:

```rust
// Keep WithMessage as-is (already has .with_code() builder method)
// Simplify WithCode to just a thin wrapper

pub fn with_code<V>(validator: V, code: impl Into<String>) -> WithMessage<V> {
    WithMessage {
        inner: validator,
        message: String::new(), // empty = keep original message
        code: Some(code.into()),
    }
}
```

Actually, the cleanest approach: keep `WithMessage` struct, make `WithCode` a type alias or remove it entirely. Update all re-exports.

**Step 2: Update mod.rs and prelude.rs re-exports**

If `WithCode` is removed, update:
- `combinators/mod.rs`: remove `WithCode` from `pub use message::{...}`
- `prelude.rs`: remove `WithCode` from combinators re-export
- `foundation/traits.rs` `ValidateExt`: if it has `.with_code()` method, update to return `WithMessage`

**Step 3: Test and commit**

```bash
cargo test -p nebula-validator --all-features
git add -A crates/validator/src/combinators/message.rs crates/validator/src/combinators/mod.rs crates/validator/src/prelude.rs
git commit -m "refactor(validator): simplify message combinators

Remove WithCode struct, use WithMessage for both message and code
overrides. Update re-exports."
```

---

### Task 6: Macro-ize numeric widening impls in validatable.rs

**Files:**
- Modify: `crates/validator/src/foundation/validatable.rs`

**Step 1: Find the repetitive impl blocks**

Look for patterns like:
```rust
impl AsValidatable<i64> for i32 { ... }
impl AsValidatable<i64> for i16 { ... }
```

**Step 2: Create a local macro and replace**

```rust
macro_rules! impl_numeric_widening {
    ($from:ty => $to:ty) => {
        impl AsValidatable<$to> for $from {
            type Output<'a> = $to;
            fn as_validatable(&self) -> Result<Self::Output<'_>, ValidationError> {
                Ok((*self).into())
            }
        }
    };
    ($($from:ty => $to:ty),+ $(,)?) => {
        $(impl_numeric_widening!($from => $to);)+
    };
}

impl_numeric_widening! {
    i8 => i64, i16 => i64, i32 => i64,
    u8 => i64, u16 => i64, u32 => i64,
    f32 => f64,
    i8 => f64, i16 => f64, i32 => f64, i64 => f64,
    u8 => f64, u16 => f64, u32 => f64,
}
```

**Step 3: Test and commit**

```bash
cargo test -p nebula-validator --all-features
git add crates/validator/src/foundation/validatable.rs
git commit -m "refactor(validator): macro-ize numeric widening impls

Replace ~120 lines of repetitive AsValidatable implementations with
a macro, reducing to ~20 lines."
```

---

### Task 7: Clean up Cargo.toml dependencies and features

**Files:**
- Modify: `crates/validator/Cargo.toml`

**Step 1: Check which deps are still used**

```bash
# Check if base64 crate is referenced anywhere in remaining code
grep -r "base64" crates/validator/src/ --include="*.rs"
grep -r "uuid" crates/validator/src/ --include="*.rs"
grep -r "url" crates/validator/src/ --include="*.rs"
```

**Step 2: Remove unused dependencies**

Based on step 1, remove deps no longer referenced:
- `base64` — only used by deleted base64.rs
- `uuid` — only used by deleted uuid.rs
- `url` — check if content.rs Url validator uses it

**Step 3: Build and commit**

```bash
cargo check -p nebula-validator --all-features
git add crates/validator/Cargo.toml
git commit -m "chore(validator): remove unused dependencies

Remove base64, uuid, and url crates no longer needed after deleting
domain-specific validators."
```

---

### Task 8: Update tests, examples, and benchmarks

**Files:**
- Modify/Delete: `tests/prelude_test.rs`
- Modify/Delete: `tests/integration_test.rs`
- Modify/Delete: `tests/json_integration.rs`
- Modify/Delete: `tests/combinator_error_test.rs`
- Modify/Delete: `tests/validation_context_test.rs`
- Modify: `benches/string_validators.rs`
- Modify: `benches/combinators.rs`
- Modify/Delete: `examples/validator_basic_usage.rs`
- Modify/Delete: `examples/combinators.rs`

**Step 1: Fix integration tests**

Read each test file. Remove any references to deleted validators (email, uuid, credit_card, etc.). Update imports. Delete test files that are now empty or irrelevant.

**Step 2: Fix benchmarks**

Remove benchmarks for deleted validators from `benches/string_validators.rs`. Keep benchmarks for length, pattern, content validators.

**Step 3: Fix examples**

Update examples to use only remaining validators. If an example is mainly about deleted functionality, delete it.

**Step 4: Run full test suite**

```bash
cargo test -p nebula-validator --all-features
cargo test -p nebula-validator  # default features
cargo clippy -p nebula-validator --all-features -- -D warnings
cargo doc --no-deps -p nebula-validator
```

**Step 5: Commit**

```bash
git add -A crates/validator/tests/ crates/validator/benches/ crates/validator/examples/
git commit -m "test(validator): update tests, benchmarks, and examples

Remove references to deleted validators. Update integration tests
and benchmarks for the simplified API."
```

---

### Task 9: Add rustdoc documentation

**Files:**
- Modify: `crates/validator/src/lib.rs` — crate-level docs
- Modify: `crates/validator/src/macros.rs` — macro docs with examples
- Modify: `crates/validator/src/prelude.rs` — prelude docs
- Modify: `crates/validator/src/foundation/mod.rs` — foundation docs
- Modify: `crates/validator/src/foundation/traits.rs` — Validate trait docs
- Modify: `crates/validator/src/validators/mod.rs` — validators overview

**Step 1: Add crate-level documentation to lib.rs**

```rust
//! # nebula-validator
//!
//! A composable, type-safe validation framework for the Nebula workflow engine.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use nebula_validator::prelude::*;
//!
//! // Compose validators with .and() / .or() / .not()
//! let username = min_length(3).and(max_length(20)).and(alphanumeric());
//! assert!(username.validate("alice").is_ok());
//!
//! // Create custom validators with the validator! macro
//! validator! {
//!     pub Positive for i64;
//!     rule(input) { *input > 0 }
//!     error(input) { ValidationError::new("positive", "must be positive") }
//!     fn positive();
//! }
//! ```
//!
//! ## Creating Validators
//!
//! Use the [`validator!`] macro for zero-boilerplate validators,
//! or implement [`Validate`](foundation::Validate) manually for complex cases.
```

**Step 2: Verify docs build**

```bash
cargo doc --no-deps -p nebula-validator 2>&1 | head -20
```

**Step 3: Commit**

```bash
git add -A crates/validator/src/
git commit -m "docs(validator): add comprehensive rustdoc documentation

Add crate-level docs, macro usage examples, and updated module docs
for the simplified validator API."
```

---

### Task 10: Final verification

**Step 1: Full CI pipeline**

```bash
cargo fmt --all -- --check
cargo clippy -p nebula-validator --all-features -- -D warnings
cargo check -p nebula-validator --all-targets
cargo test -p nebula-validator --all-features
cargo doc --no-deps -p nebula-validator
```

**Step 2: Verify workspace still builds**

```bash
cargo check --workspace --all-features
cargo test --workspace
```

**Step 3: Commit any final fixes**

```bash
git add -A
git commit -m "chore(validator): final cleanup after simplification"
```
