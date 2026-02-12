# Phase 4 Migration Progress: nebula-expression

**Date**: 2026-02-11
**Status**: IN PROGRESS (Approximately 70% complete)

## Summary

Phase 4 (migrating nebula-expression from nebula_value to serde_json) is the most complex migration phase. Significant foundational work has been completed, with bulk pattern replacements applied across all 15 source files. Remaining work focuses on fixing compilation errors in specific modules.

## Completed Work ✅

### 1. Dependencies & Infrastructure (T021-T025)
- ✅ Updated Cargo.toml - removed nebula-value dependency, made chrono non-optional
- ✅ Updated lib.rs - changed all public API imports to serde_json::Value
- ✅ Updated error types - added Json and InvalidDate variants, removed Clone derive
- ✅ Updated Context module - migrated HashMap storage, fixed object/array construction
- ✅ Created value_utils.rs - comprehensive helper functions for Value operations

### 2. Bulk Pattern Replacements (T036-T037)
Applied across all 15 source files:
- ✅ `.is_integer()` → `.is_i64()`
- ✅ `.is_float()` → `.is_f64()`
- ✅ `.is_text()` → `.is_string()`
- ✅ `.as_integer()` → `.as_i64()`
- ✅ `.as_float()` → `.as_f64()`
- ✅ `.as_text()` → `.as_str()`
- ✅ `Value::text(` → `Value::String(`
- ✅ `Value::integer(` → `Value::Number(`
- ✅ `Value::boolean(` → `Value::Bool(`
- ✅ `Value::array_empty()` → `Value::Array(Vec::new())`
- ✅ `Value::object_empty()` → `Value::Object(serde_json::Map::new())`
- ✅ `Value::Integer` → `Value::Number`
- ✅ `Value::Float` → `Value::Number`
- ✅ `Value::Text` → `Value::String`
- ✅ `nebula_value::Array` → `serde_json::Value` or `Vec<Value>`
- ✅ `nebula_value::Object` → `serde_json::Map`

### 3. Helper Functions Created
In `value_utils.rs`:
- `value_type_name()` - Get type name for error messages
- `number_as_i64()` - Extract i64 from Number (handles both i64 and f64)
- `number_as_f64()` - Extract f64 from Number (handles both f64 and i64)
- `is_integer_number()` - Check if Number is integer type
- `can_add_as_int()` - Check if two numbers can be added as integers
- `is_truthy()` - Check if value is truthy (JavaScript-style)
- `to_boolean()` - Convert Value to boolean
- `to_integer()` - Convert Value to i64 with error
- `to_float()` - Convert Value to f64 with error

## Remaining Work 🚧

### Compilation Errors: 251 total

**Error Categories:**
1. **`.value()` on Number** (95 errors) - serde_json::Number doesn't have .value() method
   - Solution: Use `number_as_i64(num)` or `number_as_f64(num)` helper functions

2. **`.kind()` method** (54 errors) - serde_json::Value doesn't have .kind() method
   - Solution: Use `value_type_name(&value)` for error messages

3. **Type mismatches** (23 errors) - Various type annotation issues
   - Solution: Add explicit type annotations or fix conversions

4. **`.to_boolean()` method** (12 errors) - No equivalent in serde_json
   - Solution: Use `to_boolean(&value)` helper function

5. **`Value::from_vec` constructor** (11 errors) - Should be `Value::Array(vec)`
   - Solution: Replace with `Value::Array(...)`

6. **`.to_float()`, `.to_integer()` methods** (8 errors) - No equivalents
   - Solution: Use `to_float(&value)?` and `to_integer(&value)?` helpers

**Files Requiring Most Work:**
1. **eval/mod.rs** - 151 errors (evaluation engine)
2. **builtins/array.rs** - 25 errors
3. **builtins/datetime.rs** - 22 errors
4. **builtins/string.rs** - 11 errors
5. **builtins/mod.rs** - 10 errors
6. **builtins/conversion.rs** - 9 errors
7. **builtins/math.rs** - 8 errors
8. Other files - <5 errors each

### Recommended Completion Strategy

**Phase 1: Fix eval/mod.rs** (2-3 hours)
- This file has 151 errors, mostly following the same patterns
- Systematic replacements:
  - `num.value()` → `number_as_i64(num)` or `number_as_f64(num)`
  - `val.kind().name()` → `value_type_name(&val)`
  - `val.to_boolean()` → `to_boolean(&val)`
  - `Value::from_vec(...)` → `Value::Array(...)`

**Phase 2: Fix builtin modules** (2-3 hours)
- Work through each module systematically:
  1. datetime.rs - temporal handling changes
  2. array.rs - array operations
  3. string.rs - string operations
  4. conversion.rs - type conversions
  5. math.rs - numeric operations
  6. object.rs - object operations

**Phase 3: Fix remaining files** (1 hour)
- maybe.rs, parser/mod.rs, template modules
- Likely fewer, simpler errors

**Phase 4: Testing & Quality Gates** (2 hours)
- Run cargo test -p nebula-expression
- Fix test failures
- Run cargo fmt, cargo clippy
- Generate documentation

**Total Estimated Remaining Effort**: 7-9 hours

## Technical Notes

### serde_json::Number vs nebula_value Number
- **nebula_value**: Wrapped primitives with `.value()` method
- **serde_json**: Direct primitive access via `.as_i64()`, `.as_f64()`, `.as_u64()`
- **Solution**: Use helper functions that try both i64 and f64 representations

### Pattern Matching vs .kind()
- **nebula_value**: Had `.kind()` method returning enum for type checking
- **serde_json**: Use pattern matching or `.is_*()` methods
- **Solution**: `value_type_name()` for error messages, pattern matching for logic

### Boolean Conversion
- **nebula_value**: Had `.to_boolean()` with JavaScript-style truthiness
- **serde_json**: No built-in conversion
- **Solution**: `to_boolean()` helper implements same semantics

### Temporal Types
- **nebula_value**: Had Date, DateTime, Duration variants
- **serde_json**: Store as strings (ISO 8601/RFC 3339) or numbers (milliseconds)
- **Solution**: Parse with chrono when needed, store as strings/numbers

## Files Modified

1. ✅ Cargo.toml
2. ✅ src/lib.rs
3. ✅ src/error.rs
4. ✅ src/context/mod.rs
5. ✅ src/value_utils.rs (NEW)
6. 🚧 src/eval/mod.rs (151 errors)
7. 🚧 src/builtins/array.rs (25 errors)
8. 🚧 src/builtins/datetime.rs (22 errors)
9. 🚧 src/builtins/string.rs (11 errors)
10. 🚧 src/builtins/mod.rs (10 errors)
11. 🚧 src/builtins/conversion.rs (9 errors)
12. 🚧 src/builtins/math.rs (8 errors)
13. 🚧 src/builtins/object.rs (4 errors)
14. 🚧 src/builtins/util.rs (4 errors)
15. 🚧 src/maybe.rs (3 errors)
16. 🚧 src/parser/mod.rs (3 errors)
17. 🚧 src/template.rs (1 error)
18. 🚧 src/core/ast.rs (partial updates)
19. 🚧 src/engine.rs (partial updates)
20. 🚧 src/error_formatter.rs (partial updates)

## Next Steps

1. **Option A: Complete Phase 4 now** (recommended if time permits)
   - Follow the 4-phase completion strategy above
   - Estimated 7-9 hours of focused work

2. **Option B: Pause and resume later**
   - Current progress is saved and documented
   - Can resume with clear roadmap

3. **Option C: Delegate to automated tooling**
   - Use code mod tools for systematic pattern replacement
   - Manual review and testing still required

## Success Criteria

- [ ] cargo check -p nebula-expression - compiles with zero errors
- [ ] cargo test -p nebula-expression - 100% pass rate
- [ ] cargo clippy -p nebula-expression -- -D warnings - zero warnings
- [ ] cargo doc -p nebula-expression --no-deps - successful build
- [ ] No nebula_value imports remain in codebase
