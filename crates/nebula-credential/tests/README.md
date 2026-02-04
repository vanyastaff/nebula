# Test Organization

This directory contains tests for `nebula-credential` organized by test type and scope.

## Structure

```
tests/
├── units/              # Unit tests for individual components
│   ├── encryption_tests.rs    - Cryptographic operations (encrypt/decrypt, key derivation)
│   ├── error_tests.rs          - Error handling and error types
│   ├── storage_trait_tests.rs - StorageProvider trait contracts
│   └── validation_tests.rs     - Input validation logic
│
├── providers/          # Storage provider implementation tests
│   ├── mock_provider_tests.rs  - MockStorageProvider (10 tests)
│   └── local_provider_tests.rs - LocalStorageProvider (8 tests)
│
├── integration/        # End-to-end integration tests
│   └── local_storage_integration.rs - Real-world scenarios (10 tests)
│
└── mod.rs              # Test module organization
```

## Running Tests

### All tests
```bash
cargo test --package nebula-credential
```

### By category
```bash
# Library unit tests (in src/)
cargo test --package nebula-credential --lib

# Test suite (organized tests)
cargo test --package nebula-credential --test mod
```

### By module
```bash
# Run all unit tests
cargo test --package nebula-credential --test mod units::

# Run provider tests
cargo test --package nebula-credential --test mod providers::

# Run integration tests
cargo test --package nebula-credential --test mod integration::
```

### Specific test file
```bash
# Example: Run only encryption tests
cargo test --package nebula-credential --test mod units::encryption_tests::
```

## Test Coverage

**Unit Tests (units/)**: 42 tests
- Encryption: 7 tests (AES-256-GCM, key derivation, PKCE)
- Error handling: 8 tests (all error variants)
- Storage traits: 19 tests (trait contracts, filter logic)
- Validation: 8 tests (credential IDs, secret strings)

**Provider Tests (providers/)**: 18 tests
- MockStorageProvider: 10 tests (in-memory operations, error simulation)
- LocalStorageProvider: 8 tests (filesystem operations, filtering)

**Integration Tests (integration/)**: 10 tests
- Concurrent access (100+ parallel operations)
- File permissions (Unix 0600/0700)
- Error recovery and resilience
- Performance benchmarks

**Total: 70+ tests** across unit, provider, and integration levels.

## Legacy Tests

The following test files are currently disabled (commented out in `mod.rs`):
- `caching_tests.rs` - Requires CredentialManager
- `concurrency_tests.rs` - Requires CredentialManager
- `locking_tests.rs` - Requires CredentialManager
- `manager_tests.rs` - Requires CredentialManager
- `registry_tests.rs` - Requires CredentialRegistry

These will be re-enabled when the corresponding components are implemented in future phases.

## Adding New Tests

### Unit test
1. Create test file in `units/`
2. Add `mod your_test;` to `units/mod.rs`

### Provider test
1. Create test file in `providers/`
2. Add `mod your_provider_tests;` to `providers/mod.rs`

### Integration test
1. Create test file in `integration/`
2. Add `mod your_integration;` to `integration/mod.rs`

## Test Conventions

- Use `#[tokio::test]` for async tests
- Use `TempDir` from `tempfile` crate for filesystem tests
- Use descriptive test names: `test_feature_scenario`
- Group related tests in same file
- Add `#[cfg(unix)]` or `#[cfg(windows)]` for platform-specific tests
