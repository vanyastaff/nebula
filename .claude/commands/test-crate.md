---
description: Run comprehensive tests for a crate
---

Test crate {{arg:crate_name}} comprehensively:

## Test Strategy

### 1. **Compilation Check**
```bash
cargo check -p {{arg:crate_name}}
cargo check -p {{arg:crate_name}} --all-features
```

### 2. **Unit Tests**
```bash
# Library tests
cargo test -p {{arg:crate_name}} --lib

# All tests
cargo test -p {{arg:crate_name}}

# With output
cargo test -p {{arg:crate_name}} -- --nocapture
```

### 3. **Integration Tests**
```bash
# Specific test file
cargo test -p {{arg:crate_name}} --test <test_name>

# All integration tests
cargo test -p {{arg:crate_name}} --tests
```

### 4. **Doc Tests**
```bash
cargo test -p {{arg:crate_name}} --doc
```

### 5. **Feature Combinations**
```bash
# No default features
cargo test -p {{arg:crate_name}} --no-default-features

# All features
cargo test -p {{arg:crate_name}} --all-features

# Specific feature
cargo test -p {{arg:crate_name}} --features <feature>
```

## Test Analysis

After running tests, analyze:

### Pass/Fail Ratio
```
test result: ok. X passed; Y failed; Z ignored
```

- ✅ All pass: Ready to commit
- ⚠️ Some fail: Investigate failures
- ❌ Many fail: Architectural issue

### Performance
```bash
# With timing
cargo test -p {{arg:crate_name}} -- --report-time

# Benchmarks
cargo bench -p {{arg:crate_name}}
```

### Coverage (if available)
```bash
cargo tarpaulin -p {{arg:crate_name}}
```

## Common Test Failures

### Type Errors (Rust 2024)
```
error[E0599]: no method named `validate` found
```
**Fix**: Check trait bounds and type annotations

### Lifetime Issues
```
error[E0597]: borrowed value does not live long enough
```
**Fix**: Review lifetime annotations

### Test Data Issues
```
thread 'test' panicked at 'assertion failed'
```
**Fix**: Update test expectations or fix logic

## Test Quality Checks

- [ ] Tests compile without warnings
- [ ] All tests have clear names
- [ ] Edge cases covered
- [ ] Error paths tested
- [ ] No ignored tests without reason
- [ ] Test data is realistic

## Example Usage

```
/test-crate nebula-validator
```

This will run comprehensive tests for the nebula-validator crate and report results.
