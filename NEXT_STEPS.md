# Next Steps for nebula-validator

## Current Status: 63 Compilation Errors Remaining

### What Was Done This Session

✅ **nebula-derive** - Complete and working
✅ **nebula-validator/src/validators/numeric/properties.rs** - Fixed
✅ **nebula-validator/src/validators/collection/size.rs** - Fixed
⚠️ **63 errors remain** - systematic patterns identified

---

## Remaining Errors Breakdown

### 1. Type Parameter Constraints (E0207) - ~30 errors

**Problem**: Validators declared without generics, but impl uses generic parameters

**Example (WRONG):**
```rust
struct Unique;  // ❌ No generic

impl<T> TypedValidator for Unique  // ❌ T is unconstrained
where T: Hash + Eq
{
    type Input = [T];
    // ...
}
```

**Fix (CORRECT):**
```rust
struct Unique<T> {  // ✅ Generic in struct
    _phantom: PhantomData<T>,
}

impl<T> TypedValidator for Unique<T>  // ✅ T is constrained
where T: Hash + Eq
{
    type Input = Vec<T>;  // or [T]
    // ...
}

pub fn unique<T>() -> Unique<T> {
    Unique { _phantom: PhantomData }
}
```

**Files to Fix:**
- `src/validators/collection/elements.rs` - Unique, All, Any, ContainsElement
- `src/validators/collection/structure.rs` - HasKey
- `src/validators/logical/nullable.rs` - Required

---

### 2. Trait Bound Issues (E0277) - ~19 errors

**Problem**: Cached validator and some combinators have trait bound issues

**Location**: `src/combinators/cached.rs`

**Fix Strategy:**
- Review trait bounds on Cached implementation
- Ensure V: TypedValidator is properly constrained
- May need to add where clauses on struct definition

---

### 3. Bridge/Value Integration (E0599) - ~7 errors

**Problem**: nebula-value API mismatch

**Location**: `src/bridge/value.rs`

**Examples:**
- `no method named type_name`
- `no variant named Number`

**Fix Strategy:**
- Check nebula-value current API
- Update bridge module to match
- May need to refactor Value enum access

---

### 4. Miscellaneous (E0308, E0282, E0220) - ~7 errors

**Types:**
- Type mismatches in combinators
- Type annotations needed
- Associated type not found

**Fix Strategy:**
- Review each error individually
- Most are likely consequences of fixing the above issues

---

## Systematic Fix Plan

### Phase 1: Collection Validators (2-3 hours)

1. **elements.rs** - Fix all validators:
   ```rust
   // Pattern to follow:
   pub struct Unique<T> { _phantom: PhantomData<T> }
   impl<T> TypedValidator for Unique<T> where T: Hash + Eq { ... }
   pub fn unique<T>() -> Unique<T> { ... }
   ```

2. **structure.rs** - Fix HasKey:
   ```rust
   pub struct HasKey<K, V> {
       key: K,
       _phantom: PhantomData<V>,
   }
   ```

### Phase 2: Logical Validators (30 min)

1. **nullable.rs** - Fix Required:
   ```rust
   pub struct Required<T> { _phantom: PhantomData<T> }
   impl<T> TypedValidator for Required<T> { ... }
   ```

### Phase 3: Combinators (1-2 hours)

1. **Review all combinator trait bounds**
2. **Fix Cached implementation**
3. **Test And, Or, Not, Map, When**

### Phase 4: Bridge Module (1 hour)

1. **Check nebula-value API**
2. **Update bridge adapters**
3. **Test Value integration**

### Phase 5: Final Testing (2 hours)

1. **Run all tests**
2. **Fix any remaining issues**
3. **Add missing tests**

**Total Estimated Time: 6-8 hours**

---

## Quick Reference: Generic Pattern

For any validator that works with typed data:

```rust
// 1. Struct with PhantomData
pub struct MyValidator<T> {
    config: SomeConfig,
    _phantom: PhantomData<T>,
}

// 2. Impl with same generic
impl<T> TypedValidator for MyValidator<T>
where
    T: YourTraitBounds,
{
    type Input = T;  // or [T], Vec<T>, etc.
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        // implementation
    }
}

// 3. Constructor function
pub fn my_validator<T>(config: SomeConfig) -> MyValidator<T>
where
    T: YourTraitBounds,
{
    MyValidator {
        config,
        _phantom: PhantomData,
    }
}
```

---

## Running Tests

```bash
# Check compilation
cargo check -p nebula-validator

# Run tests (once compiling)
cargo test -p nebula-validator

# Run specific test
cargo test -p nebula-validator --test validator_tests

# Check with all features
cargo check -p nebula-validator --all-features
```

---

## Priority Files to Fix (in order)

1. ✅ `src/validators/numeric/properties.rs` - DONE
2. ✅ `src/validators/collection/size.rs` - DONE
3. ⏭️ `src/validators/collection/elements.rs` - NEXT
4. ⏭️ `src/validators/collection/structure.rs`
5. ⏭️ `src/validators/logical/nullable.rs`
6. ⏭️ `src/combinators/cached.rs`
7. ⏭️ `src/bridge/value.rs`

---

## Expected Outcome

After completing these fixes:
- ✅ 0 compilation errors
- ✅ All validators generic and type-safe
- ✅ Works with nebula-derive seamlessly
- ✅ Full test coverage
- ✅ Ready for production use

---

## Contact/Notes

Current progress saved in:
- [PROGRESS.md](PROGRESS.md) - Session summary
- [NEXT_STEPS.md](NEXT_STEPS.md) - This file (next actions)
- [nebula-derive/DESIGN.md](crates/nebula-derive/DESIGN.md) - Design decisions

**Key Achievement**: Universal `expr` attribute in nebula-derive means no more updates needed when adding validators! 🎉
