# Refactor Plan: `crates/validator`

## Context

The `nebula-validator` crate is already well-architected with a clean macro system, solid trait design, and comprehensive tests. This refactoring focuses on polishing: fixing a compile bug, cleaning up redundant modules, standardizing imports, adding missing annotations, and improving documentation consistency.

## Changes

### 1. Fix `category.rs` compile error without `caching` feature

**File:** `src/foundation/category.rs`

The `Cached` import and its `Sealed`/`CompositeValidator` impls are not behind `#[cfg(feature = "caching")]`, causing a compile failure when the `caching` feature is disabled.

- Wrap the `Cached` import and both impl blocks in `#[cfg(feature = "caching")]`

### 2. Remove redundant `combinators::prelude` module

**File:** `src/combinators/mod.rs`

The `combinators::prelude` sub-module duplicates `src/prelude.rs`. Two preludes is confusing. The top-level prelude is the canonical one.

- Remove the `pub mod prelude { ... }` block from `combinators/mod.rs`

### 3. Remove redundant `foundation::prelude` module

**File:** `src/foundation/mod.rs`

Same issue - `foundation::prelude` duplicates parts of `src/prelude.rs`.

- Remove the `pub mod prelude { ... }` block from `foundation/mod.rs`

### 4. Remove stale TODO comment

**File:** `src/combinators/mod.rs`

```rust
// TODO: Re-enable when lru crate is added as dependency
// #[cfg(feature = "lru")]
// pub use cached::{LruCached, lru_cached};
```

- Delete these 3 lines

### 5. Add `#[must_use]` to combinator factory functions

**Files:** `src/combinators/and.rs`, `or.rs`, `not.rs`, `when.rs`, `unless.rs`, `each.rs`, `optional.rs`, `cached.rs`, `message.rs`, `field.rs`, `lazy.rs`, `json_field.rs`

Many factory functions (e.g., `and()`, `or()`, `not()`, `when()`) return validators but lack `#[must_use]`. Discarding a validator is almost certainly a bug.

- Add `#[must_use]` to all public free-standing factory functions that return validators

### 6. Standardize import ordering across all files

**All source files**

Apply consistent ordering following Rust convention:
1. `std` imports
2. External crate imports (`regex`, `moka`, `serde_json`)
3. `crate::` imports

Group with blank lines between sections. Remove unused imports.

### 7. Refactor `size.rs` to use `validator!` macro

**File:** `src/validators/size.rs`

The `size.rs` validators are manually implemented while the macro supports phantom generics (entry points 4 & 5). Refactoring to use the macro ensures consistency with `length.rs`, `pattern.rs`, `range.rs`, etc.

- Refactor `NotEmptyCollection<T>` to use `validator!` (phantom unit pattern)
- Keep `MinSize`, `MaxSize`, `ExactSize`, `SizeRange` as manual impls since they need public fields hidden behind `PhantomData` which the macro doesn't support cleanly for these cases

### 8. Improve doc comments on under-documented modules

Add module-level doc comments and doc comments on public items that lack them:

- `src/validators/range.rs` - Add module doc beyond the one-liner
- `src/validators/boolean.rs` - Add module doc beyond the one-liner
- `src/validators/nullable.rs` - Add module doc beyond the one-liner
- `src/combinators/and.rs` - Add doc comments on public methods
- `src/combinators/or.rs` - Add doc comments on public methods
- `src/combinators/not.rs` - Add doc comments on public methods
- `src/combinators/optional.rs` - Add doc comments on public methods
- `src/combinators/when.rs` - Add doc comments on public methods

### 9. Clean up `lib.rs` doc comment

**File:** `src/lib.rs`

- Update the built-in validators list to include all categories (currently missing `ExclusiveRange`, `GreaterThan`, `LessThan`, `LengthRange`, etc.)

## Files Modified

| File | Change |
|------|--------|
| `src/foundation/category.rs` | Feature-gate `Cached` imports/impls |
| `src/foundation/mod.rs` | Remove redundant prelude |
| `src/combinators/mod.rs` | Remove redundant prelude + stale TODO |
| `src/combinators/and.rs` | `#[must_use]` + doc comments |
| `src/combinators/or.rs` | `#[must_use]` + doc comments |
| `src/combinators/not.rs` | `#[must_use]` + doc comments |
| `src/combinators/when.rs` | `#[must_use]` + doc comments |
| `src/combinators/unless.rs` | `#[must_use]` |
| `src/combinators/each.rs` | `#[must_use]` |
| `src/combinators/optional.rs` | `#[must_use]` + doc comments |
| `src/combinators/cached.rs` | `#[must_use]` |
| `src/combinators/message.rs` | `#[must_use]` |
| `src/combinators/field.rs` | `#[must_use]` |
| `src/combinators/lazy.rs` | `#[must_use]` |
| `src/combinators/json_field.rs` | `#[must_use]` |
| `src/validators/size.rs` | Refactor `NotEmptyCollection` |
| `src/validators/range.rs` | Improve module docs |
| `src/validators/boolean.rs` | Improve module docs |
| `src/validators/nullable.rs` | Improve module docs |
| `src/lib.rs` | Update doc comment |
| All files | Standardize import ordering |

## Verification

```bash
# Must all pass:
cargo check -p nebula-validator --all-features
cargo check -p nebula-validator --no-default-features
cargo check -p nebula-validator --features caching
cargo test -p nebula-validator --all-features
cargo clippy -p nebula-validator --all-features -- -D warnings
cargo fmt -p nebula-validator -- --check
cargo doc -p nebula-validator --no-deps --all-features
```
